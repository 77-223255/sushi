//! Universal patch files for Sushi.
//!
//! A patch is a plain JSON document that holds the value of every parameter of
//! the synthesizer, keyed by the stable id of the parameter. Any program can
//! read or write such a file, and a patch that was written by a newer version
//! of Sushi can still be loaded by an older one: unknown parameters are
//! ignored and missing ones keep their factory default.
//!
//! The patches live in a folder that the user can choose. By default that is
//! `~/Documents/Sushi Patches`; the choice is stored in the configuration file
//! `$XDG_CONFIG_HOME/sushi/config.json`.

use nice_plug::prelude::*;
use nice_plug::tracing::warn;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

use crate::params::SushiParams;

/// The name of the format, as it is written into the header of a patch.
const FORMAT: &str = "sushi-patch";

/// The version of the format, as it is written into the header of a patch.
const VERSION: u32 = 1;

/// The configuration of Sushi.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchConfig {
    /// The folder in which the patches are stored.
    pub patch_dir: PathBuf,
}

impl Default for PatchConfig {
    fn default() -> Self {
        Self {
            patch_dir: default_patch_dir(),
        }
    }
}

/// The home directory of the user.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// The folder in which the patches are stored by default.
pub fn default_patch_dir() -> PathBuf {
    match home_dir() {
        Some(home) => home.join("Documents").join("Sushi Patches"),
        None => PathBuf::from("sushi-patches"),
    }
}

/// The file in which the configuration of Sushi is stored.
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        });

    base.join("sushi").join("config.json")
}

/// Reads the configuration of Sushi.
///
/// A missing or broken configuration file gives the default configuration, so
/// that Sushi always starts.
pub fn load_config() -> PatchConfig {
    let path = config_path();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "the configuration {} could not be read: {error}",
                    path.display()
                );
            }
            return PatchConfig::default();
        }
    };

    let value: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            warn!("the configuration {} is broken: {error}", path.display());
            return PatchConfig::default();
        }
    };

    let Some(patch_dir) = value.get("patch_dir").and_then(Value::as_str) else {
        return PatchConfig::default();
    };
    if patch_dir.trim().is_empty() {
        return PatchConfig::default();
    }

    let patch_dir = PathBuf::from(patch_dir);
    if !patch_dir.is_absolute() {
        // A relative folder depends on the working directory of the process,
        // so it is of no use to the user. The default is used instead.
        warn!(
            "the folder {} of the configuration is not absolute, so it is ignored",
            patch_dir.display()
        );
        return PatchConfig::default();
    }

    PatchConfig { patch_dir }
}

/// Checks that a folder can hold the patches.
///
/// An empty folder would be resolved against the working directory of the
/// process, which is never the folder that the user chose.
pub fn check_patch_dir(dir: &Path) -> Result<(), String> {
    if dir.to_string_lossy().trim().is_empty() {
        return Err("the folder of the patches is empty".to_string());
    }

    Ok(())
}

/// Writes the configuration of Sushi.
pub fn save_config(config: &PatchConfig) -> std::io::Result<()> {
    if config.patch_dir.to_string_lossy().trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the folder of the patches is empty",
        ));
    }

    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let value = json!({ "patch_dir": config.patch_dir.to_string_lossy() });
    write_atomic(&path, &serde_json::to_string_pretty(&value)?)
}

/// Writes a text into a file without ever leaving a half written file
/// behind.
///
/// The text goes into a temporary file next to the target, and the temporary
/// file is then renamed over the target. Both files live in the same folder,
/// so the rename cannot fail halfway: either the old file is still there or
/// the new one is complete.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = tmp_path(path);
    fs::write(&tmp, text)?;
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }

    Ok(())
}

/// The name of the temporary file that is used while a file is written.
fn tmp_path(path: &Path) -> PathBuf {
    match path.file_name() {
        Some(name) => {
            let mut tmp = name.to_os_string();
            tmp.push(".tmp");
            path.with_file_name(tmp)
        }
        None => path.join("sushi.tmp"),
    }
}

/// Turns the name of a patch into a name that can be used as a file name.
///
/// The characters that are not allowed in a file name are removed, runs of
/// whitespace become a single space and the name is cut off at 64 characters.
/// A name that is empty after that is called "untitled".
pub fn sanitize_file_name(name: &str) -> String {
    let mut cleaned = String::new();
    let mut space = false;

    for character in name.trim().chars() {
        if matches!(
            character,
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
        ) {
            continue;
        }
        if character.is_whitespace() {
            space = !cleaned.is_empty();
            continue;
        }
        if space {
            cleaned.push(' ');
            space = false;
        }
        cleaned.push(character);
    }

    let cleaned: String = cleaned.chars().take(64).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "untitled".to_string()
    } else {
        cleaned.to_string()
    }
}

