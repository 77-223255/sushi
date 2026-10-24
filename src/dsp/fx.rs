//! The effect chain: a drive, a phaser, a chorus, a ping-pong delay, a
//! compact reverb, a three band EQ, a compressor and a tremolo/auto-pan.
//!
//! All the delay lines are allocated up front (in [`Fx::new`]) so that the
//! processing itself never allocates. The new effects default to completely
//! neutral settings, so old patches and presets keep sounding the same.
//!
//! The eight stages have a fixed original order, `stage_order(0)`. The `order`
//! parameter selects one of the `8! = 40320` permutations of that sequence, and
//! `bypass_mask` skips individual stages. `global_mix` is a dry/wet crossfade
//! around the whole chain; its default of `1.0` returns the wet signal
//! untouched, bit for bit.
//!
//! The old `6! = 720` order values keep their exact six-stage chain and get
//! the compressor and the tremolo appended at the end. Old patches therefore
//! keep their sound: both new stages are neutral by default. The remaining
//! full eight-stage permutations start at order 720.

use std::f32::consts::PI;

use super::adaa::AdaaTanh;
use super::reverb_legacy::LegacyReverb;

/// The base delay of the chorus in seconds.
const CHORUS_BASE_DELAY: f32 = 0.012;
/// The maximum modulation depth of the chorus in seconds.
const CHORUS_MAX_DEPTH: f32 = 0.004;
/// The frequency of the lowpass of the wet signal of the chorus, in Hz.
const CHORUS_LOWPASS: f32 = 8_000.0;
/// The maximum stereo widening of the chorus. At full width the side signal
/// of the wet chorus is doubled.
const CHORUS_MAX_WIDTH: f32 = 1.0;
/// The maximum delay time of the delay line in seconds.
const DELAY_MAX_TIME: f32 = 2.0;
/// The frequency of the lowpass in the feedback path of the delay, in Hz.
const DELAY_LOWPASS: f32 = 5_000.0;
/// The lowest feedback damping cutoff of the delay, in Hz. Higher damping
/// moves the fixed 5 kHz lowpass down towards this frequency.
const DELAY_MIN_LOWPASS: f32 = 150.0;
/// The time constant of the one-pole that follows a change of the delay time,
/// in seconds. Without it a moved control jumps the read position, which is
/// heard as a click or a zipper.
const DELAY_TIME_SMOOTHING: f32 = 0.02;
/// The mix below which an effect is switched off completely.
const BYPASS_MIX: f32 = 1e-4;
/// The maximum pre-gain of the drive, in decibels.
const DRIVE_MAX_GAIN_DB: f32 = 24.0;
/// The output compensation of the drive at full amount. The tanh already
/// limits the wet signal to ±1; this keeps a hot input from sounding louder
/// than the dry signal.
const DRIVE_COMPENSATION: f32 = 1.5;
/// The crossover of the post-drive tone tilt, in Hz.
const DRIVE_TONE_FREQUENCY: f32 = 800.0;
/// The boost or cut of the drive tone tilt at full amount, in decibels.
const DRIVE_TONE_MAX_DB: f32 = 6.0;
/// A tone setting closer to zero than this is the exact old algorithm.
const TONE_BYPASS: f32 = 1e-6;
/// The number of first-order allpass stages in the phaser.
const PHASER_STAGES: usize = 4;
/// The centre frequency of the phaser, in Hz.
const PHASER_CENTER: f32 = 700.0;
/// The sweep range of the phaser at full depth, in octaves.
const PHASER_OCTAVES: f32 = 2.0;
/// The lowest frequency of an allpass stage. The coefficient stays well
/// inside the stable range even at very low sample rates.
const PHASER_MIN_FREQUENCY: f32 = 10.0;
/// The frequency of the low/mid crossover of the EQ, in Hz.
const EQ_LOW_CROSSOVER: f32 = 250.0;
/// The frequency of the mid/high crossover of the EQ, in Hz.
const EQ_HIGH_CROSSOVER: f32 = 4_000.0;
/// The largest boost or cut of an EQ band, in decibels.
const EQ_MAX_GAIN_DB: f32 = 12.0;
/// A band gain closer to 0 dB than this is treated as neutral.
const EQ_BYPASS_DB: f32 = 1e-4;
/// The crossover of the reverb tone tilt, in Hz.
const REVERB_TONE_FREQUENCY: f32 = 1_200.0;
/// The boost or cut of the reverb tone tilt at full amount, in decibels.
const REVERB_TONE_MAX_DB: f32 = 9.0;

/// The number of stages in the effect chain.
pub const FX_STAGE_COUNT: usize = 8;

/// The number of stages in the original six-stage chain.
pub const FX_LEGACY_STAGE_COUNT: usize = 6;

/// The number of possible stage orders: every permutation of the eight stages.
pub const FX_ORDER_COUNT: usize = 40_320;

/// The number of the old six-stage permutations, kept for migration.
pub const FX_LEGACY_ORDER_COUNT: usize = 720;

/// The largest valid value of the old six-stage `fx_order` parameter.
pub const FX_LEGACY_ORDER_MAX: i32 = FX_LEGACY_ORDER_COUNT as i32 - 1;

/// The largest valid value of the `fx_order` parameter.
pub const FX_ORDER_MAX: i32 = FX_ORDER_COUNT as i32 - 1;

/// The mask with every stage bit set.
pub const FX_BYPASS_ALL: u32 = (1 << FX_STAGE_COUNT) - 1;

/// A stage of the effect chain.
///
/// The discriminant is also the bit that bypasses the stage in
/// [`FxParams::bypass_mask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FxStage {
    Drive = 0,
    Phaser = 1,
    Chorus = 2,
    Delay = 3,
    Reverb = 4,
    Eq = 5,
    Compressor = 6,
    Tremolo = 7,
}

impl FxStage {
    /// Every stage in the original order, with the new stages at the end.
    pub const ALL: [Self; FX_STAGE_COUNT] = [
        Self::Drive,
        Self::Phaser,
        Self::Chorus,
        Self::Delay,
        Self::Reverb,
        Self::Eq,
        Self::Compressor,
        Self::Tremolo,
    ];

    /// The stages of the original six-stage chain.
    pub const LEGACY: [Self; FX_LEGACY_STAGE_COUNT] = [
        Self::Drive,
        Self::Phaser,
        Self::Chorus,
        Self::Delay,
        Self::Reverb,
        Self::Eq,
    ];

    /// The bit that bypasses this stage in the mask.
    #[inline]
    pub const fn bit(self) -> u32 {
        1 << self as u32
    }

