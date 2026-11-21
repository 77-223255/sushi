
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

/// The documents folder from the XDG user directories file, if the user has
/// configured one. This keeps localized folders such as `~/文档` working.
fn xdg_documents_dir() -> Option<PathBuf> {
    let home = home_dir()?;
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let text = fs::read_to_string(config_home.join("user-dirs.dirs")).ok()?;

    for line in text.lines() {
        let line = line.trim();
        let Some(value) = line.strip_prefix("XDG_DOCUMENTS_DIR=") else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        let expanded = if value == "$HOME" {
            home.to_string_lossy().into_owned()
        } else if let Some(rest) = value.strip_prefix("$HOME/") {
            home.join(rest).to_string_lossy().into_owned()
        } else {
            value.to_string()
        };
        let path = PathBuf::from(expanded);
        if path.is_absolute() {
            return Some(path);
        }
    }

    None
}

/// The folder in which the patches are stored by default.
pub fn default_patch_dir() -> PathBuf {
    match xdg_documents_dir().or_else(|| home_dir().map(|home| home.join("Documents"))) {
        Some(documents) => documents.join("Sushi Patches"),
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

/// The name with which a variant is written into a patch.
///
/// The second 24 dB alias is displayed as "24 dB", but old patches already
/// used that text for the first one. It is therefore spelled out so that a
/// patch can never change the saved slope value by accident.
fn variant_to_json_name(variants: &[&str], name: &str) -> String {
    if name == "24 dB" && variants.contains(&"24 dB OLD") {
        "24 dB NEW".to_string()
    } else {
        name.to_string()
    }
}

/// The index of a variant that was read from a patch.
///
/// "24 dB" is the name that old patches gave to the first 24 dB alias, so it
/// keeps resolving to "24 dB OLD". The second alias is only selected by the
/// explicit "24 dB NEW" spelling, which is what this version writes.
fn variant_index_from_name(variants: &[&str], name: &str) -> Option<usize> {
    let wanted = normalize_name(name);

    if variants.contains(&"24 dB OLD") {
        if wanted == normalize_name("24 dB") {
            return variants.iter().position(|variant| *variant == "24 dB OLD");
        }
        if wanted == normalize_name("24 dB NEW") {
            return variants.iter().position(|variant| *variant == "24 dB");
        }
    }

    variants
        .iter()
        .position(|variant| normalize_name(variant) == wanted)
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
            Some(name) => Value::String(variant_to_json_name(T::variants(), name)),
            None => Value::Null,
        }
    }

    fn read_json(&self, value: &Value) -> Result<T, String> {
        if let Some(name) = value.as_str() {
            if let Some(index) = variant_index_from_name(T::variants(), name) {
                return Ok(T::from_index(index));
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

        /// Checks every value of a JSON object without changing the
        /// synthesizer. A broken patch must not leave the current sound in a
        /// half loaded state.
        fn params_validate_json(
            params: &SushiParams,
            map: &Map<String, Value>,
        ) -> Result<(), String> {
            for (id, value) in map {
                match id.as_str() {
                    $(
                        $id => {
                            params
                                .$field
                                .read_json(value)
                                .map_err(|error| format!("{id}: {error}"))?;
                        }
                    )*
                    _ => {}
                }
            }
            Ok(())
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
    "sub_level" => sub_level,
    "osc_sync" => osc_sync,
    "fm_amount" => fm_amount,
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
    "filter_slope" => filter_slope,
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
    "lfo1_sync" => lfo1_sync,
    "lfo1_div" => lfo1_div,
    "lfo1_retrigger" => lfo1_retrigger,
    "lfo2_sync" => lfo2_sync,
    "lfo2_div" => lfo2_div,
    "lfo2_retrigger" => lfo2_retrigger,
    "drive_amount" => drive_amount,
    "drive_mix" => drive_mix,
    "phaser_rate" => phaser_rate,
    "phaser_depth" => phaser_depth,
    "phaser_mix" => phaser_mix,
    "eq_low" => eq_low,
    "eq_mid" => eq_mid,
    "eq_high" => eq_high,
    "hpf_cutoff" => hpf_cutoff,
    "vca_width" => vca_width,
    "vca_pan" => vca_pan,
    "ae_curve" => ae_curve,
    "fe_curve" => fe_curve,
    "ring_mod" => ring_mod,
    "osc_phase_reset" => osc_phase_reset,
    "lfo1_phase" => lfo1_phase,
    "lfo2_phase" => lfo2_phase,
    "lfo1_curve" => lfo1_curve,
    "lfo2_curve" => lfo2_curve,
    "arp_chord" => arp_chord,
    "arp_strum" => arp_strum,
    "arp_probability" => arp_probability,
    "fx_order" => fx_order,
    "fx_bypass_mask" => fx_bypass_mask,
    "fx_global_mix" => fx_global_mix,
    "drive_tone" => drive_tone,
    "phaser_feedback" => phaser_feedback,
    "chorus_width" => chorus_width,
    "delay_damping" => delay_damping,
    "reverb_tone" => reverb_tone,
    "comp_threshold" => comp_threshold,
    "comp_ratio" => comp_ratio,
    "comp_attack" => comp_attack,
    "comp_release" => comp_release,
    "comp_makeup" => comp_makeup,
    "comp_mix" => comp_mix,
    "trem_rate" => trem_rate,
    "trem_depth" => trem_depth,
    "trem_stereo" => trem_stereo,
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
    // Validate first: applying the defaults before discovering a bad value
    // would destroy the sound that is currently loaded.
    params_validate_json(params, &map)?;
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