/// The lowest and the highest value of a floating point parameter.
fn float_bounds(range: FloatRange) -> (f32, f32) {
    match range {
        FloatRange::Linear { min, max } => (min, max),
        FloatRange::Skewed { min, max, .. } => (min, max),
        FloatRange::SymmetricalSkewed { min, max, .. } => (min, max),
        FloatRange::Reversed(inner) => float_bounds(*inner),
    }
}

/// The lowest and the highest value of an integer parameter.
fn int_bounds(range: IntRange) -> (i32, i32) {
    match range {
        IntRange::Linear { min, max } => (min, max),
        IntRange::Reversed(inner) => int_bounds(*inner),
    }
}

/// The name of a value, without the spaces, so that "As Played" and
/// "AsPlayed" name the same value.
fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_whitespace() && *character != '_' && *character != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// The value of a floating point parameter as a JSON number.
///
/// The shortest text that can be read back as the same number is used, so that
/// the patch files stay readable.
fn float_to_json(value: f32) -> Value {
    let text = format!("{value}");
    match text
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
    {
        Some(number) => Value::Number(number),
        None => Value::Null,
    }
}

/// A parameter of the synthesizer that can be stored in a patch file.
trait PatchParam {
    /// The value of the parameter, as the host stores it.
    type Plain;

    /// The factory default of the parameter.
    fn default_plain(&self) -> Self::Plain;

    /// Writes a value into the parameter.
    fn write(&self, setter: &ParamSetter, value: Self::Plain);

    /// Writes the factory default of the parameter into the host.
    fn write_default(&self, setter: &ParamSetter) {
        self.write(setter, self.default_plain());
    }

    /// The value of the parameter as a JSON value.
    fn to_json(&self) -> Value;

    /// Reads the value of the parameter from a JSON value.
    fn read_json(&self, value: &Value) -> Result<Self::Plain, String>;
}

impl PatchParam for FloatParam {
    type Plain = f32;

    fn default_plain(&self) -> f32 {
        self.default_plain_value()
    }

    fn write(&self, setter: &ParamSetter, value: f32) {
        setter.set_parameter(self, value);
    }

    fn to_json(&self) -> Value {
        float_to_json(self.value())
    }

    fn read_json(&self, value: &Value) -> Result<f32, String> {
        let number = value.as_f64().ok_or("expected a number")?;
        let number = number as f32;
        if !number.is_finite() {
            return Err("the number is not finite".to_string());
        }

        let (min, max) = float_bounds(self.range());
        Ok(number.clamp(min, max))
    }
}

impl PatchParam for IntParam {
    type Plain = i32;

    fn default_plain(&self) -> i32 {
        self.default_plain_value()
    }

    fn write(&self, setter: &ParamSetter, value: i32) {
        setter.set_parameter(self, value);
    }

    fn to_json(&self) -> Value {
        Value::from(self.value())
    }

    fn read_json(&self, value: &Value) -> Result<i32, String> {
        let number = value
            .as_i64()
            .or_else(|| {
                value.as_f64().and_then(|number| {
                    if number.fract() == 0.0 {
                        Some(number as i64)
                    } else {
                        None
                    }
                })
            })
            .ok_or("expected a whole number")?;
        let number = i32::try_from(number).map_err(|_| "the number is too large")?;

        let (min, max) = int_bounds(self.range());
        Ok(number.clamp(min, max))
    }
}

impl PatchParam for BoolParam {
    type Plain = bool;

    fn default_plain(&self) -> bool {
        self.default_plain_value()
    }

    fn write(&self, setter: &ParamSetter, value: bool) {
        setter.set_parameter(self, value);
    }

    fn to_json(&self) -> Value {
        Value::Bool(self.value())
    }

    fn read_json(&self, value: &Value) -> Result<bool, String> {
        match value {
            Value::Bool(value) => Ok(*value),
            Value::Number(number) => match number.as_i64() {
                Some(0) => Ok(false),
                Some(1) => Ok(true),
                _ => Err("expected true, false, 0 or 1".to_string()),
            },
            _ => Err("expected true, false, 0 or 1".to_string()),
        }
    }
}