    /// The index of this stage in the original order.
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// The original six stages in the order selected by an old permutation index.
///
/// The first six orders keep the original rotation semantics, where every
/// step takes the first stage and appends it at the end:
///
/// - `0`: Drive > Phaser > Chorus > Delay > Reverb > EQ
/// - `1`: Phaser > Chorus > Delay > Reverb > EQ > Drive
/// - `2`: Chorus > Delay > Reverb > EQ > Drive > Phaser
/// - `3`: Delay > Reverb > EQ > Drive > Phaser > Chorus
/// - `4`: Reverb > EQ > Drive > Phaser > Chorus > Delay
/// - `5`: EQ > Drive > Phaser > Chorus > Delay > Reverb
///
/// Orders `6..=719` are the remaining permutations in lexicographic order.
/// Together the 720 orders cover every possible old chain order exactly once.
///
/// Out-of-range indices are clamped, so a bad host value can never index past
/// the end of the table.
#[inline]
pub fn legacy_stage_order(order: i32) -> [FxStage; FX_LEGACY_STAGE_COUNT] {
    LEGACY_STAGE_ORDERS[order.clamp(0, FX_LEGACY_ORDER_MAX) as usize]
}

/// The permutation index of an old six-stage chain order, `0..=719`.
///
/// This is the inverse of [`legacy_stage_order()`].
#[inline]
pub fn legacy_stage_order_index(order: [FxStage; FX_LEGACY_STAGE_COUNT]) -> i32 {
    LEGACY_STAGE_ORDERS
        .iter()
        .position(|candidate| *candidate == order)
        .map_or(0, |index| index as i32)
}

/// The explicit migration of an old order value: the old six-stage chain
/// followed by the compressor and the tremolo.
///
/// Both new stages are neutral by default, so an old patch with `fx_order`
/// `0..=719` keeps its exact sound. This is the mapping that the first 720
/// entries of [`stage_order()`] use.
#[inline]
pub fn migrated_stage_order(order: i32) -> [FxStage; FX_STAGE_COUNT] {
    let legacy = legacy_stage_order(order);
    [
        legacy[0],
        legacy[1],
        legacy[2],
        legacy[3],
        legacy[4],
        legacy[5],
        FxStage::Compressor,
        FxStage::Tremolo,
    ]
}

/// The eight stages in the order selected by a permutation index, `0..=40319`.
///
/// Orders `0..=719` are the explicit [`migrated_stage_order()`] of the old
/// six-stage chain: the old order plus Compressor and Tremolo at the end.
/// Orders `720..=40319` are the remaining eight-stage permutations in
/// lexicographic order, which makes every order a unique, reversible
/// description of the chain.
///
/// Out-of-range indices are clamped, so a bad host value can never leave the
/// valid permutation range.
#[inline]
pub fn stage_order(order: i32) -> [FxStage; FX_STAGE_COUNT] {
    let order = order.clamp(0, FX_ORDER_MAX);
    if order < FX_LEGACY_ORDER_COUNT as i32 {
        migrated_stage_order(order)
    } else {
        permutation_from_rank(nth_allowed_rank(
            (order - FX_LEGACY_ORDER_COUNT as i32) as usize,
        ))
    }
}

/// The permutation index of an eight-stage chain order, `0..=40319`.
///
/// This is the inverse of [`stage_order()`]. Every possible permutation of the
/// eight stages appears exactly once, so this always finds the original
/// order.
#[inline]
pub fn stage_order_index(order: [FxStage; FX_STAGE_COUNT]) -> i32 {
    // The migrated old orders all end in Compressor, Tremolo. Their index is
    // simply the old six-stage index.
    if order[FX_LEGACY_STAGE_COUNT].index() == FxStage::Compressor.index()
        && order[FX_LEGACY_STAGE_COUNT + 1].index() == FxStage::Tremolo.index()
    {
        return legacy_stage_order_index([
            order[0], order[1], order[2], order[3], order[4], order[5],
        ]);
    }

    let rank = permutation_rank(order);
    let allowed_before = rank - reserved_before(rank);
    FX_LEGACY_ORDER_COUNT as i32 + allowed_before as i32
}

/// `n!` for the tiny values needed by the permutation tables.
const fn factorial(n: usize) -> usize {
    let mut result = 1;
    let mut factor = 2;
    while factor <= n {
        result *= factor;
        factor += 1;
    }
    result
}

/// Decodes a zero-based lexicographic permutation rank into an eight-stage
/// order.
const fn permutation_from_rank(mut rank: usize) -> [FxStage; FX_STAGE_COUNT] {
    let mut used = [false; FX_STAGE_COUNT];
    let mut result = [FxStage::Drive; FX_STAGE_COUNT];
    let mut position = 0;

    while position < FX_STAGE_COUNT {
        let block = factorial(FX_STAGE_COUNT - 1 - position);
        let mut index = rank / block;
        rank %= block;

        let mut candidate = 0;
        while candidate < FX_STAGE_COUNT {
            if !used[candidate] {
                if index == 0 {
                    result[position] = FxStage::ALL[candidate];
                    used[candidate] = true;
                    break;
                }
                index -= 1;
            }
            candidate += 1;
        }

        position += 1;
    }

    result
}

/// The zero-based lexicographic rank of an eight-stage order.
const fn permutation_rank(permutation: [FxStage; FX_STAGE_COUNT]) -> usize {
    let mut used = [false; FX_STAGE_COUNT];
    let mut rank = 0;
    let mut position = 0;

    while position < FX_STAGE_COUNT {
        let target = permutation[position].index();
        let mut index = 0;
        let mut candidate = 0;
        while candidate < target {
            if !used[candidate] {
                index += 1;
            }
            candidate += 1;
        }

        rank += index * factorial(FX_STAGE_COUNT - 1 - position);
        used[target] = true;
        position += 1;
    }

    rank
}

/// Decodes a zero-based lexicographic permutation rank into a six-stage order.
const fn legacy_permutation_from_rank(mut rank: usize) -> [FxStage; FX_LEGACY_STAGE_COUNT] {
    let mut used = [false; FX_LEGACY_STAGE_COUNT];
    let mut result = [FxStage::Drive; FX_LEGACY_STAGE_COUNT];
    let mut position = 0;

    while position < FX_LEGACY_STAGE_COUNT {
        let block = factorial(FX_LEGACY_STAGE_COUNT - 1 - position);
        let mut index = rank / block;
        rank %= block;

        let mut candidate = 0;
        while candidate < FX_LEGACY_STAGE_COUNT {
            if !used[candidate] {
                if index == 0 {
                    result[position] = FxStage::LEGACY[candidate];
                    used[candidate] = true;
                    break;
                }
                index -= 1;
            }
            candidate += 1;
        }

        position += 1;
    }

    result
}

/// The zero-based lexicographic rank of a six-stage order.
const fn legacy_permutation_rank(permutation: [FxStage; FX_LEGACY_STAGE_COUNT]) -> usize {
    let mut used = [false; FX_LEGACY_STAGE_COUNT];
    let mut rank = 0;
    let mut position = 0;

    while position < FX_LEGACY_STAGE_COUNT {
        let target = permutation[position].index();
        let mut index = 0;
        let mut candidate = 0;
        while candidate < target {
            if !used[candidate] {
                index += 1;
            }
            candidate += 1;
        }

        rank += index * factorial(FX_LEGACY_STAGE_COUNT - 1 - position);
        used[target] = true;
        position += 1;
    }

    rank
}

/// Builds the old order table: the six original rotations first, then every
/// remaining six-stage permutation in lexicographic order.
const fn build_legacy_stage_order_table()
-> [[FxStage; FX_LEGACY_STAGE_COUNT]; FX_LEGACY_ORDER_COUNT] {
    let mut table = [[FxStage::Drive; FX_LEGACY_STAGE_COUNT]; FX_LEGACY_ORDER_COUNT];

    let mut rotation = 0;
    while rotation < FX_LEGACY_STAGE_COUNT {
        let mut position = 0;
        while position < FX_LEGACY_STAGE_COUNT {
            table[rotation][position] =
                FxStage::LEGACY[(position + rotation) % FX_LEGACY_STAGE_COUNT];
            position += 1;
        }
        rotation += 1;
    }

    let mut rotation_ranks = [usize::MAX; FX_LEGACY_STAGE_COUNT];
    let mut index = 0;
    while index < FX_LEGACY_STAGE_COUNT {
        rotation_ranks[index] = legacy_permutation_rank(table[index]);
        index += 1;
    }

    let mut order = FX_LEGACY_STAGE_COUNT;
    let mut rank = 0;
    while rank < FX_LEGACY_ORDER_COUNT {
        let mut is_rotation = false;
        let mut rotation = 0;
        while rotation < FX_LEGACY_STAGE_COUNT {
            if rotation_ranks[rotation] == rank {
                is_rotation = true;
                break;
            }
            rotation += 1;
        }

        if !is_rotation {
            table[order] = legacy_permutation_from_rank(rank);
            order += 1;
        }
        rank += 1;
    }

    table
}

/// The old six-stage orders, built at compile time.
const LEGACY_STAGE_ORDERS: [[FxStage; FX_LEGACY_STAGE_COUNT]; FX_LEGACY_ORDER_COUNT] =
    build_legacy_stage_order_table();

/// Builds the lexicographic ranks of the migrated old orders, sorted so that
/// the number of reserved ranks below any rank can be counted quickly.
const fn build_reserved_ranks() -> [u16; FX_LEGACY_ORDER_COUNT] {
    let mut ranks = [0u16; FX_LEGACY_ORDER_COUNT];
    let mut count = 0;

    while count < FX_LEGACY_ORDER_COUNT {
        let legacy = LEGACY_STAGE_ORDERS[count];
        let mut permutation = [FxStage::Drive; FX_STAGE_COUNT];
        let mut position = 0;
        while position < FX_LEGACY_STAGE_COUNT {
            permutation[position] = legacy[position];
            position += 1;
        }
        permutation[FX_LEGACY_STAGE_COUNT] = FxStage::Compressor;
        permutation[FX_LEGACY_STAGE_COUNT + 1] = FxStage::Tremolo;
        let rank = permutation_rank(permutation) as u16;

        // Insertion sort: the table has only 720 entries and is built once.
        let mut slot = count;
        while slot > 0 && ranks[slot - 1] > rank {
            ranks[slot] = ranks[slot - 1];
            slot -= 1;
        }
        ranks[slot] = rank;
        count += 1;
    }

    ranks
}

/// The sorted lexicographic ranks of every permutation that ends in
/// Compressor, Tremolo; these are the 720 migrated old orders.
const RESERVED_RANKS: [u16; FX_LEGACY_ORDER_COUNT] = build_reserved_ranks();

/// The number of reserved (migrated old) permutations whose lexicographic
/// rank is below `rank`.
#[inline]
fn reserved_before(rank: usize) -> usize {
    let mut low = 0;
    let mut high = RESERVED_RANKS.len();
    while low < high {
        let middle = (low + high) / 2;
        if (RESERVED_RANKS[middle] as usize) < rank {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

/// Whether a lexicographic rank belongs to a migrated old order.
#[inline]
fn is_reserved_rank(rank: usize) -> bool {
    RESERVED_RANKS.binary_search(&(rank as u16)).is_ok()
}

/// The lexicographic rank of the `index`th eight-stage permutation that does
/// not end in Compressor, Tremolo.
fn nth_allowed_rank(index: usize) -> usize {
    // The number of allowed permutations below `rank` is
    // `rank - reserved_before(rank)`. Find the first rank where that count
    // reaches `index`, then skip a reserved rank if the boundary sits on one.
    let mut low = 0;
    let mut high = FX_ORDER_COUNT;
    while low < high {
        let middle = (low + high) / 2;
        if middle - reserved_before(middle) >= index {
            high = middle;
        } else {
            low = middle + 1;
        }
    }

    let mut rank = low;
    while rank < FX_ORDER_COUNT && is_reserved_rank(rank) {
        rank += 1;
    }
    rank
}

/// The parameters of the effect chain.
#[derive(Debug, Clone, Copy)]
pub struct FxParams {
    /// The permutation index of the eight-stage chain, `0..=40319`. See
    /// [`stage_order()`] for the exact mapping.
    pub order: i32,
    /// The bypass mask. Bit 0 is Drive, 1 Phaser, 2 Chorus, 3 Delay,
    /// 4 Reverb, 5 EQ, 6 Compressor and 7 Tremolo.
    pub bypass_mask: i32,
    /// The dry/wet crossfade around the entire chain, `0..=1`. At `1` the
    /// chain output is returned bit-for-bit; at `0` the dry input is returned
    /// and the effects are cleared.
    pub global_mix: f32,
    pub drive_amount: f32,
    pub drive_mix: f32,
    /// The post-drive tone tilt, `-1..=1`. Zero is bit-exact with the old
    /// drive.
    pub drive_tone: f32,
    pub phaser_rate: f32,
    pub phaser_depth: f32,
    pub phaser_mix: f32,
    /// The feedback around the phaser allpass chain, `0..=0.9`. Zero is
    /// bit-exact with the old phaser.
    pub phaser_feedback: f32,
    pub eq_low: f32,
    pub eq_mid: f32,
    pub eq_high: f32,
    pub chorus_mix: f32,
    pub chorus_rate: f32,
    pub chorus_depth: f32,
    /// The stereo width of the wet chorus, `0..=1`. Zero is the old stereo
    /// behavior.
    pub chorus_width: f32,
    pub delay_time_s: f32,
    pub delay_feedback: f32,
    pub delay_mix: f32,
    /// Extra damping of the delay feedback, `0..=1`. Zero keeps the old fixed
    /// 5 kHz lowpass.
    pub delay_damping: f32,
    pub reverb_mix: f32,
    pub reverb_decay: f32,
    /// The tone of the reverb wet signal, `-1..=1`. Zero is bit-exact with
    /// the old reverb.
    pub reverb_tone: f32,
    /// The threshold of the stereo-linked compressor, in dB.
    pub comp_threshold: f32,
    /// The compression ratio, `1..=20`.
    pub comp_ratio: f32,
    /// The attack time of the compressor, in milliseconds.
    pub comp_attack: f32,
    /// The release time of the compressor, in milliseconds.
    pub comp_release: f32,
    /// The makeup gain of the compressor, in dB.
    pub comp_makeup: f32,
    /// The dry/wet mix of the compressor.
    pub comp_mix: f32,
    /// The rate of the tremolo/auto-pan, in Hz.
    pub trem_rate: f32,
    /// The depth of the tremolo/auto-pan, `0..=1`. Zero is exact neutral.
    pub trem_depth: f32,
    /// Whether the tremolo pans between the channels instead of modulating
    /// both channels together.
    pub trem_stereo: bool,
}

impl Default for FxParams {
    fn default() -> Self {
        Self {
            order: 0,
            bypass_mask: 0,
            global_mix: 1.0,
            drive_amount: 0.0,
            drive_mix: 0.0,
            drive_tone: 0.0,
            phaser_rate: 0.3,
            phaser_depth: 0.5,
            phaser_mix: 0.0,
            phaser_feedback: 0.0,
            eq_low: 0.0,
            eq_mid: 0.0,
            eq_high: 0.0,
            chorus_mix: 0.0,
            chorus_rate: 0.8,
            chorus_depth: 0.5,
            chorus_width: 0.0,
            delay_time_s: 0.35,
            delay_feedback: 0.3,
            delay_mix: 0.0,
            delay_damping: 0.0,
            reverb_mix: 0.0,
            reverb_decay: 0.6,
            reverb_tone: 0.0,
            comp_threshold: 0.0,
            comp_ratio: 1.0,
            comp_attack: 20.0,
            comp_release: 200.0,
            comp_makeup: 0.0,
            comp_mix: 0.0,
            trem_rate: 5.0,
            trem_depth: 0.0,
            trem_stereo: false,
        }
    }
}

impl FxParams {
    /// The global mix, clamped to the valid range. A non-finite host value
    /// falls back to fully wet instead of poisoning the audio path.
    #[inline]
    pub fn clamped_global_mix(&self) -> f32 {
        if self.global_mix.is_finite() {
            self.global_mix.clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// The bypass mask with every bit outside the eight stages cleared.
    #[inline]
    pub fn clamped_bypass_mask(&self) -> u32 {
        self.bypass_mask.clamp(0, FX_BYPASS_ALL as i32) as u32
    }

    /// Whether the compressor would change the signal. Ratio 1, a threshold
    /// at 0 dB and no makeup gain are all exact neutral.
    #[inline]
    fn compressor_active(&self) -> bool {
        if self.comp_mix.clamp(0.0, 1.0) <= BYPASS_MIX {
            return false;
        }
        let ratio = self.comp_ratio.clamp(1.0, 20.0);
        let threshold = self.comp_threshold.clamp(-60.0, 0.0);
        let makeup = self.comp_makeup.clamp(0.0, 24.0);
        !((ratio <= 1.0 || threshold >= 0.0) && makeup <= 1e-6)
    }

    /// Whether a single stage would change the signal. The bypass mask and
    /// the global mix are not considered here; see [`Self::is_active`].
    #[inline]
    pub fn stage_active(&self, stage: FxStage) -> bool {
        match stage {
            FxStage::Drive => self.drive_mix.clamp(0.0, 1.0) > BYPASS_MIX,
            FxStage::Phaser => self.phaser_mix.clamp(0.0, 1.0) > BYPASS_MIX,
            FxStage::Chorus => self.chorus_mix.clamp(0.0, 1.0) > BYPASS_MIX,
            FxStage::Delay => self.delay_mix.clamp(0.0, 1.0) > BYPASS_MIX,
            FxStage::Reverb => self.reverb_mix.clamp(0.0, 1.0) > BYPASS_MIX,
            FxStage::Eq => {
                let low = self.eq_low.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
                let mid = self.eq_mid.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
                let high = self.eq_high.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
                !(low.abs() <= EQ_BYPASS_DB
                    && mid.abs() <= EQ_BYPASS_DB
                    && high.abs() <= EQ_BYPASS_DB)
            }
            FxStage::Compressor => self.compressor_active(),
            FxStage::Tremolo => self.trem_depth.clamp(0.0, 1.0) > BYPASS_MIX,
        }
    }

    /// Whether the chain can change the signal, using an already clamped
    /// bypass mask and global mix. `Fx::process` clamps both values once and
    /// reuses them for the whole sample.
    #[inline]
    fn is_active_with(&self, mask: u32, global_mix: f32) -> bool {
        global_mix > 0.0
            && FxStage::ALL
                .iter()
                .any(|&stage| mask & stage.bit() == 0 && self.stage_active(stage))
    }

    /// Whether the chain can change the signal: the global mix is above zero
    /// and at least one stage is active and not bypassed.
    pub fn is_active(&self) -> bool {
        self.is_active_with(self.clamped_bypass_mask(), self.clamped_global_mix())
    }
}

/// The coefficient of a one-pole lowpass at the given frequency.
#[inline]
fn lowpass_coefficient(frequency: f32, sample_rate: f32) -> f32 {
    1.0 - (-2.0 * PI * frequency / sample_rate.max(1.0)).exp()
}

/// The coefficient of a one-pole that follows a control with the given time
/// constant. The time constant is in seconds, so the smoothing takes the same
/// time at every sample rate.
#[inline]
fn smoothing_coefficient(time_seconds: f32, sample_rate: f32) -> f32 {
    let samples = (time_seconds.max(1e-6) * sample_rate.max(1.0)).max(1.0);
    1.0 - (-1.0 / samples).exp()
}

/// Flushes a value that is too small to be heard to exactly zero.
#[inline]
fn flush_denormal(value: f32) -> f32 {
    if value.abs() < 1e-20 { 0.0 } else { value }
}

/// The coefficient of a first-order allpass at the given frequency. The
/// frequency is clamped so the coefficient stays inside the stable range even
/// at the lowest sample rates.
#[inline]
fn allpass_coefficient(frequency: f32, sample_rate: f32) -> f32 {
    let sample_rate = sample_rate.max(1.0);
    let max_frequency = (sample_rate * 0.45).max(PHASER_MIN_FREQUENCY);
    let frequency = frequency.clamp(PHASER_MIN_FREQUENCY, max_frequency);
    let tangent = (PI * frequency / sample_rate).tan();
    (1.0 - tangent) / (1.0 + tangent)
}

/// One first-order allpass stage in direct form II. The state is the single
/// delay of the stage.
#[inline]
fn allpass_stage(input: f32, coefficient: f32, state: &mut f32) -> f32 {
    let output = coefficient * input + *state;
    *state = flush_denormal(input - coefficient * output);
    output
}

/// Crossfades between the dry and the wet signal of a stage. The operation
/// order matches the original inline expressions, so the result is bit-exact.
#[inline]
fn crossfade(dry_left: f32, dry_right: f32, wet_left: f32, wet_right: f32, mix: f32) -> (f32, f32) {
    let dry_gain = 1.0 - mix;
    (
        dry_left * dry_gain + wet_left * mix,
        dry_right * dry_gain + wet_right * mix,
    )
}

/// Splits the input into a low and a high band and applies the two gains.
/// The caller skips this function at neutral settings, so the old algorithm
/// stays bit-exact.
#[inline]
fn tilt_sample(
    input: f32,
    state: &mut f32,
    coefficient: f32,
    low_gain: f32,
    high_gain: f32,
) -> f32 {
    *state += (input - *state) * coefficient;
    *state = flush_denormal(*state);
    let low = *state;
    let high = input - low;
    low * low_gain + high * high_gain
}

/// The effect chain of the synthesizer.
pub struct Fx {
    sample_rate: f32,

    // The drive.
    drive_adaa_left: AdaaTanh,
    drive_adaa_right: AdaaTanh,
    /// The low band of the post-drive tone tilt.
    drive_tone_left: f32,
    drive_tone_right: f32,
    drive_tone_coefficient: f32,

    // The phaser.
    phaser_phase: f32,
    phaser_left: [f32; PHASER_STAGES],
    phaser_right: [f32; PHASER_STAGES],
    /// The last wet sample, fed back into the allpass chain.
    phaser_feedback_left: f32,
    phaser_feedback_right: f32,

    // The three band EQ. The low band is the output of a lowpass at the low
    // crossover, the high band is the input minus a lowpass at the high
    // crossover, and the mid band is the rest.
    eq_low_left: f32,
    eq_low_right: f32,
    eq_high_left: f32,
    eq_high_right: f32,
    eq_low_coefficient: f32,
    eq_high_coefficient: f32,

    // The chorus.
    chorus_left: Vec<f32>,
    chorus_right: Vec<f32>,
    chorus_index: usize,
    chorus_phase: f32,
    /// The state of the lowpass of the wet signal.
    chorus_lp_left: f32,
    chorus_lp_right: f32,
    /// The coefficient of the lowpass of the wet signal.
    chorus_coefficient: f32,

    // The ping-pong delay.
    delay_left: Vec<f32>,
    delay_right: Vec<f32>,
    delay_index: usize,
    /// The smoothed delay time of the ping-pong delay, in samples. `None`
    /// until the first sample is processed, so the first read uses the
    /// requested time directly instead of ramping up from zero.
    delay_time_samples: Option<f32>,
    /// The coefficient of the one-pole that smoothes the delay time.
    delay_smoothing: f32,
    /// The state of the lowpass in the feedback path.
    delay_lp_left: f32,
    delay_lp_right: f32,
    /// The coefficient of the lowpass in the feedback path.
    delay_coefficient: f32,
    // The reverb.
    legacy_reverb: LegacyReverb,
    /// The low band of the reverb tone tilt.
    reverb_tone_left: f32,
    reverb_tone_right: f32,
    reverb_tone_coefficient: f32,

    // The compressor. A single gain envelope links the two channels.
    compressor_gain: f32,

    // The tremolo/auto-pan.
    tremolo_phase: f32,

    /// The order value and its decoded stage order. Decoding only happens
    /// when the order changes, not on every sample.
    last_order: i32,
    order_cache: [FxStage; FX_STAGE_COUNT],

    /// Whether the whole chain was bypassed on the previous sample. The
    /// first bypassed sample clears all the state, so re-enabling an effect
    /// cannot replay an old tail.
    bypassed: bool,
    /// The bypass mask that was applied on the previous sample. A stage that
    /// is newly bypassed is cleared once, at the transition, so it cannot
    /// replay stale audio when it is enabled again.
    last_bypass_mask: u32,
}

impl Fx {
    /// Allocates all the delay lines. This is called from the plugin's
    /// `activate` function, never from the audio thread.
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);

        let chorus_len =
            (sample_rate * (CHORUS_BASE_DELAY + CHORUS_MAX_DEPTH + 0.002)).ceil() as usize + 2;
        let delay_len = (sample_rate * DELAY_MAX_TIME).ceil() as usize + 2;

        Self {
            sample_rate,
            drive_adaa_left: AdaaTanh::default(),
            drive_adaa_right: AdaaTanh::default(),
            drive_tone_left: 0.0,
            drive_tone_right: 0.0,
            drive_tone_coefficient: lowpass_coefficient(DRIVE_TONE_FREQUENCY, sample_rate),
            phaser_phase: 0.0,
            phaser_left: [0.0; PHASER_STAGES],
            phaser_right: [0.0; PHASER_STAGES],
            phaser_feedback_left: 0.0,
            phaser_feedback_right: 0.0,
            eq_low_left: 0.0,
            eq_low_right: 0.0,
            eq_high_left: 0.0,
            eq_high_right: 0.0,
            eq_low_coefficient: lowpass_coefficient(EQ_LOW_CROSSOVER, sample_rate),
            eq_high_coefficient: lowpass_coefficient(EQ_HIGH_CROSSOVER, sample_rate),
            chorus_left: vec![0.0; chorus_len],
            chorus_right: vec![0.0; chorus_len],
            chorus_index: 0,
            chorus_phase: 0.0,
            chorus_lp_left: 0.0,
            chorus_lp_right: 0.0,
            chorus_coefficient: lowpass_coefficient(CHORUS_LOWPASS, sample_rate),
            delay_left: vec![0.0; delay_len],
            delay_right: vec![0.0; delay_len],
            delay_index: 0,
            delay_time_samples: None,
            delay_smoothing: smoothing_coefficient(DELAY_TIME_SMOOTHING, sample_rate),
            delay_lp_left: 0.0,
            delay_lp_right: 0.0,
            delay_coefficient: lowpass_coefficient(DELAY_LOWPASS, sample_rate),
            legacy_reverb: LegacyReverb::new(sample_rate),
            reverb_tone_left: 0.0,
            reverb_tone_right: 0.0,
            reverb_tone_coefficient: lowpass_coefficient(REVERB_TONE_FREQUENCY, sample_rate),
            compressor_gain: 1.0,
            tremolo_phase: 0.0,
            last_order: 0,
            order_cache: FxStage::ALL,
            bypassed: true,
            last_bypass_mask: 0,
        }
    }

    /// Clears all the delay lines and the state of the other effects.
    pub fn reset(&mut self) {
        for stage in FxStage::ALL {
            self.reset_stage(stage);
        }
        self.bypassed = true;
        self.last_bypass_mask = 0;
    }

    /// Clears the state of one stage without touching the other stages.
    fn reset_stage(&mut self, stage: FxStage) {
        match stage {
            FxStage::Drive => {
                self.drive_adaa_left.reset();
                self.drive_adaa_right.reset();
                self.drive_tone_left = 0.0;
                self.drive_tone_right = 0.0;
            }
            FxStage::Phaser => {
                self.phaser_phase = 0.0;
                self.phaser_left = [0.0; PHASER_STAGES];
                self.phaser_right = [0.0; PHASER_STAGES];
                self.phaser_feedback_left = 0.0;
                self.phaser_feedback_right = 0.0;
            }
            FxStage::Chorus => {
                self.chorus_left.fill(0.0);
                self.chorus_right.fill(0.0);
                self.chorus_index = 0;
                self.chorus_phase = 0.0;
                self.chorus_lp_left = 0.0;
                self.chorus_lp_right = 0.0;
            }
            FxStage::Delay => {
                self.delay_left.fill(0.0);
                self.delay_right.fill(0.0);
                self.delay_index = 0;
                self.delay_time_samples = None;
                self.delay_lp_left = 0.0;
                self.delay_lp_right = 0.0;
            }
            FxStage::Reverb => {
                self.legacy_reverb.reset();
                self.reverb_tone_left = 0.0;
                self.reverb_tone_right = 0.0;
            }
            FxStage::Eq => {
                self.eq_low_left = 0.0;
                self.eq_low_right = 0.0;
                self.eq_high_left = 0.0;
                self.eq_high_right = 0.0;
            }
            FxStage::Compressor => {
                self.compressor_gain = 1.0;
            }
            FxStage::Tremolo => {
                self.tremolo_phase = 0.0;
            }
        }
    }

    /// Processes a single stereo sample.
    pub fn process(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        // The effect chain is not initialized yet.
        if self.delay_left.is_empty() {
            return (left, right);
        }

        // Clamp the mask and the global mix once, then reuse them for the
        // activity test, the stage loop and the final crossfade.
        let bypass_mask = params.clamped_bypass_mask();
        let global_mix = params.clamped_global_mix();

        // `is_active_with` includes the per-stage mix and EQ neutral checks,
        // the bypass mask and the global mix. With the default fully wet chain
        // it is exactly the old full-FX bypass test.
        if !params.is_active_with(bypass_mask, global_mix) {
            if !self.bypassed {
                // Entering bypass clears every delay line, the reverb and
                // the new effects, so a later enable starts from silence
                // instead of replaying an old tail.
                self.reset();
            }
            // The whole chain is clean. Remember the mask so that a later
            // unmask still starts from a cleared stage.
            self.last_bypass_mask = bypass_mask;
            return (left, right);
        }

        // The reset on entering bypass left the state clean. The first active
        // sample can therefore start here.
        self.bypassed = false;

        // A stage that becomes bypassed is cleared once at the transition,
        // never on every sample.
        if bypass_mask != self.last_bypass_mask {
            for stage in FxStage::ALL {
                if bypass_mask & stage.bit() != 0 {
                    self.reset_stage(stage);
                }
            }
            self.last_bypass_mask = bypass_mask;
        }

        let (dry_left, dry_right) = (left, right);
        let mut wet_left = left;
        let mut wet_right = right;

        // The stage order is a fixed stack array, so reordering allocates
        // nothing and an out-of-range order simply clamps. The decoded order
        // is cached: the factoradic lookup runs only when the control moves.
        if params.order != self.last_order {
            self.last_order = params.order;
            self.order_cache = stage_order(params.order);
        }
        for stage in self.order_cache {
            if bypass_mask & stage.bit() != 0 {
                continue;
            }
            let processed = match stage {
                FxStage::Drive => self.process_drive(wet_left, wet_right, params),
                FxStage::Phaser => self.process_phaser(wet_left, wet_right, params),
                FxStage::Chorus => self.process_chorus(wet_left, wet_right, params),
                FxStage::Delay => self.process_delay(wet_left, wet_right, params),
                FxStage::Reverb => self.process_reverb(wet_left, wet_right, params),
                FxStage::Eq => self.process_eq(wet_left, wet_right, params),
                FxStage::Compressor => self.process_compressor(wet_left, wet_right, params),
                FxStage::Tremolo => self.process_tremolo(wet_left, wet_right, params),
            };
            wet_left = processed.0;
            wet_right = processed.1;
        }

        // At unity the wet signal is returned directly, bit for bit. Any
        // other value is a crossfade between the chain input and output.
        if global_mix >= 1.0 {
            (wet_left, wet_right)
        } else {
            crossfade(dry_left, dry_right, wet_left, wet_right, global_mix)
        }
    }

    /// An anti-aliased tanh drive with a dry/wet crossfade.
    fn process_drive(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let mix = params.drive_mix.clamp(0.0, 1.0);

        if mix <= BYPASS_MIX {
            // A bypassed drive must not colour the signal, and its ADAA
            // history must not leak into a later enable.
            self.drive_adaa_left.reset();
            self.drive_adaa_right.reset();
            self.drive_tone_left = 0.0;
            self.drive_tone_right = 0.0;
            return (left, right);
        }

        let amount = params.drive_amount.clamp(0.0, 1.0);
        // The amount maps to 0..24 dB of pre-gain. The tanh limits the wet
        // signal, while the compensation keeps a hot input close to the level
        // of the dry signal.
        let gain = 10f32.powf(amount * DRIVE_MAX_GAIN_DB / 20.0);
        let compensation = 1.0 / (1.0 + amount * DRIVE_COMPENSATION);
        let wet_left = self.drive_adaa_left.process(left * gain) * compensation;
        let wet_right = self.drive_adaa_right.process(right * gain) * compensation;

        // The tone control tilts the driven signal after the saturator. Zero
        // skips the split completely, so the old drive stays bit-exact.
        let tone = params.drive_tone.clamp(-1.0, 1.0);
        let (wet_left, wet_right) = if tone.abs() > TONE_BYPASS {
            let high_gain = 10f32.powf(tone * DRIVE_TONE_MAX_DB / 20.0);
            let low_gain = 10f32.powf(-tone * DRIVE_TONE_MAX_DB / 20.0);
            (
                tilt_sample(
                    wet_left,
                    &mut self.drive_tone_left,
                    self.drive_tone_coefficient,
                    low_gain,
                    high_gain,
                ),
                tilt_sample(
                    wet_right,
                    &mut self.drive_tone_right,
                    self.drive_tone_coefficient,
                    low_gain,
                    high_gain,
                ),
            )
        } else {
            self.drive_tone_left = 0.0;
            self.drive_tone_right = 0.0;
            (wet_left, wet_right)
        };

        crossfade(left, right, wet_left, wet_right, mix)
    }

    /// A four stage first-order allpass phaser with a stereo quadrature LFO.
    fn process_phaser(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let mix = params.phaser_mix.clamp(0.0, 1.0);

        if mix <= BYPASS_MIX {
            // No state may influence the sound while the phaser is off.
            self.phaser_phase = 0.0;
            self.phaser_left = [0.0; PHASER_STAGES];
            self.phaser_right = [0.0; PHASER_STAGES];
            self.phaser_feedback_left = 0.0;
            self.phaser_feedback_right = 0.0;
            return (left, right);
        }

        let rate = params.phaser_rate.clamp(0.0, self.sample_rate * 0.5);
        self.phaser_phase += rate / self.sample_rate;
        if self.phaser_phase >= 1.0 {
            self.phaser_phase -= self.phaser_phase.floor();
        }

        let phase = self.phaser_phase * PI * 2.0;
        let depth = params.phaser_depth.clamp(0.0, 1.0) * PHASER_OCTAVES;
        let (lfo_left, lfo_right) = phase.sin_cos();
        let frequency_left = PHASER_CENTER * (depth * lfo_left).exp2();
        let frequency_right = PHASER_CENTER * (depth * lfo_right).exp2();
        let coefficient_left = allpass_coefficient(frequency_left, self.sample_rate);
        let coefficient_right = allpass_coefficient(frequency_right, self.sample_rate);

        // The feedback wraps around the allpass chain. At zero the old path
        // is used unchanged, so the old phaser stays bit-exact.
        let feedback = params.phaser_feedback.clamp(0.0, 0.9);
        let (wet_left, wet_right) = if feedback > TONE_BYPASS {
            let fed_left = left + self.phaser_feedback_left * feedback;
            let fed_right = right + self.phaser_feedback_right * feedback;
            let wet_left = self.phaser_left.iter_mut().fold(fed_left, |sample, state| {
                allpass_stage(sample, coefficient_left, state)
            });
            let wet_right = self
                .phaser_right
                .iter_mut()
                .fold(fed_right, |sample, state| {
                    allpass_stage(sample, coefficient_right, state)
                });
            self.phaser_feedback_left = flush_denormal(wet_left);
            self.phaser_feedback_right = flush_denormal(wet_right);
            (wet_left, wet_right)
        } else {
            self.phaser_feedback_left = 0.0;
            self.phaser_feedback_right = 0.0;
            let wet_left = self.phaser_left.iter_mut().fold(left, |sample, state| {
                allpass_stage(sample, coefficient_left, state)
            });
            let wet_right = self.phaser_right.iter_mut().fold(right, |sample, state| {
                allpass_stage(sample, coefficient_right, state)
            });
            (wet_left, wet_right)
        };

        crossfade(left, right, wet_left, wet_right, mix)
    }

    /// A three band EQ built from two one-pole crossovers. The bands always
    /// sum back to the input, so three gains of 0 dB would be transparent (and
    /// the neutral case is bypassed outright).
    fn process_eq(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let low_db = params.eq_low.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
        let mid_db = params.eq_mid.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
        let high_db = params.eq_high.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);

        if low_db.abs() <= EQ_BYPASS_DB
            && mid_db.abs() <= EQ_BYPASS_DB
            && high_db.abs() <= EQ_BYPASS_DB
        {
            self.eq_low_left = 0.0;
            self.eq_low_right = 0.0;
            self.eq_high_left = 0.0;
            self.eq_high_right = 0.0;
            return (left, right);
        }

        self.eq_low_left += (left - self.eq_low_left) * self.eq_low_coefficient;
        self.eq_low_right += (right - self.eq_low_right) * self.eq_low_coefficient;
        self.eq_high_left += (left - self.eq_high_left) * self.eq_high_coefficient;
        self.eq_high_right += (right - self.eq_high_right) * self.eq_high_coefficient;
        self.eq_low_left = flush_denormal(self.eq_low_left);
        self.eq_low_right = flush_denormal(self.eq_low_right);
        self.eq_high_left = flush_denormal(self.eq_high_left);
        self.eq_high_right = flush_denormal(self.eq_high_right);

        let low_gain = 10f32.powf(low_db / 20.0);
        let mid_gain = 10f32.powf(mid_db / 20.0);
        let high_gain = 10f32.powf(high_db / 20.0);

        let (low_left, high_left) = (self.eq_low_left, left - self.eq_high_left);
        let (low_right, high_right) = (self.eq_low_right, right - self.eq_high_right);
        let mid_left = left - low_left - high_left;
        let mid_right = right - low_right - high_right;

        (
            low_left * low_gain + mid_left * mid_gain + high_left * high_gain,
            low_right * low_gain + mid_right * mid_gain + high_right * high_gain,
        )
    }

    /// A two tap chorus with a quadrature LFO.
    fn process_chorus(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let mix = params.chorus_mix.clamp(0.0, 1.0);

        let sample_rate = self.sample_rate;

        // The LFO.
        self.chorus_phase += (params.chorus_rate / sample_rate).max(0.0);
        if self.chorus_phase >= 1.0 {
            self.chorus_phase -= self.chorus_phase.floor();
        }
        let phase = self.chorus_phase * PI * 2.0;
        let lfo_left = phase.sin();
        let lfo_right = phase.cos();

        let depth = params.chorus_depth.clamp(0.0, 1.0) * CHORUS_MAX_DEPTH;
        let delay_left = (CHORUS_BASE_DELAY + depth * lfo_left) * sample_rate;
        let delay_right = (CHORUS_BASE_DELAY + depth * lfo_right) * sample_rate;

        // Write the dry signal into the delay lines.
        let index = self.chorus_index;
        self.chorus_left[index] = left;
        self.chorus_right[index] = right;
        let next_index = index + 1;
        self.chorus_index = if next_index == self.chorus_left.len() {
            0
        } else {
            next_index
        };

        let mut wet_left = read_interpolated(&self.chorus_left, index, delay_left);
        let mut wet_right = read_interpolated(&self.chorus_right, index, delay_right);

        // The wet signal is lowpassed, so that the chorus does not sound
        // harsh.
        self.chorus_lp_left += (wet_left - self.chorus_lp_left) * self.chorus_coefficient;
        self.chorus_lp_right += (wet_right - self.chorus_lp_right) * self.chorus_coefficient;
        self.chorus_lp_left = flush_denormal(self.chorus_lp_left);
        self.chorus_lp_right = flush_denormal(self.chorus_lp_right);
        wet_left = self.chorus_lp_left;
        wet_right = self.chorus_lp_right;

        // Widening works on the mid/side image of the wet signal. At zero the
        // old stereo chorus is returned unchanged.
        let width = params.chorus_width.clamp(0.0, 1.0);
        if width > TONE_BYPASS {
            let mid = (wet_left + wet_right) * 0.5;
            let side = (wet_left - wet_right) * 0.5;
            let side_gain = 1.0 + width * CHORUS_MAX_WIDTH;
            wet_left = mid + side * side_gain;
            wet_right = mid - side * side_gain;
        }

        if mix <= BYPASS_MIX {
            // Keep the delay line and the LFO moving so re-enabling the
            // chorus cannot play audio from minutes ago.
            return (left, right);
        }

        crossfade(left, right, wet_left, wet_right, mix)
    }

    /// A ping-pong delay with a fractional read position.
    fn process_delay(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let mix = params.delay_mix.clamp(0.0, 1.0);

        let len = self.delay_left.len();
        let max_delay = (len - 2) as f32;
        let target = (params.delay_time_s * self.sample_rate).clamp(1.0, max_delay);
        // Follow the requested time with a one-pole. A static delay adds
        // exactly zero on every step, so its output does not change at all;
        // a moved control glides instead of stepping the read position.
        let delay = match self.delay_time_samples {
            Some(current) => {
                (current + (target - current) * self.delay_smoothing).clamp(1.0, max_delay)
            }
            // The first sample (and the first after a reset) uses the
            // requested delay directly.
            None => target,
        };
        self.delay_time_samples = Some(delay);
        // A bypassed delay writes the dry signal without feedback, so old
        // repeats cannot survive in the line and reappear later.
        let feedback = if mix <= BYPASS_MIX {
            0.0
        } else {
            params.delay_feedback.clamp(0.0, 0.95)
        };

        // Read the delayed signal before writing the new one.
        let wet_left = read_interpolated(&self.delay_left, self.delay_index, delay);
        let wet_right = read_interpolated(&self.delay_right, self.delay_index, delay);

        // The feedback is damped and softly clipped, so that a long feedback
        // does not build up into a howl.
        let (mut feedback_left, mut feedback_right) = if feedback > 0.0 {
            ((wet_right * feedback).tanh(), (wet_left * feedback).tanh())
        } else {
            (0.0, 0.0)
        };
        // The feedback lowpass is fixed at 5 kHz when the damping is zero.
        // Raising the damping moves its cutoff down towards 150 Hz.
        let damping = params.delay_damping.clamp(0.0, 1.0);
        let damping_coefficient = if damping <= TONE_BYPASS {
            self.delay_coefficient
        } else {
            let cutoff = DELAY_LOWPASS + (DELAY_MIN_LOWPASS - DELAY_LOWPASS) * damping;
            lowpass_coefficient(cutoff, self.sample_rate)
        };
        self.delay_lp_left += (feedback_left - self.delay_lp_left) * damping_coefficient;
        self.delay_lp_right += (feedback_right - self.delay_lp_right) * damping_coefficient;
        self.delay_lp_left = flush_denormal(self.delay_lp_left);
        self.delay_lp_right = flush_denormal(self.delay_lp_right);
        feedback_left = self.delay_lp_left;
        feedback_right = self.delay_lp_right;
        // Ping-pong: the left channel is fed by the right one and vice versa.
        self.delay_left[self.delay_index] = left + feedback_left;
        self.delay_right[self.delay_index] = right + feedback_right;
        let next_index = self.delay_index + 1;
        self.delay_index = if next_index == len { 0 } else { next_index };

        if mix <= BYPASS_MIX {
            // The line is kept up to date while the delay is bypassed.
            return (left, right);
        }

        crossfade(left, right, wet_left, wet_right, mix)
    }

    /// A stereo-linked, feed-forward compressor with a soft gain envelope.
    ///
    /// The peak of the two channels sets the gain for both, so the stereo
    /// image stays stable. At mix zero, or with ratio 1 / threshold 0 dB and
    /// no makeup gain, the stage returns the input bit for bit.
    fn process_compressor(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        if !params.compressor_active() {
            self.compressor_gain = 1.0;
            return (left, right);
        }

        let mix = params.comp_mix.clamp(0.0, 1.0);
        let threshold = params.comp_threshold.clamp(-60.0, 0.0);
        let ratio = params.comp_ratio.clamp(1.0, 20.0);
        let makeup_db = params.comp_makeup.clamp(0.0, 24.0);

        // The detector is the stereo-linked peak. The floor keeps a silent
        // input out of the logarithm. A non-finite host value is ignored.
        let level = left.abs().max(right.abs());
        let level = if level.is_finite() {
            level.max(1e-12)
        } else {
            1e-12
        };
        let level_db = 20.0 * level.log10();
        let over_db = (level_db - threshold).max(0.0);
        let gain_db = -over_db * (1.0 - 1.0 / ratio);
        let target = 10f32.powf(gain_db / 20.0);

        let attack = smoothing_coefficient(params.comp_attack.max(0.1) * 0.001, self.sample_rate);
        let release =
            smoothing_coefficient(params.comp_release.max(10.0) * 0.001, self.sample_rate);
        let coefficient = if target < self.compressor_gain {
            attack
        } else {
            release
        };
        self.compressor_gain += (target - self.compressor_gain) * coefficient;
        self.compressor_gain = self.compressor_gain.clamp(0.0, 1.0);

        let makeup = 10f32.powf(makeup_db / 20.0);
        let wet_left = left * self.compressor_gain * makeup;
        let wet_right = right * self.compressor_gain * makeup;

        crossfade(left, right, wet_left, wet_right, mix)
    }

    /// A sine tremolo, or an auto-pan when the stereo switch is on.
    fn process_tremolo(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let depth = params.trem_depth.clamp(0.0, 1.0);
        if depth <= BYPASS_MIX {
            self.tremolo_phase = 0.0;
            return (left, right);
        }

        let rate = params.trem_rate.clamp(0.05, 20.0);
        self.tremolo_phase += rate / self.sample_rate;
        if self.tremolo_phase >= 1.0 {
            self.tremolo_phase -= self.tremolo_phase.floor();
        }
        let lfo = (self.tremolo_phase * PI * 2.0).sin();

        if params.trem_stereo {
            // Linear auto-pan: at full depth one channel is silent at each
            // end of the sweep. At zero depth both gains are exactly one.
            let left_gain = 1.0 - depth * (0.5 - 0.5 * lfo);
            let right_gain = 1.0 - depth * (0.5 + 0.5 * lfo);
            (left * left_gain, right * right_gain)
        } else {
            // The mono tremolo uses the same gain on both channels, so the
            // stereo image does not move.
            let gain = 1.0 - depth * (0.5 - 0.5 * lfo);
            (left * gain, right * gain)
        }
    }

    /// The original Freeverb-style reverb. Its wet signal is added to the
    /// dry signal, so the effect keeps the level behavior of the first release.
    fn process_reverb(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let mix = params.reverb_mix.clamp(0.0, 1.0);

        let (mut wet_left, mut wet_right) =
            self.legacy_reverb
                .process(left, right, params.reverb_decay, mix);

        if mix <= BYPASS_MIX {
            // The reverb keeps its lines current while bypassed, so enabling
            // it again cannot replay an old tail.
            self.reverb_tone_left = 0.0;
            self.reverb_tone_right = 0.0;
            return (left, right);
        }

        // The tone tilt colours only the wet signal. Zero skips the split, so
        // the Freeverb output stays bit-exact.
        let tone = params.reverb_tone.clamp(-1.0, 1.0);
        if tone.abs() > TONE_BYPASS {
            let high_gain = 10f32.powf(tone * REVERB_TONE_MAX_DB / 20.0);
            let low_gain = 10f32.powf(-tone * REVERB_TONE_MAX_DB / 20.0);
            wet_left = tilt_sample(
                wet_left,
                &mut self.reverb_tone_left,
                self.reverb_tone_coefficient,
                low_gain,
                high_gain,
            );
            wet_right = tilt_sample(
                wet_right,
                &mut self.reverb_tone_right,
                self.reverb_tone_coefficient,
                low_gain,
                high_gain,
            );
        } else {
            self.reverb_tone_left = 0.0;
            self.reverb_tone_right = 0.0;
        }

        // The Freeverb path adds its wet signal instead of crossfading, so
        // the original level behavior is kept.
        (left + wet_left * mix, right + wet_right * mix)
    }
}

/// Reads from a delay line with a fractional delay, using linear interpolation.
/// `write_index` is the position that was just written to.
#[inline]
fn read_interpolated(buffer: &[f32], write_index: usize, delay: f32) -> f32 {
    let len = buffer.len();

    // A single conditional wrap replaces `rem_euclid`: all the callers keep
    // the delay below the length of the line, so the read position is never
    // more than one line away from the valid range.
    let mut position = write_index as f32 - delay;
    if position < 0.0 {
        position += len as f32;
    }

    let floored = position.floor();
    let mut index = floored as usize;
    if index >= len {
        index -= len;
    }
    let fraction = position - floored;
    let next = if index + 1 == len { 0 } else { index + 1 };

    buffer[index] + (buffer[next] - buffer[index]) * fraction
}

impl Default for Fx {
    /// An effect chain without any allocated delay lines. Use [`Fx::new`] to
    /// create a usable one.
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            drive_adaa_left: AdaaTanh::default(),
            drive_adaa_right: AdaaTanh::default(),
            drive_tone_left: 0.0,
            drive_tone_right: 0.0,
            drive_tone_coefficient: 0.0,
            phaser_phase: 0.0,
            phaser_left: [0.0; PHASER_STAGES],
            phaser_right: [0.0; PHASER_STAGES],
            phaser_feedback_left: 0.0,
            phaser_feedback_right: 0.0,
            eq_low_left: 0.0,
            eq_low_right: 0.0,
            eq_high_left: 0.0,
            eq_high_right: 0.0,
            eq_low_coefficient: 0.0,
            eq_high_coefficient: 0.0,
            chorus_left: Vec::new(),
            chorus_right: Vec::new(),
            chorus_index: 0,
            chorus_phase: 0.0,
            chorus_lp_left: 0.0,
            chorus_lp_right: 0.0,
            chorus_coefficient: 0.0,
            delay_left: Vec::new(),
            delay_right: Vec::new(),
            delay_index: 0,
            delay_time_samples: None,
            delay_smoothing: 0.0,
            delay_lp_left: 0.0,
            delay_lp_right: 0.0,
            delay_coefficient: 0.0,
            legacy_reverb: LegacyReverb::default(),
            reverb_tone_left: 0.0,
            reverb_tone_right: 0.0,
            reverb_tone_coefficient: 0.0,
            compressor_gain: 1.0,
            tremolo_phase: 0.0,
            last_order: i32::MIN,
            order_cache: FxStage::ALL,
            bypassed: true,
            last_bypass_mask: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_impulse_produces_finite_output() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams {
            order: 0,
            bypass_mask: 0,
            global_mix: 1.0,
            drive_amount: 0.0,
            drive_mix: 0.0,
            phaser_rate: 0.3,
            phaser_depth: 0.5,
            phaser_mix: 0.0,
            eq_low: 0.0,
            eq_mid: 0.0,
            eq_high: 0.0,
            chorus_mix: 0.5,
            chorus_rate: 0.8,
            chorus_depth: 0.5,
            delay_time_s: 0.35,
            delay_feedback: 0.3,
            delay_mix: 0.5,
            reverb_mix: 0.5,
            reverb_decay: 0.6,
            ..FxParams::default()
        };

        for sample in 0..48_000 {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            let (left, right) = fx.process(input, input, &params);
            assert!(left.is_finite() && right.is_finite());
            assert!(left.abs() < 10.0 && right.abs() < 10.0);
        }
    }

    #[test]
    fn a_mix_of_zero_is_a_bypass() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams::default();

        for _ in 0..1_000 {
            let (left, right) = fx.process(0.5, -0.5, &params);
            assert_eq!(left, 0.5);
            assert_eq!(right, -0.5);
        }
    }

    #[test]
    fn the_delay_repeats_the_signal() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams {
            delay_mix: 1.0,
            delay_time_s: 0.1,
            delay_feedback: 0.0,
            ..FxParams::default()
        };

        // The impulse comes back after 100 ms.
        let mut first_repeat = None;
        for sample in 0..10_000 {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            let (left, _) = fx.process(input, 0.0, &params);
            if sample > 0 && left.abs() > 0.5 {
                first_repeat = Some(sample);
                break;
            }
        }

        let first_repeat: i32 = first_repeat.expect("the delay did not repeat the signal");
        assert!(
            (first_repeat - 4_800).abs() < 8,
            "the repeat came at {first_repeat}"
        );
    }

