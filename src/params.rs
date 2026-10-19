use nice_plug::prelude::*;

/// The different oscillator shapes Sushi can generate.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum Waveform {
    Sine,
    #[default]
    Saw,
    Square,
}

#[derive(Params)]
pub struct SushiParams {
    /// Master output gain, stored as a linear amplitude.
    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "wave"]
    pub waveform: EnumParam<Waveform>,

    /// Filter cutoff in Hz.
    #[id = "cutoff"]
    pub cutoff: FloatParam,

    /// Filter resonance, `0..1`.
    #[id = "reso"]
    pub resonance: FloatParam,

    /// Amplitude envelope attack time in milliseconds.
    #[id = "attack"]
    pub attack: FloatParam,

    /// Amplitude envelope decay time in milliseconds.
    #[id = "decay"]
    pub decay: FloatParam,

    /// Amplitude envelope sustain level, `0..1`.
    #[id = "sustain"]
    pub sustain: FloatParam,

    /// Amplitude envelope release time in milliseconds.
    #[id = "release"]
    pub release: FloatParam,
}

impl Default for SushiParams {
    fn default() -> Self {
        Self {
            gain: FloatParam::new(
                "Gain",
                util::db_to_gain(-12.0),
                FloatRange::Linear {
                    min: util::db_to_gain(-36.0),
                    max: util::db_to_gain(0.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(5.0))
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            waveform: EnumParam::new("Waveform", Waveform::default()),
            cutoff: FloatParam::new(
                "Cutoff",
                8_000.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(10.0))
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(0))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            resonance: FloatParam::new(
                "Resonance",
                0.2,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.99,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            attack: FloatParam::new(
                "Attack",
                10.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 2_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_step_size(0.1)
            .with_unit(" ms"),
            decay: FloatParam::new(
                "Decay",
                300.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 4_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_step_size(0.1)
            .with_unit(" ms"),
            sustain: FloatParam::new("Sustain", 0.7, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            release: FloatParam::new(
                "Release",
                400.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 8_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_step_size(0.1)
            .with_unit(" ms"),
        }
    }
}