impl<T: Enum + PartialEq + 'static> PatchParam for EnumParam<T> {
    type Plain = T;

    fn default_plain(&self) -> T {
        self.default_plain_value()
    }

    fn write(&self, setter: &ParamSetter, value: T) {
        setter.set_parameter(self, value);
    }

    fn to_json(&self) -> Value {
        let index = T::to_index(self.value());
        match T::variants().get(index) {
            Some(name) => Value::String((*name).to_string()),
            None => Value::Null,
        }
    }

    fn read_json(&self, value: &Value) -> Result<T, String> {
        if let Some(name) = value.as_str() {
            let wanted = normalize_name(name);
            for (index, variant) in T::variants().iter().enumerate() {
                if normalize_name(variant) == wanted {
                    return Ok(T::from_index(index));
                }
            }
            return Err(format!("\"{name}\" is not a valid value"));
        }

        if let Some(index) = value.as_u64() {
            let index = usize::try_from(index).map_err(|_| "the index is too large")?;
            if index < T::variants().len() {
                return Ok(T::from_index(index));
            }
            return Err(format!("the index {index} is out of range"));
        }

        Err("expected the name or the index of a value".to_string())
    }
}

/// The table of all the parameters of the synthesizer.
///
/// Every parameter is listed exactly once, so that none of them can be
/// forgotten when a patch is written or read.
macro_rules! sushi_params {
    ($( $id:literal => $field:ident ),* $(,)?) => {
        /// The ids of all the parameters of the synthesizer.
        pub const PARAM_IDS: &[&str] = &[ $( $id ),* ];

        /// Writes the factory default of every parameter into the host.
        pub fn apply_defaults(params: &SushiParams, setter: &ParamSetter) {
            $(
                params.$field.write_default(setter);
            )*
        }

        /// Collects the values of all the parameters into a JSON object.
        fn params_to_json(params: &SushiParams) -> Map<String, Value> {
            let mut map = Map::new();
            $(
                map.insert($id.to_string(), params.$field.to_json());
            )*
            map
        }

        /// Writes the values of a JSON object into the parameters of the host.
        fn params_from_json(
            params: &SushiParams,
            setter: &ParamSetter,
            map: &Map<String, Value>,
        ) -> Result<(), String> {
            for (id, value) in map {
                match id.as_str() {
                    $(
                        $id => {
                            let plain = params
                                .$field
                                .read_json(value)
                                .map_err(|error| format!("{id}: {error}"))?;
                            params.$field.write(setter, plain);
                        }
                    )*
                    _ => warn!("the patch holds the unknown parameter \"{id}\", which is ignored"),
                }
            }
            Ok(())
        }
    };
}

sushi_params! {
    "osc1_wave" => osc1_wave,
    "osc1_octave" => osc1_octave,
    "osc1_semi" => osc1_semi,
    "osc1_fine" => osc1_fine,
    "osc1_pw" => osc1_pw,
    "osc1_level" => osc1_level,
    "osc2_wave" => osc2_wave,
    "osc2_octave" => osc2_octave,
    "osc2_semi" => osc2_semi,
    "osc2_fine" => osc2_fine,
    "osc2_pw" => osc2_pw,
    "osc2_level" => osc2_level,
    "noise_level" => noise_level,
    "unison" => unison,
    "detune" => detune,
    "spread" => spread,
    "poly_mode" => poly_mode,
    "glide" => glide,
    "tune" => tune,
    "voices" => voices,
    "volume" => volume,
    "filter_mode" => filter_mode,
    "cutoff" => cutoff,
    "resonance" => resonance,
    "filter_env_amount" => filter_env_amount,
    "filter_keytrack" => filter_keytrack,
    "filter_drive" => filter_drive,
    "fe_attack" => fe_attack,
    "fe_decay" => fe_decay,
    "fe_sustain" => fe_sustain,
    "fe_release" => fe_release,
    "ae_attack" => ae_attack,
    "ae_decay" => ae_decay,
    "ae_sustain" => ae_sustain,
    "ae_release" => ae_release,
    "lfo1_wave" => lfo1_wave,
    "lfo1_rate" => lfo1_rate,
    "lfo1_amount" => lfo1_amount,
    "lfo1_target" => lfo1_target,
    "lfo2_wave" => lfo2_wave,
    "lfo2_rate" => lfo2_rate,
    "lfo2_amount" => lfo2_amount,
    "lfo2_target" => lfo2_target,
    "chorus_mix" => chorus_mix,
    "chorus_rate" => chorus_rate,
    "chorus_depth" => chorus_depth,
    "delay_time" => delay_time,
    "delay_feedback" => delay_feedback,
    "delay_mix" => delay_mix,
    "reverb_mix" => reverb_mix,
    "reverb_decay" => reverb_decay,
    "mod1_source" => mod1_source,
    "mod1_dest" => mod1_dest,
    "mod1_amount" => mod1_amount,
    "mod2_source" => mod2_source,
    "mod2_dest" => mod2_dest,
    "mod2_amount" => mod2_amount,
    "mod3_source" => mod3_source,
    "mod3_dest" => mod3_dest,
    "mod3_amount" => mod3_amount,
    "mod4_source" => mod4_source,
    "mod4_dest" => mod4_dest,
    "mod4_amount" => mod4_amount,
    "mod5_source" => mod5_source,
    "mod5_dest" => mod5_dest,
    "mod5_amount" => mod5_amount,
    "mod6_source" => mod6_source,
    "mod6_dest" => mod6_dest,
    "mod6_amount" => mod6_amount,
    "mod7_source" => mod7_source,
    "mod7_dest" => mod7_dest,
    "mod7_amount" => mod7_amount,
    "mod8_source" => mod8_source,
    "mod8_dest" => mod8_dest,
    "mod8_amount" => mod8_amount,
    "macro_bright" => macro_bright,
    "macro_motion" => macro_motion,
    "macro_space" => macro_space,
    "character" => character,
    "drift" => drift,
    "velocity" => velocity,
    "tone" => tone,
    "arp_on" => arp_on,
    "arp_pattern" => arp_pattern,
    "arp_rate" => arp_rate,
    "arp_octaves" => arp_octaves,
    "arp_gate" => arp_gate,
    "arp_latch" => arp_latch,
    "arp_swing" => arp_swing,
    "arp_length" => arp_length,
    "arp_ratchet" => arp_ratchet,
}