    #[test]
    fn a_jump_in_the_delay_time_moves_gradually() {
        let sample_rate = 48_000.0f32;
        let mut fx = Fx::new(sample_rate);
        let mut params = FxParams {
            delay_mix: 1.0,
            delay_time_s: 0.1,
            delay_feedback: 0.0,
            ..FxParams::default()
        };

        // The first processed sample uses the requested delay directly, with
        // no ramp up from zero.
        fx.process(0.0, 0.0, &params);
        let start = 0.1 * sample_rate;
        assert_eq!(fx.delay_time_samples.unwrap().to_bits(), start.to_bits());

        params.delay_time_s = 0.2;
        let target = 0.2 * sample_rate;
        let total = target - start;
        let mut previous = start;
        let mut last_step = total;

        for sample in 0..(sample_rate * 0.25) as usize {
            fx.process(0.0, 0.0, &params);
            let current = fx.delay_time_samples.unwrap();
            let step = current - previous;

            // The one-pole never runs backwards and it may land exactly on
            // the requested time while the control is held, which is fine.
            assert!(
                step >= 0.0,
                "the delay moved backwards at sample {sample}: {step}"
            );
            assert!(
                step <= total * 0.01,
                "the delay jumped {step} samples at sample {sample}"
            );
            // A one-pole moves the most on the first sample and then slows
            // down, so the read position never overshoots the control.
            assert!(
                step <= last_step,
                "the glide sped up at sample {sample}: {step} after {last_step}"
            );
            assert!(current <= target, "the delay overshot at sample {sample}");

            previous = current;
            last_step = step;
        }

        // Quarter of a second is twelve time constants, so the glide is done.
        assert!(
            (previous - target).abs() < 1.0,
            "the delay stopped at {previous} instead of {target}"
        );
    }

