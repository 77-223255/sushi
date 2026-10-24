//! The undo and redo history of the editor.
//!
//! Every entry is a full snapshot of the sound (all 109 patch parameters) and
//! of the patch identity that the panel shows. The snapshots only live on the
//! GUI thread: the audio thread never allocates for them, because it never
//! reads or writes this history.
//!
//! A continuous gesture (a fader drag, a fine-lane drag or an amount bar) is
//! coalesced into one step. The first edit of the gesture snapshots the sound
//! before it, and the snapshot is only pushed when no gesture is open anymore.
//! Discrete actions (a popup choice, a typed number, a reset, a preset load, a
//! mutation or a patch load) commit at the end of the frame in which they
//! happen.

use std::collections::VecDeque;
use std::path::PathBuf;

use nice_plug::prelude::*;
use serde_json::Value;

use super::{EditorState, refresh_patch_preview};
use crate::params::SushiParams;
use crate::patch;

/// How many undo steps are kept. The oldest step is dropped when the history
/// is full, so the memory of the editor stays bounded.
pub(super) const UNDO_LIMIT: usize = 64;

/// The complete state that one undo step restores.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Snapshot {
    /// The sound, in the JSON format of the patch files.
    document: Value,
    /// The factory preset that the sound belongs to.
    preset_index: usize,
    /// The name of the loaded patch of the user, if any.
    patch_active: Option<String>,
    /// The file of the loaded patch of the user, if any.
    patch_active_path: Option<PathBuf>,
    /// The name in the save field of the patch browser.
    patch_name: String,
    /// Whether the sound was changed after it was loaded.
    dirty: bool,
}

impl Snapshot {
    /// Takes a snapshot of the current sound and patch identity.
    pub(super) fn capture(params: &SushiParams, state: &EditorState) -> Self {
        Self {
            // The name of the document is not part of the sound, so a fixed
            // one keeps two snapshots of the same sound equal even when the
            // name of the patch in the display changed in between.
            document: patch::to_json(params, "UNDO"),
            preset_index: state.preset_index,
            patch_active: state.patch_active.clone(),
            patch_active_path: state.patch_active_path.clone(),
            patch_name: state.patch_name.clone(),
            dirty: state.dirty,
        }
    }

    /// Writes the snapshot back into the parameters and the panel state.
    pub(super) fn apply(
        &self,
        params: &SushiParams,
        setter: &ParamSetter,
        state: &mut EditorState,
    ) -> Result<(), String> {
        patch::apply_json(params, setter, &self.document)?;

        state.preset_index = self.preset_index;
        state.patch_active = self.patch_active.clone();
        state.patch_active_path = self.patch_active_path.clone();
        state.patch_name = self.patch_name.clone();
        state.dirty = self.dirty;
        refresh_patch_preview(params, state);

        Ok(())
    }
}

/// The bounded undo and redo stacks.
#[derive(Default)]
pub(super) struct UndoHistory {
    /// The states before the edits, oldest first.
    undo: VecDeque<Snapshot>,
    /// The states that were undone, newest last.
    redo: VecDeque<Snapshot>,
    /// The state before the edit that is currently in progress.
    pending: Option<Snapshot>,
    /// The before and after of the last edit that left a step behind. A
    /// double-click reset uses it to swallow the first click of the double
    /// click, so undoing a reset returns to the value before the whole
    /// double click instead of the value under the pointer.
    last_edit: Option<(Snapshot, Snapshot)>,
}

impl UndoHistory {
    /// Whether there is a step to undo.
    pub(super) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is a step to redo.
    pub(super) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Forgets every step. It is used when the host reloads the project or
    /// performs an undo of its own, so that the editor never restores a
    /// snapshot from a different sound.
    pub(super) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
        self.last_edit = None;
    }

    /// The number of undo steps. Only the tests need to see the depth.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.undo.len()
    }

    /// Remembers the state before the first change of an edit. Further calls
    /// while the edit is in progress keep the first snapshot, which coalesces
    /// a continuous gesture into one step.
    pub(super) fn mark(&mut self, params: &SushiParams, state: &EditorState) {
        if self.pending.is_none() {
            self.pending = Some(Snapshot::capture(params, state));
        }
    }

    /// Finishes the edit that is in progress. A gesture that did not change
    /// the sound does not leave an empty undo step behind.
    pub(super) fn commit(&mut self, params: &SushiParams, state: &EditorState) {
        let Some(before) = self.pending.take() else {
            return;
        };
        let after = Snapshot::capture(params, state);
        if before == after {
            // A click that did not move anything must not leave a stale
            // merge candidate behind for a later reset.
            self.last_edit = None;
            return;
        }
        self.push(before.clone());
        self.last_edit = Some((before, after));
    }

    /// Merges the first click of a double-click reset into the reset itself.
    /// This only happens when the last step is exactly the state that the
    /// first click produced, so an unrelated older edit is never swallowed.
    pub(super) fn merge_double_click(&mut self, params: &SushiParams, state: &EditorState) {
        let Some((before, after)) = self.last_edit.take() else {
            return;
        };
        if Snapshot::capture(params, state) != after || self.undo.back() != Some(&before) {
            return;
        }
        self.undo.pop_back();
        self.pending = Some(before);
    }

    /// Goes one step back. Returns whether the sound was restored.
    pub(super) fn undo(
        &mut self,
        params: &SushiParams,
        setter: &ParamSetter,
        state: &mut EditorState,
    ) -> bool {
        // An undo ends any edit that is still pending instead of recording
        // the restore as a new edit.
        self.pending = None;
        self.last_edit = None;
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        let current = Snapshot::capture(params, state);
        if previous.apply(params, setter, state).is_err() {
            self.undo.push_back(previous);
            return false;
        }
        self.push_redo(current);
        true
    }

    /// Goes one step forward again. Returns whether the sound was restored.
    pub(super) fn redo(
        &mut self,
        params: &SushiParams,
        setter: &ParamSetter,
        state: &mut EditorState,
    ) -> bool {
        self.pending = None;
        self.last_edit = None;
        let Some(next) = self.redo.pop_back() else {
            return false;
        };
        let current = Snapshot::capture(params, state);
        if next.apply(params, setter, state).is_err() {
            self.redo.push_back(next);
            return false;
        }
        self.push_undo(current);
        true
    }

    /// Pushes an undo step and forgets the redo branch, because it belongs to
    /// a history that does not exist anymore.
    fn push(&mut self, snapshot: Snapshot) {
        self.push_undo(snapshot);
        self.redo.clear();
    }

    fn push_undo(&mut self, snapshot: Snapshot) {
        if self.undo.len() == UNDO_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(snapshot);
    }

    fn push_redo(&mut self, snapshot: Snapshot) {
        if self.redo.len() == UNDO_LIMIT {
            self.redo.pop_front();
        }
        self.redo.push_back(snapshot);
    }
}