/// Collects the values of all the parameters into a JSON document.
pub fn to_json(params: &SushiParams, name: &str) -> Value {
    json!({
        "format": FORMAT,
        "version": VERSION,
        "name": name,
        "params": Value::Object(params_to_json(params)),
    })
}

/// The parameters of a patch document.
///
/// A document without a "params" object is the parameter map itself, which
/// makes the files easier to edit by hand.
fn param_map(value: &Value) -> Result<Map<String, Value>, String> {
    let object = value.as_object().ok_or("the patch is not a JSON object")?;

    match object.get("params") {
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => Err("\"params\" is not a JSON object".to_string()),
        None => {
            let mut map = object.clone();
            map.remove("format");
            map.remove("version");
            map.remove("name");
            Ok(map)
        }
    }
}

/// Writes the values of a JSON document into the parameters of the host.
///
/// The factory defaults are written first, so that a patch that only holds a
/// few parameters still sounds complete. Parameters that the document does
/// not hold are ignored, which keeps old patches loadable.
pub fn apply_json(params: &SushiParams, setter: &ParamSetter, value: &Value) -> Result<(), String> {
    let map = param_map(value)?;
    apply_defaults(params, setter);
    params_from_json(params, setter, &map)
}

/// Writes the parameters into a patch file in the given folder.
///
/// The folder is created if it does not exist yet.
pub fn save_patch(dir: &Path, name: &str, params: &SushiParams) -> Result<PathBuf, String> {
    check_patch_dir(dir)?;

    fs::create_dir_all(dir)
        .map_err(|error| format!("the folder {} could not be created: {error}", dir.display()))?;

    let path = dir.join(format!("{}.json", sanitize_file_name(name)));
    let text = serde_json::to_string_pretty(&to_json(params, name))
        .map_err(|error| format!("the patch could not be written: {error}"))?;
    write_atomic(&path, &text)
        .map_err(|error| format!("the patch {} could not be written: {error}", path.display()))?;

    Ok(path)
}

/// Reads a patch file and writes its values into the parameters of the host.
///
/// The name of the patch is the one in the file, or the name of the file if it
/// does not have one.
pub fn load_patch(
    path: &Path,
    params: &SushiParams,
    setter: &ParamSetter,
) -> Result<String, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("the patch {} could not be read: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("the patch {} is not valid JSON: {error}", path.display()))?;

    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "untitled".to_string());

    apply_json(params, setter, &value)?;

    Ok(name)
}