    #[test]
    fn a_full_bypass_returns_dry_and_clears_the_state() {
        let mut fx = Fx::new(48_000.0);
        let mut params = FxParams {
            drive_amount: 1.0,
            drive_mix: 1.0,
            phaser_rate: 5.0,
            phaser_depth: 1.0,
            phaser_mix: 1.0,
            eq_low: 12.0,
            eq_mid: -6.0,
            eq_high: 9.0,
            chorus_mix: 1.0,
            delay_mix: 1.0,
            delay_time_s: 0.1,
            delay_feedback: 0.85,
            reverb_mix: 1.0,
            reverb_decay: 1.0,
            ..FxParams::default()
        };

        // Excite every effect, then let the tails fill the delay lines and
        // the reverb.
        fx.process(1.0, -1.0, &params);
        for _ in 0..20_000 {
            fx.process(0.0, 0.0, &params);
        }

        // Bypassing the whole chain returns the dry signal and clears the
        // state on the first bypassed sample.
        params.drive_mix = 0.0;
        params.phaser_mix = 0.0;
        params.eq_low = 0.0;
        params.eq_mid = 0.0;
        params.eq_high = 0.0;
        params.chorus_mix = 0.0;
        params.delay_mix = 0.0;
        params.reverb_mix = 0.0;
        for _ in 0..1_000 {
            let (left, right) = fx.process(0.25, -0.75, &params);
            assert_eq!(left, 0.25);
            assert_eq!(right, -0.75);
        }
        assert!(fx.delay_left.iter().all(|&sample| sample == 0.0));
        assert!(fx.delay_right.iter().all(|&sample| sample == 0.0));
        assert!(fx.chorus_left.iter().all(|&sample| sample == 0.0));
        assert!(fx.chorus_right.iter().all(|&sample| sample == 0.0));
        assert!(fx.phaser_left.iter().all(|&state| state == 0.0));
        assert!(fx.phaser_right.iter().all(|&state| state == 0.0));
        assert_eq!(fx.phaser_phase, 0.0);
        assert_eq!(fx.eq_low_left, 0.0);
        assert_eq!(fx.eq_low_right, 0.0);
        assert_eq!(fx.eq_high_left, 0.0);
        assert_eq!(fx.eq_high_right, 0.0);
        assert_eq!(fx.compressor_gain, 1.0);
        assert_eq!(fx.tremolo_phase, 0.0);

        // The old and the new effects start from silence when they are
        // enabled again, with no tail left over from before the bypass.
        for stage in 0..FX_STAGE_COUNT {
            params.bypass_mask = 0;
            match stage {
                0 => params.drive_mix = 1.0,
                1 => params.phaser_mix = 1.0,
                2 => params.chorus_mix = 1.0,
                3 => params.delay_mix = 1.0,
                4 => params.reverb_mix = 1.0,
                5 => params.eq_low = 12.0,
                6 => {
                    params.comp_threshold = -30.0;
                    params.comp_ratio = 4.0;
                    params.comp_mix = 1.0;
                }
                _ => params.trem_depth = 1.0,
            }
            let mut peak: f32 = 0.0;
            for _ in 0..10_000 {
                let (left, right) = fx.process(0.0, 0.0, &params);
                peak = peak.max(left.abs()).max(right.abs());
            }
            assert!(
                peak < 1e-4,
                "stage {stage} replayed an old tail (peak {peak})"
            );
        }
    }