/// The patches in the given folder, sorted by their file name.
pub fn list_patches(dir: &Path) -> Vec<PathBuf> {
    if check_patch_dir(dir).is_err() {
        return Vec::new();
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut patches: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .collect();

    patches.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });

    patches
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A stub of the host that the tests of the patch system write into.

    use nice_plug::params::internals::ParamPtr;
    use nice_plug::prelude::*;
    use std::cell::Cell;

    thread_local! {
        /// The number of parameter values that were set.
        pub static SETS: Cell<usize> = const { Cell::new(0) };
    }

    /// The lock that keeps the tests that change the environment apart.
    pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Stub;

    impl GuiContextInner for Stub {
        fn plugin_api(&self) -> nice_plug::context::PluginApi {
            unimplemented!()
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
            SETS.with(|sets| sets.set(sets.get() + 1));
            unsafe {
                param._internal_set_normalized_value(normalized);
                // The real host updates the smoother of the parameter as well,
                // so that a value that was just set is used right away.
                param._internal_update_smoother(48_000.0, true);
            }
        }
        unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {}
        fn get_state(&self) -> PluginState {
            unimplemented!()
        }
        fn set_state(&self, _state: PluginState) {}
        fn request_restart(&self) {}
    }

    /// A setter that writes into the parameters of a synthesizer.
    pub fn setter() -> ParamSetter<'static> {
        // The stub is leaked on purpose: the setter lives as long as the test.
        let inner: &'static dyn GuiContextInner = Box::leak(Box::new(Stub));
        ParamSetter::new(inner)
    }

    /// The number of parameter values that were set since the counter was
    /// reset.
    pub fn set_count() -> usize {
        SETS.with(|sets| sets.get())
    }

    /// Forgets the parameter values that were set.
    pub fn reset_count() {
        SETS.with(|sets| sets.set(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{
        Character, FilterMode, LfoWaveform, ModDest, ModSource, NoteDiv, PolyMode, Waveform,
    };
    use crate::patch::test_support::{reset_count, set_count, setter};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A counter that makes the names of the temporary folders unique.
    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A folder in the temporary directory that is removed again when it is
    /// dropped.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        /// Creates a folder with the given name.
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "sushi-patch-{name}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("the temporary folder could not be created");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// The lock that keeps the tests that change the environment apart.
    use crate::patch::test_support::ENV_LOCK;

    /// Compares two patch documents.
    ///
    /// The numbers may differ a little, because the values of the parameters
    /// travel through the normalized values of the host.
    fn assert_same_patch(left: &Value, right: &Value, path: &str) {
        match (left, right) {
            (Value::Number(left), Value::Number(right)) => {
                let (left, right) = (left.as_f64().unwrap(), right.as_f64().unwrap());
                let tolerance = 1e-3 * right.abs().max(1.0);
                assert!(
                    (left - right).abs() <= tolerance,
                    "{path}: {left} is not {right}"
                );
            }
            (Value::Object(left), Value::Object(right)) => {
                assert_eq!(
                    left.len(),
                    right.len(),
                    "{path}: the maps are not the same size"
                );
                for (id, value) in left {
                    let other = right
                        .get(id)
                        .unwrap_or_else(|| panic!("{path}: the parameter {id} is missing"));
                    assert_same_patch(value, other, &format!("{path}.{id}"));
                }
            }
            (left, right) => assert_eq!(left, right, "{path}"),
        }
    }

    #[test]
    fn every_parameter_is_in_the_table() {
        assert_eq!(PARAM_IDS.len(), 91);

        let mut ids: Vec<&str> = PARAM_IDS.to_vec();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 91, "the ids of the parameters are not unique");
    }

    #[test]
    fn the_defaults_are_written_into_the_host() {
        let params = SushiParams::default();
        let setter = setter();

        reset_count();
        apply_defaults(&params, &setter);

        // Every one of the 91 parameters is written.
        assert_eq!(set_count(), 91);
    }

    #[test]
    fn the_defaults_reset_every_parameter() {
        let params = SushiParams::default();
        let setter = setter();

        // Move a few parameters away from their default.
        setter.set_parameter(&params.cutoff, 100.0);
        setter.set_parameter(&params.resonance, 0.9);
        setter.set_parameter(&params.osc1_wave, Waveform::Square);
        setter.set_parameter(&params.arp_on, true);
        setter.set_parameter(&params.unison, 4);

        apply_defaults(&params, &setter);

        assert_eq!(params.cutoff.value(), 3_000.0);
        assert_eq!(params.resonance.value(), 0.2);
        assert_eq!(params.osc1_wave.value(), Waveform::Saw);
        assert!(!params.arp_on.value());
        assert_eq!(params.unison.value(), 1);
    }

    #[test]
    fn all_parameters_survive_a_round_trip() {
        let params = SushiParams::default();
        let setter = setter();

        // Move some of the parameters, so that the round trip has something
        // to carry.
        setter.set_parameter(&params.cutoff, 2_400.0);
        setter.set_parameter(&params.osc1_wave, Waveform::Pulse);
        setter.set_parameter(&params.arp_on, true);
        setter.set_parameter(&params.unison, 3);
        setter.set_parameter(&params.tone, -0.4);
        setter.set_parameter(&params.mod1_source, ModSource::Macro1);
        setter.set_parameter(&params.mod1_dest, ModDest::Pitch);
        setter.set_parameter(&params.mod1_amount, 0.5);

        let document = to_json(&params, "ROUND TRIP");
        let other = SushiParams::default();
        apply_json(&other, &setter, &document).expect("the patch could not be applied");

        // The slots of the modulation matrix are part of the patch.
        assert_eq!(other.mod1_source.value(), ModSource::Macro1);
        assert_eq!(other.mod1_dest.value(), ModDest::Pitch);
        assert!((other.mod1_amount.value() - 0.5).abs() < 0.01);

        let restored = to_json(&other, "ROUND TRIP");
        assert_same_patch(&document, &restored, "patch");
    }

    #[test]
    fn the_patch_holds_every_parameter() {
        let params = SushiParams::default();
        let document = to_json(&params, "TEST");

        assert_eq!(document["format"], "sushi-patch");
        assert_eq!(document["version"], 1);
        assert_eq!(document["name"], "TEST");

        let map = document["params"].as_object().unwrap();
        assert_eq!(map.len(), 91);
        for id in PARAM_IDS {
            assert!(map.contains_key(*id), "the parameter {id} is missing");
        }
    }

    #[test]
    fn enums_can_be_read_by_name_and_by_index() {
        let params = SushiParams::default();
        let setter = setter();

        let by_name = json!({ "params": { "osc1_wave": "pulse", "arp_rate": "1/16" } });
        apply_json(&params, &setter, &by_name).expect("the patch could not be applied");
        assert_eq!(params.osc1_wave.value(), Waveform::Pulse);
        assert_eq!(params.arp_rate.value(), NoteDiv::Sixteenth);

        let by_index = json!({ "params": { "osc1_wave": 4, "arp_rate": 2 } });
        apply_json(&params, &setter, &by_index).expect("the patch could not be applied");
        assert_eq!(params.osc1_wave.value(), Waveform::Pulse);
        assert_eq!(params.arp_rate.value(), NoteDiv::Sixteenth);

        // The names are also matched when they are written differently.
        let by_other_name = json!({ "params": { "arp_pattern": "as played" } });
        apply_json(&params, &setter, &by_other_name).expect("the patch could not be applied");
        assert_eq!(
            params.arp_pattern.value(),
            crate::params::ArpPattern::AsPlayed
        );
    }

    #[test]
    fn booleans_can_be_read_as_numbers() {
        let params = SushiParams::default();
        let setter = setter();

        let document = json!({ "params": { "arp_on": true, "arp_latch": 1 } });
        apply_json(&params, &setter, &document).expect("the patch could not be applied");
        assert!(params.arp_on.value());
        assert!(params.arp_latch.value());

        let document = json!({ "params": { "arp_on": false, "arp_latch": 0 } });
        apply_json(&params, &setter, &document).expect("the patch could not be applied");
        assert!(!params.arp_on.value());
        assert!(!params.arp_latch.value());
    }

    #[test]
    fn unknown_parameters_are_ignored() {
        let params = SushiParams::default();
        let setter = setter();

        let document = json!({ "params": { "osc1_wave": "Square", "osc9_wave": "Saw" } });
        apply_json(&params, &setter, &document).expect("the patch could not be applied");
        assert_eq!(params.osc1_wave.value(), Waveform::Square);
    }

    #[test]
    fn the_parameters_may_be_the_document_itself() {
        let params = SushiParams::default();
        let setter = setter();

        let document = json!({
            "format": "sushi-patch",
            "version": 1,
            "name": "HAND MADE",
            "osc1_wave": "Triangle",
            "poly_mode": "Mono",
            "cutoff": 1234.0,
        });
        apply_json(&params, &setter, &document).expect("the patch could not be applied");
        assert_eq!(params.osc1_wave.value(), Waveform::Triangle);
        assert_eq!(params.poly_mode.value(), PolyMode::Mono);
        assert!((params.cutoff.value() - 1234.0).abs() < 0.1);

        // The parameters that the document does not hold keep their default.
        assert_eq!(params.osc2_wave.value(), Waveform::Saw);
        assert_eq!(params.filter_mode.value(), FilterMode::Lowpass);
        assert_eq!(params.volume.value(), util::db_to_gain(-6.0));
    }

    #[test]
    fn broken_documents_are_reported() {
        let params = SushiParams::default();
        let setter = setter();

        assert!(apply_json(&params, &setter, &json!(42)).is_err());
        assert!(apply_json(&params, &setter, &json!({ "params": 42 })).is_err());
        assert!(
            apply_json(
                &params,
                &setter,
                &json!({ "params": { "osc1_wave": "Nonsense" } })
            )
            .is_err()
        );
        assert!(apply_json(&params, &setter, &json!({ "params": { "unison": 1.5 } })).is_err());
        assert!(apply_json(&params, &setter, &json!({ "params": { "arp_on": 7 } })).is_err());
    }

    #[test]
    fn the_values_are_clamped_to_their_range() {
        let params = SushiParams::default();
        let setter = setter();

        let document = json!({ "params": { "cutoff": 1e9, "unison": 99, "resonance": -3.0 } });
        apply_json(&params, &setter, &document).expect("the patch could not be applied");

        assert_eq!(params.cutoff.value(), 20_000.0);
        assert_eq!(params.unison.value(), 4);
        assert_eq!(params.resonance.value(), 0.0);
    }

    #[test]
    fn the_file_names_are_cleaned_up() {
        assert_eq!(sanitize_file_name("JUNO PAD"), "JUNO PAD");
        assert_eq!(sanitize_file_name("  JUNO PAD  "), "JUNO PAD");
        assert_eq!(sanitize_file_name("a/b\\c:d*e?f\"g<h>i|j"), "abcdefghij");
        assert_eq!(sanitize_file_name("two   spaces"), "two spaces");
        assert_eq!(sanitize_file_name(""), "untitled");
        assert_eq!(sanitize_file_name("///"), "untitled");
        assert_eq!(sanitize_file_name(&"x".repeat(100)).len(), 64);
    }

    #[test]
    fn the_patch_folder_is_in_the_documents() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());

        let old = std::env::var_os("HOME");
        let home = TempDir::new("home");
        unsafe {
            std::env::set_var("HOME", &home.path);
        }

        assert_eq!(
            default_patch_dir(),
            home.path.join("Documents").join("Sushi Patches")
        );
        assert_eq!(PatchConfig::default().patch_dir, default_patch_dir());

        match old {
            Some(value) => unsafe {
                std::env::set_var("HOME", value);
            },
            None => unsafe {
                std::env::remove_var("HOME");
            },
        }
    }

    #[test]
    fn the_config_path_follows_the_environment() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());

        let old = std::env::var_os("XDG_CONFIG_HOME");
        let home = TempDir::new("config");
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &home.path);
        }

        let path = config_path();
        assert_eq!(path, home.path.join("sushi").join("config.json"));

        // A relative path is ignored, just like the XDG specification says.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", "relative");
        }
        assert!(config_path().is_absolute());
        assert!(config_path().ends_with("sushi/config.json"));

        match old {
            Some(value) => unsafe {
                std::env::set_var("XDG_CONFIG_HOME", value);
            },
            None => unsafe {
                std::env::remove_var("XDG_CONFIG_HOME");
            },
        }
    }

    #[test]
    fn the_config_can_be_saved_and_loaded() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());

        let old = std::env::var_os("XDG_CONFIG_HOME");
        let home = TempDir::new("config-save");
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &home.path);
        }

        let config = PatchConfig {
            patch_dir: home.path.join("Patches"),
        };
        save_config(&config).expect("the configuration could not be written");

        assert_eq!(config_path(), home.path.join("sushi").join("config.json"));
        assert_eq!(load_config(), config);

        // A broken configuration gives the default one.
        fs::write(config_path(), "not json").expect("the configuration could not be written");
        assert_eq!(load_config(), PatchConfig::default());

        match old {
            Some(value) => unsafe {
                std::env::set_var("XDG_CONFIG_HOME", value);
            },
            None => unsafe {
                std::env::remove_var("XDG_CONFIG_HOME");
            },
        }
    }

    #[test]
    fn a_patch_can_be_saved_and_loaded() {
        let params = SushiParams::default();
        let setter = setter();
        let dir = TempDir::new("save");

        setter.set_parameter(&params.cutoff, 2_400.0);
        setter.set_parameter(&params.osc2_wave, Waveform::Square);
        setter.set_parameter(&params.arp_on, true);

        let path = save_patch(&dir.path, "MY PATCH", &params).expect("the patch was not written");
        assert_eq!(path, dir.path.join("MY PATCH.json"));

        // The other parameters keep their default.
        let other = SushiParams::default();
        let name = load_patch(&path, &other, &setter).expect("the patch was not read");
        assert_eq!(name, "MY PATCH");
        assert!((other.cutoff.value() - 2_400.0).abs() < 0.1);
        assert_eq!(other.osc2_wave.value(), Waveform::Square);
        assert!(other.arp_on.value());
    }

    #[test]
    fn an_empty_folder_is_refused() {
        let params = SushiParams::default();

        // An empty folder would be the working directory of the process,
        // which is never the folder that the user chose.
        assert!(check_patch_dir(Path::new("")).is_err());
        assert!(check_patch_dir(Path::new("   ")).is_err());
        assert!(check_patch_dir(Path::new("/tmp")).is_ok());

        assert!(save_patch(Path::new(""), "TEST", &params).is_err());
        assert!(save_patch(Path::new("   "), "TEST", &params).is_err());

        assert!(list_patches(Path::new("")).is_empty());
        assert!(list_patches(Path::new("   ")).is_empty());
    }

    #[test]
    fn the_configuration_needs_a_folder() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());

        let old = std::env::var_os("XDG_CONFIG_HOME");
        let home = TempDir::new("config-empty");
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &home.path);
        }

        let config = PatchConfig {
            patch_dir: PathBuf::from("   "),
        };
        assert!(save_config(&config).is_err());
        // Nothing was written.
        assert!(!config_path().exists());

        match old {
            Some(value) => unsafe {
                std::env::set_var("XDG_CONFIG_HOME", value);
            },
            None => unsafe {
                std::env::remove_var("XDG_CONFIG_HOME");
            },
        }
    }

    #[test]
    fn a_relative_folder_is_ignored_by_the_configuration() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());

        let old = std::env::var_os("XDG_CONFIG_HOME");
        let home = TempDir::new("config-relative");
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &home.path);
        }

        if let Some(parent) = config_path().parent() {
            fs::create_dir_all(parent).expect("the folder was not created");
        }
        fs::write(config_path(), json!({ "patch_dir": "patches" }).to_string())
            .expect("the configuration was not written");

        // A relative folder depends on the working directory of the process,
        // so the default is used instead.
        assert_eq!(load_config(), PatchConfig::default());

        match old {
            Some(value) => unsafe {
                std::env::set_var("XDG_CONFIG_HOME", value);
            },
            None => unsafe {
                std::env::remove_var("XDG_CONFIG_HOME");
            },
        }
    }

    #[test]
    fn a_patch_is_written_atomically() {
        let params = SushiParams::default();
        let setter = setter();
        let dir = TempDir::new("atomic");

        setter.set_parameter(&params.cutoff, 1_500.0);
        let path = save_patch(&dir.path, "ATOMIC", &params).expect("the patch was not written");

        // The temporary file of the write is gone, so only the patch itself
        // is in the folder.
        let mut files: Vec<String> = fs::read_dir(&dir.path)
            .expect("the folder was not read")
            .map(|entry| {
                entry
                    .expect("the entry was not read")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        files.sort();
        assert_eq!(files, ["ATOMIC.json"]);

        // The patch is a complete file that can be read again.
        let other = SushiParams::default();
        let name = load_patch(&path, &other, &setter).expect("the patch was not read");
        assert_eq!(name, "ATOMIC");
        assert!((other.cutoff.value() - 1_500.0).abs() < 1.0);
    }

    #[test]
    fn the_configuration_is_written_atomically() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());

        let old = std::env::var_os("XDG_CONFIG_HOME");
        let home = TempDir::new("config-atomic");
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &home.path);
        }

        let config = PatchConfig {
            patch_dir: home.path.join("Patches"),
        };
        save_config(&config).expect("the configuration was not written");

        // The temporary file of the write is gone.
        let path = config_path();
        let folder = path.parent().expect("the folder was not found");
        let mut files: Vec<String> = fs::read_dir(folder)
            .expect("the folder was not read")
            .map(|entry| {
                entry
                    .expect("the entry was not read")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        files.sort();
        assert_eq!(files, ["config.json"]);
        assert_eq!(load_config(), config);

        match old {
            Some(value) => unsafe {
                std::env::set_var("XDG_CONFIG_HOME", value);
            },
            None => unsafe {
                std::env::remove_var("XDG_CONFIG_HOME");
            },
        }
    }

    #[test]
    fn a_patch_without_a_name_takes_the_name_of_its_file() {
        let params = SushiParams::default();
        let setter = setter();
        let dir = TempDir::new("no-name");

        let path = dir.path.join("MY FILE.json");
        fs::write(&path, json!({ "params": { "cutoff": 1000.0 } }).to_string())
            .expect("the patch was not written");

        let name = load_patch(&path, &params, &setter).expect("the patch was not read");
        assert_eq!(name, "MY FILE");
    }

    #[test]
    fn the_patches_are_listed_in_order() {
        let dir = TempDir::new("list");
        for name in ["beta.json", "Alpha.json", "gamma.json", "notes.txt"] {
            fs::write(dir.path.join(name), "{}").expect("the file was not written");
        }

        let patches = list_patches(&dir.path);
        let names: Vec<String> = patches
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();

        assert_eq!(names, ["Alpha.json", "beta.json", "gamma.json"]);
        assert!(list_patches(&dir.path.join("nowhere")).is_empty());
    }

    #[test]
    fn the_enums_have_readable_names() {
        // The names in the patches are the ones of the enums.
        assert_eq!(Waveform::variants()[0], "Sine");
        assert_eq!(FilterMode::variants()[0], "Lowpass");
        assert_eq!(LfoWaveform::variants()[4], "S&H");
        assert_eq!(NoteDiv::variants()[2], "1/16");
        assert_eq!(Character::variants()[1], "Grit");
    }
}