    /// A deterministic noise source for the tests.
    fn noise(state: &mut u32) -> f32 {
        *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((*state >> 8) as f32 / (1u32 << 23) as f32) * 2.0 - 1.0
    }

    /// The settings used by the order and bypass tests: every stage is
    /// audible, so a reorder or a bypass has something to change.
    fn all_stages_active() -> FxParams {
        FxParams {
            order: 0,
            bypass_mask: 0,
            global_mix: 1.0,
            drive_amount: 0.7,
            drive_mix: 0.6,
            drive_tone: 0.5,
            phaser_rate: 1.2,
            phaser_depth: 0.8,
            phaser_mix: 0.5,
            phaser_feedback: 0.4,
            eq_low: 8.0,
            eq_mid: -5.0,
            eq_high: 4.0,
            chorus_mix: 0.5,
            chorus_rate: 0.7,
            chorus_depth: 0.8,
            chorus_width: 0.7,
            delay_time_s: 0.071,
            delay_feedback: 0.5,
            delay_mix: 0.45,
            delay_damping: 0.6,
            reverb_mix: 0.4,
            reverb_decay: 0.8,
            reverb_tone: 0.5,
            comp_threshold: -30.0,
            comp_ratio: 4.0,
            comp_attack: 10.0,
            comp_release: 150.0,
            comp_makeup: 3.0,
            comp_mix: 0.8,
            trem_rate: 4.0,
            trem_depth: 0.6,
            trem_stereo: true,
        }
    }

    /// Renders a deterministic stereo signal and hashes the output bits.
    fn fx_hash(params: &FxParams, samples: usize) -> u64 {
        let mut fx = Fx::new(48_000.0);
        let mut state = 0x1234_5678;
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;

        for sample in 0..samples {
            let left = if sample % 9_600 == 0 {
                0.9
            } else {
                noise(&mut state) * 0.35
            };
            let right = if sample % 9_600 == 0 {
                -0.9
            } else {
                noise(&mut state) * 0.35
            };
            let (out_left, out_right) = fx.process(left, right, params);
            for bits in [out_left.to_bits(), out_right.to_bits()] {
                hash ^= bits as u64;
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        }

        hash
    }

    #[test]
    fn neutral_defaults_are_bit_exact_dry() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams::default();
        let mut state = 0x5EED_1234;

        for sample in 0..48_000 {
            let left = noise(&mut state) * 0.7;
            let right = noise(&mut state) * 0.7;
            let (out_left, out_right) = fx.process(left, right, &params);
            assert_eq!(out_left.to_bits(), left.to_bits(), "sample {sample}");
            assert_eq!(out_right.to_bits(), right.to_bits(), "sample {sample}");
        }
    }

    #[test]
    fn the_drive_changes_the_signal_and_stays_bounded() {
        let mut fx = Fx::new(48_000.0);
        let mut params = FxParams {
            drive_amount: 1.0,
            drive_mix: 1.0,
            ..FxParams::default()
        };

        let mut difference = 0.0f32;
        let mut peak: f32 = 0.0;
        for sample in 0..48_000 {
            let time = sample as f32 / 48_000.0;
            let input = (time * 440.0 * std::f32::consts::TAU).sin() * 0.8;
            let (left, right) = fx.process(input, input, &params);
            assert!(left.is_finite() && right.is_finite());
            difference += (left - input).abs();
            peak = peak.max(left.abs()).max(right.abs());
        }

        assert!(difference > 1.0, "the drive did not change the signal");
        assert!(peak <= 1.01, "the drive exceeded the tanh range ({peak})");

        // With the mix at zero the drive is a bit-exact bypass, even though
        // the amount is still at its maximum.
        params.drive_mix = 0.0;
        for _ in 0..1_000 {
            let (left, right) = fx.process(0.25, -0.75, &params);
            assert_eq!(left, 0.25);
            assert_eq!(right, -0.75);
        }
    }

    #[test]
    fn the_phaser_changes_the_signal_and_stays_bounded() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams {
            phaser_rate: 1.5,
            phaser_depth: 0.8,
            phaser_mix: 1.0,
            ..FxParams::default()
        };

        let mut difference = 0.0f32;
        let mut peak: f32 = 0.0;
        for sample in 0..48_000 {
            let time = sample as f32 / 48_000.0;
            let input = (time * 440.0 * std::f32::consts::TAU).sin() * 0.5;
            let (left, right) = fx.process(input, input, &params);
            assert!(left.is_finite() && right.is_finite());
            difference += (left - input).abs();
            peak = peak.max(left.abs()).max(right.abs());
        }

        assert!(difference > 1.0, "the phaser did not change the signal");
        assert!(peak < 1.0, "the phaser ran away ({peak})");

        // Turning the mix off returns exactly the dry input, and no allpass
        // state is left behind.
        let off = FxParams {
            phaser_mix: 0.0,
            ..params
        };
        for _ in 0..1_000 {
            let (left, right) = fx.process(0.25, -0.75, &off);
            assert_eq!(left, 0.25);
            assert_eq!(right, -0.75);
        }
        assert!(fx.phaser_left.iter().all(|&state| state == 0.0));
        assert!(fx.phaser_right.iter().all(|&state| state == 0.0));
        assert_eq!(fx.phaser_phase, 0.0);
    }

    #[test]
    fn the_eq_changes_the_bands_and_stays_bounded() {
        fn rms_of(frequency: f32, params: &FxParams) -> f32 {
            let mut fx = Fx::new(48_000.0);
            let mut sum = 0.0f32;
            let mut peak: f32 = 0.0;
            for sample in 0..48_000 {
                let time = sample as f32 / 48_000.0;
                let input = (time * frequency * std::f32::consts::TAU).sin() * 0.25;
                let (left, right) = fx.process(input, input, params);
                assert!(left.is_finite() && right.is_finite());
                // Ignore the first quarter second while the filters settle.
                if sample >= 12_000 {
                    sum += left * left;
                    peak = peak.max(left.abs()).max(right.abs());
                }
            }
            assert!(peak < 4.0, "the EQ ran away at {frequency} Hz ({peak})");
            (sum / 36_000.0).sqrt()
        }

        let input_rms = 0.25 / 2f32.sqrt();
        let boosted = rms_of(
            100.0,
            &FxParams {
                eq_low: 12.0,
                eq_mid: -12.0,
                eq_high: -12.0,
                ..FxParams::default()
            },
        );
        let cut = rms_of(
            10_000.0,
            &FxParams {
                eq_low: -12.0,
                eq_mid: -12.0,
                eq_high: 12.0,
                ..FxParams::default()
            },
        );

        assert!(
            boosted > input_rms * 2.0,
            "the low boost did not raise the level ({boosted})"
        );
        assert!(
            cut > input_rms * 2.0,
            "the high boost did not raise the level ({cut})"
        );
    }

    #[test]
    fn reset_clears_the_new_effects() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams {
            drive_amount: 1.0,
            drive_mix: 1.0,
            phaser_mix: 1.0,
            eq_low: 12.0,
            eq_mid: -12.0,
            eq_high: 12.0,
            ..FxParams::default()
        };

        for sample in 0..4_800 {
            let input = (sample as f32 * 0.01).sin();
            fx.process(input, input, &params);
        }
        assert!(
            fx.phaser_left.iter().any(|&state| state != 0.0)
                || fx.eq_low_left != 0.0
                || fx.eq_high_left != 0.0
        );

        fx.reset();
        assert_eq!(fx.phaser_phase, 0.0);
        assert!(fx.phaser_left.iter().all(|&state| state == 0.0));
        assert!(fx.phaser_right.iter().all(|&state| state == 0.0));
        assert_eq!(fx.eq_low_left, 0.0);
        assert_eq!(fx.eq_low_right, 0.0);
        assert_eq!(fx.eq_high_left, 0.0);
        assert_eq!(fx.eq_high_right, 0.0);
    }

    #[test]
    fn the_compressor_reduces_and_links_the_channels() {
        let params = FxParams {
            comp_threshold: -30.0,
            comp_ratio: 8.0,
            comp_attack: 0.1,
            comp_release: 50.0,
            comp_makeup: 0.0,
            comp_mix: 1.0,
            ..FxParams::default()
        };
        let mut fx = Fx::new(48_000.0);

        // A loud left channel and a quiet right channel share one gain
        // envelope, so the right channel is reduced by the same factor.
        let mut last = (0.0, 0.0);
        for _ in 0..4_800 {
            last = fx.process(0.8, 0.1, &params);
        }
        let gain = last.0 / 0.8;
        assert!(
            gain < 0.2,
            "the compressor did not reduce the level ({gain})"
        );
        assert!(
            (last.1 - 0.1 * gain).abs() < 1e-4,
            "the compressor did not link the channels ({} vs {})",
            last.1,
            0.1 * gain
        );

        // Ratio 1, a 0 dB threshold without makeup, and a zero mix are all
        // exact neutral, even with an otherwise aggressive setting.
        for neutral in [
            FxParams {
                comp_threshold: -40.0,
                comp_ratio: 1.0,
                comp_makeup: 0.0,
                comp_mix: 1.0,
                ..params
            },
            FxParams {
                comp_threshold: 0.0,
                comp_ratio: 20.0,
                comp_makeup: 0.0,
                comp_mix: 1.0,
                ..params
            },
            FxParams {
                comp_mix: 0.0,
                ..params
            },
        ] {
            let mut fx = Fx::new(48_000.0);
            for _ in 0..1_000 {
                let (left, right) = fx.process(0.37, -0.21, &neutral);
                assert_eq!(left.to_bits(), 0.37f32.to_bits());
                assert_eq!(right.to_bits(), (-0.21f32).to_bits());
            }
        }

        // Makeup alone lifts a quiet signal without compressing it.
        let makeup = FxParams {
            comp_threshold: 0.0,
            comp_ratio: 1.0,
            comp_makeup: 12.0,
            comp_mix: 1.0,
            ..params
        };
        let mut fx = Fx::new(48_000.0);
        let (left, right) = fx.process(0.1, -0.1, &makeup);
        let expected = 0.1 * 10f32.powf(12.0 / 20.0);
        assert!((left - expected).abs() < 1e-5, "makeup was {left}");
        assert!((right + expected).abs() < 1e-5, "makeup was {right}");
    }

    #[test]
    fn the_tremolo_and_auto_pan_move_the_level_and_never_blow_up() {
        // At 10 Hz one sample step is 1/4800 of a cycle. After 1200 samples
        // the sine is +1, after 3600 samples it is -1.
        let tremolo = FxParams {
            trem_rate: 10.0,
            trem_depth: 1.0,
            trem_stereo: false,
            ..FxParams::default()
        };
        let mut fx = Fx::new(48_000.0);
        let mut output = (0.0, 0.0);
        for sample in 0..3_600 {
            output = fx.process(0.5, -0.5, &tremolo);
            if sample == 1_199 {
                // The positive peak of the sine is unity gain.
                assert!((output.0 - 0.5).abs() < 1e-5);
                assert!((output.1 + 0.5).abs() < 1e-5);
            }
        }
        // The negative peak silences both channels of the mono tremolo.
        assert!(output.0.abs() < 1e-5 && output.1.abs() < 1e-5);

        // The stereo mode pans: at the positive peak the left channel is
        // open and the right channel is closed, and vice versa.
        let pan = FxParams {
            trem_stereo: true,
            ..tremolo
        };
        let mut fx = Fx::new(48_000.0);
        let mut output = (0.0, 0.0);
        for sample in 0..3_600 {
            output = fx.process(0.5, -0.5, &pan);
            if sample == 1_199 {
                assert!((output.0 - 0.5).abs() < 1e-5);
                assert!(output.1.abs() < 1e-5);
            }
        }
        assert!(output.0.abs() < 1e-5);
        assert!((output.1 + 0.5).abs() < 1e-5);

        // Depth zero is a bit-exact bypass for either mode.
        for stereo in [false, true] {
            let params = FxParams {
                trem_depth: 0.0,
                trem_stereo: stereo,
                ..tremolo
            };
            let mut fx = Fx::new(48_000.0);
            for _ in 0..1_000 {
                let (left, right) = fx.process(0.25, -0.75, &params);
                assert_eq!(left.to_bits(), 0.25f32.to_bits());
                assert_eq!(right.to_bits(), (-0.75f32).to_bits());
            }
        }
    }

    #[test]
    fn the_default_order_bypass_and_mix_keep_the_old_output() {
        // The hash pins the fixed-order legacy chain. The default order 0,
        // bypass mask 0 and global mix 1 must reproduce it bit for bit.
        let params = FxParams {
            drive_amount: 0.6,
            drive_mix: 0.5,
            drive_tone: 0.0,
            phaser_rate: 1.1,
            phaser_depth: 0.7,
            phaser_mix: 0.5,
            phaser_feedback: 0.0,
            eq_low: 6.0,
            eq_mid: -4.0,
            eq_high: 3.0,
            chorus_mix: 0.5,
            chorus_rate: 0.6,
            chorus_depth: 0.7,
            chorus_width: 0.0,
            delay_time_s: 0.123,
            delay_feedback: 0.45,
            delay_mix: 0.4,
            delay_damping: 0.0,
            reverb_mix: 0.35,
            reverb_decay: 0.7,
            reverb_tone: 0.0,
            ..FxParams::default()
        };

        // The two new stages are appended at the end and are neutral by
        // default; the old fixed-order hash must stay bit-identical.
        assert_eq!(params.order, 0);
        assert_eq!(params.bypass_mask, 0);
        assert_eq!(params.global_mix, 1.0);
        assert_eq!(params.comp_mix, 0.0);
        assert_eq!(params.trem_depth, 0.0);
        assert_eq!(fx_hash(&params, 96_000), 0x09d4_d96d_8d80_07e5);
    }

    #[test]
    fn the_new_components_at_zero_keep_the_old_chain_and_change_at_the_extremes() {
        // Every old stage is active, while all five new component controls
        // sit exactly at zero.
        let base = FxParams {
            drive_tone: 0.0,
            phaser_feedback: 0.0,
            chorus_width: 0.0,
            delay_damping: 0.0,
            reverb_tone: 0.0,
            ..all_stages_active()
        };
        let baseline = fx_hash(&base, 48_000);

        for (name, variant) in [
            (
                "drive_tone",
                FxParams {
                    drive_tone: 1.0,
                    ..base
                },
            ),
            (
                "phaser_feedback",
                FxParams {
                    phaser_feedback: 0.8,
                    ..base
                },
            ),
            (
                "chorus_width",
                FxParams {
                    chorus_width: 1.0,
                    ..base
                },
            ),
            (
                "delay_damping",
                FxParams {
                    delay_damping: 1.0,
                    ..base
                },
            ),
            (
                "reverb_tone",
                FxParams {
                    reverb_tone: 1.0,
                    ..base
                },
            ),
        ] {
            assert_ne!(
                fx_hash(&variant, 48_000),
                baseline,
                "{name} did not change the chain at its extreme"
            );
        }

        // Zero keeps the old chain even when the parameter is explicitly
        // written after the chain has already run.
        let mut fx = Fx::new(48_000.0);
        for _ in 0..4_800 {
            fx.process(
                0.3,
                -0.3,
                &FxParams {
                    drive_tone: 1.0,
                    ..base
                },
            );
        }
        for _ in 0..1_000 {
            let (left, right) = fx.process(0.3, -0.3, &base);
            assert!(left.is_finite() && right.is_finite());
        }
    }

    #[test]
    fn the_old_orders_keep_their_rotation_semantics() {
        use FxStage::{Chorus, Compressor, Delay, Drive, Eq, Phaser, Reverb, Tremolo};

        assert_eq!(
            stage_order(0),
            [
                Drive, Phaser, Chorus, Delay, Reverb, Eq, Compressor, Tremolo
            ],
            "order 0 must be the original chain with the new stages appended"
        );
        assert_eq!(
            stage_order(1),
            [
                Phaser, Chorus, Delay, Reverb, Eq, Drive, Compressor, Tremolo
            ]
        );
        assert_eq!(
            stage_order(2),
            [
                Chorus, Delay, Reverb, Eq, Drive, Phaser, Compressor, Tremolo
            ]
        );
        assert_eq!(
            stage_order(3),
            [
                Delay, Reverb, Eq, Drive, Phaser, Chorus, Compressor, Tremolo
            ]
        );
        assert_eq!(
            stage_order(4),
            [
                Reverb, Eq, Drive, Phaser, Chorus, Delay, Compressor, Tremolo
            ]
        );
        assert_eq!(
            stage_order(5),
            [
                Eq, Drive, Phaser, Chorus, Delay, Reverb, Compressor, Tremolo
            ]
        );

        // Every old order value migrates to exactly its old six-stage chain
        // with the two neutral new stages at the end.
        for order in 0..=FX_LEGACY_ORDER_MAX {
            let stages = stage_order(order);
            assert_eq!(
                &stages[..FX_LEGACY_STAGE_COUNT],
                &legacy_stage_order(order)[..]
            );
            assert_eq!(stages[6], Compressor, "order {order}");
            assert_eq!(stages[7], Tremolo, "order {order}");
            assert_eq!(stages, migrated_stage_order(order), "order {order}");
        }
    }

    #[test]
    fn the_full_order_range_starts_after_the_old_ones() {
        use FxStage::Compressor;
        use FxStage::Tremolo;

        assert_eq!(FX_ORDER_COUNT, 40_320);
        assert_eq!(FX_ORDER_MAX, 40_319);
        assert_eq!(FX_LEGACY_ORDER_COUNT, 720);
        assert_eq!(FX_BYPASS_ALL, 0xff);

        // Orders at and after the legacy range may put the new stages
        // anywhere in the chain.
        assert_ne!(
            stage_order(FX_LEGACY_ORDER_COUNT as i32)[6..],
            [Compressor, Tremolo]
        );
        for order in [720, 721, 10_000, FX_ORDER_MAX] {
            assert_eq!(stage_order_index(stage_order(order)), order);
        }
    }

    #[test]
    fn every_order_is_a_unique_reversible_permutation() {
        use std::collections::HashSet;

        let mut seen = HashSet::with_capacity(FX_ORDER_COUNT);
        for order in 0..FX_ORDER_COUNT as i32 {
            let stages = stage_order(order);

            // Every stage must appear exactly once.
            let mut present = [false; FX_STAGE_COUNT];
            for stage in stages {
                assert!(
                    !present[stage.index()],
                    "order {order} repeats {stage:?}: {stages:?}"
                );
                present[stage.index()] = true;
            }
            assert!(
                present.iter().all(|&value| value),
                "order {order} is incomplete"
            );

            // The table covers every permutation exactly once and the inverse
            // index returns the original order.
            assert!(
                seen.insert(stages.map(|stage| stage.index() as u8)),
                "order {order} duplicates another permutation"
            );
            assert_eq!(stage_order_index(stages), order);
        }
        assert_eq!(seen.len(), FX_ORDER_COUNT);
    }

    #[test]
    fn out_of_range_orders_clamp_to_the_table_ends() {
        assert_eq!(stage_order(-1), stage_order(0));
        assert_eq!(stage_order(i32::MIN), stage_order(0));
        assert_eq!(stage_order(FX_ORDER_MAX + 1), stage_order(FX_ORDER_MAX));
        assert_eq!(stage_order(i32::MAX), stage_order(FX_ORDER_MAX));
        assert_ne!(stage_order(FX_ORDER_MAX), stage_order(0));
    }

    #[test]
    fn reordering_the_chain_changes_the_result() {
        let original = all_stages_active();

        // Order 1 is the old rotation; order 317 is one of the newly
        // available six-stage permutations, and order 10_000 is a full
        // eight-stage permutation. All must really reach the processor.
        for order in [1, 317, 10_000] {
            assert_ne!(stage_order(order), stage_order(0));

            let reordered = FxParams { order, ..original };
            let mut straight = Fx::new(48_000.0);
            let mut shuffled = Fx::new(48_000.0);
            let mut state = 0x0BAD_5EED;
            let mut difference = 0.0f32;

            for _ in 0..48_000 {
                let left = noise(&mut state) * 0.4;
                let right = noise(&mut state) * 0.4;
                let (straight_left, straight_right) = straight.process(left, right, &original);
                let (shuffled_left, shuffled_right) = shuffled.process(left, right, &reordered);
                difference +=
                    (straight_left - shuffled_left).abs() + (straight_right - shuffled_right).abs();
            }

            assert!(
                difference > 1.0,
                "order {order} sounded identical to the original chain ({difference})"
            );
        }
    }

    #[test]
    fn every_bypass_bit_matches_a_zero_mix_for_its_stage() {
        for stage in FxStage::ALL {
            let base = all_stages_active();
            let mut bypassed = Fx::new(48_000.0);
            let mut zeroed = Fx::new(48_000.0);
            let mut active = Fx::new(48_000.0);
            let bypass_params = FxParams {
                bypass_mask: stage.bit() as i32,
                ..base
            };
            let mut zero_params = base;
            match stage {
                FxStage::Drive => zero_params.drive_mix = 0.0,
                FxStage::Phaser => zero_params.phaser_mix = 0.0,
                FxStage::Chorus => zero_params.chorus_mix = 0.0,
                FxStage::Delay => zero_params.delay_mix = 0.0,
                FxStage::Reverb => zero_params.reverb_mix = 0.0,
                FxStage::Eq => {
                    zero_params.eq_low = 0.0;
                    zero_params.eq_mid = 0.0;
                    zero_params.eq_high = 0.0;
                }
                FxStage::Compressor => zero_params.comp_mix = 0.0,
                FxStage::Tremolo => zero_params.trem_depth = 0.0,
            }

            let mut state = 0xB17B_17B1;
            let mut active_difference = 0.0f32;
            for sample in 0..48_000 {
                let left = noise(&mut state) * 0.4;
                let right = noise(&mut state) * 0.4;
                let (bypassed_left, bypassed_right) = bypassed.process(left, right, &bypass_params);
                let (zeroed_left, zeroed_right) = zeroed.process(left, right, &zero_params);
                let (active_left, active_right) = active.process(left, right, &base);

                assert_eq!(
                    bypassed_left.to_bits(),
                    zeroed_left.to_bits(),
                    "{stage:?} left at sample {sample}"
                );
                assert_eq!(
                    bypassed_right.to_bits(),
                    zeroed_right.to_bits(),
                    "{stage:?} right at sample {sample}"
                );
                active_difference +=
                    (bypassed_left - active_left).abs() + (bypassed_right - active_right).abs();
            }

            assert!(
                active_difference > 1e-3,
                "bypassing {stage:?} did not change the chain"
            );
        }
    }

    #[test]
    fn a_zero_global_mix_returns_dry_and_clears_every_stage() {
        let mut params = all_stages_active();
        let mut fx = Fx::new(48_000.0);
        let mut state = 0x5EED_CAFE;

        // Fill every line and filter with a long, loud signal.
        for sample in 0..48_000 {
            let left = if sample == 0 {
                1.0
            } else {
                noise(&mut state) * 0.6
            };
            let right = if sample == 0 {
                -1.0
            } else {
                noise(&mut state) * 0.6
            };
            fx.process(left, right, &params);
        }

        params.global_mix = 0.0;
        for _ in 0..1_000 {
            let left = noise(&mut state) * 0.8;
            let right = noise(&mut state) * 0.8;
            let (out_left, out_right) = fx.process(left, right, &params);
            assert_eq!(out_left.to_bits(), left.to_bits());
            assert_eq!(out_right.to_bits(), right.to_bits());
        }

        assert!(fx.delay_left.iter().all(|&sample| sample == 0.0));
        assert!(fx.delay_right.iter().all(|&sample| sample == 0.0));
        assert!(fx.chorus_left.iter().all(|&sample| sample == 0.0));
        assert!(fx.chorus_right.iter().all(|&sample| sample == 0.0));
        assert!(fx.phaser_left.iter().all(|&state| state == 0.0));
        assert!(fx.phaser_right.iter().all(|&state| state == 0.0));
        assert_eq!(fx.eq_low_left, 0.0);
        assert_eq!(fx.eq_low_right, 0.0);
        assert_eq!(fx.eq_high_left, 0.0);
        assert_eq!(fx.eq_high_right, 0.0);

        // Turning the chain back on starts from silence instead of a tail.
        params.global_mix = 1.0;
        let mut peak: f32 = 0.0;
        for _ in 0..10_000 {
            let (left, right) = fx.process(0.0, 0.0, &params);
            peak = peak.max(left.abs()).max(right.abs());
        }
        assert!(peak < 1e-6, "a stale tail came back (peak {peak})");
    }

    #[test]
    fn a_global_mix_of_one_is_the_exact_wet_signal() {
        let params = all_stages_active();
        let unity = FxParams {
            global_mix: 1.0,
            ..params
        };
        let half = FxParams {
            global_mix: 0.5,
            ..params
        };
        let mut wet = Fx::new(48_000.0);
        let mut crossfaded = Fx::new(48_000.0);
        let mut state = 0xC0FF_EE00;

        for sample in 0..48_000 {
            let left = noise(&mut state) * 0.5;
            let right = noise(&mut state) * 0.5;
            let (wet_left, wet_right) = wet.process(left, right, &unity);
            let (out_left, out_right) = crossfaded.process(left, right, &half);

            let expected_left = left * 0.5 + wet_left * 0.5;
            let expected_right = right * 0.5 + wet_right * 0.5;
            assert_eq!(
                out_left.to_bits(),
                expected_left.to_bits(),
                "sample {sample}"
            );
            assert_eq!(
                out_right.to_bits(),
                expected_right.to_bits(),
                "sample {sample}"
            );
        }
    }

    #[test]
    fn an_all_stage_bypass_mask_returns_dry_and_clears_the_tail() {
        let mut params = all_stages_active();
        let mut fx = Fx::new(48_000.0);
        let mut state = 0xDEAD_BEEF;

        // Build up delay and reverb tails.
        for sample in 0..48_000 {
            let left = if sample % 4_800 == 0 {
                0.9
            } else {
                noise(&mut state) * 0.4
            };
            let right = if sample % 4_800 == 0 {
                -0.9
            } else {
                noise(&mut state) * 0.4
            };
            fx.process(left, right, &params);
        }

        params.bypass_mask = FX_BYPASS_ALL as i32;
        for _ in 0..1_000 {
            let left = noise(&mut state) * 0.7;
            let right = noise(&mut state) * 0.7;
            let (out_left, out_right) = fx.process(left, right, &params);
            assert_eq!(out_left.to_bits(), left.to_bits());
            assert_eq!(out_right.to_bits(), right.to_bits());
        }
        assert!(fx.delay_left.iter().all(|&sample| sample == 0.0));
        assert!(fx.delay_right.iter().all(|&sample| sample == 0.0));
        assert!(fx.chorus_left.iter().all(|&sample| sample == 0.0));
        assert!(fx.chorus_right.iter().all(|&sample| sample == 0.0));

        // Unmasking the chain while the input is silent must not replay the
        // audio that was captured before the bypass.
        params.bypass_mask = 0;
        let mut peak: f32 = 0.0;
        for _ in 0..10_000 {
            let (left, right) = fx.process(0.0, 0.0, &params);
            peak = peak.max(left.abs()).max(right.abs());
        }
        assert!(peak < 1e-6, "the unmasked chain replayed a tail ({peak})");
    }

    #[test]
    fn order_bypass_and_mix_changes_stay_finite() {
        let mut params = all_stages_active();
        params.drive_amount = 1.0;
        params.delay_feedback = 0.95;
        params.reverb_decay = 1.0;
        let mut fx = Fx::new(48_000.0);
        let mut state = 0x1357_9BDF;
        let mixes = [1.0, 0.75, 0.25, 0.0, 0.5];
        let mut peak: f32 = 0.0;

        for sample in 0..192_000 {
            if sample % 1_024 == 0 {
                let block = sample / 1_024;
                params.order = ((block * 7_919) % FX_ORDER_COUNT) as i32;
                params.bypass_mask = ((block * 13) % (FX_BYPASS_ALL as usize + 1)) as i32;
                params.global_mix = mixes[block % mixes.len()];
            }

            let left = if sample % 480 == 0 {
                0.9
            } else {
                noise(&mut state) * 0.5
            };
            let right = if sample % 720 == 0 {
                -0.9
            } else {
                noise(&mut state) * 0.5
            };
            let (out_left, out_right) = fx.process(left, right, &params);
            assert!(out_left.is_finite() && out_right.is_finite());
            peak = peak.max(out_left.abs()).max(out_right.abs());
        }

        assert!(peak < 20.0, "the changing chain ran away ({peak})");
        assert!(fx.sample_rate.is_finite());
    }
}
