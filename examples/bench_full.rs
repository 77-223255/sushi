use nice_plug::midi::{Channel, Key};
use std::time::Instant;
use sushi::dsp::fx::{Fx, FxParams};
use sushi::dsp::voice::{OscSettings, Voice, VoiceSettings};
use sushi::params::{FilterMode, FilterSlope, LfoTarget, Waveform};

fn main() {
    let sr = 48_000.0f32;
    let seconds = 5.0;
    let frames = (sr * seconds) as usize;
    let settings = VoiceSettings {
        osc1: OscSettings {
            waveform: Waveform::Saw,
            pulse_width: 0.5,
            level: 0.6,
            semitones: 0.0,
        },
        osc2: OscSettings {
            waveform: Waveform::Saw,
            pulse_width: 0.5,
            level: 0.4,
            semitones: -12.0,
        },
        unison: 4,
        detune_cents: 18.0,
        spread: 0.7,
        filter_mode: FilterMode::Lowpass,
        filter_slope: FilterSlope::FourPole,
        resonance: 0.35,
        filter_env_octaves: 2.0,
        filter_drive: 0.3,
        lfo1_target: LfoTarget::Filter,
        lfo1_amount: 0.1,
        drift_amount: 0.2,
        ..VoiceSettings::default()
    };
    let mut voices: Vec<Voice> = (0..16)
        .map(|i| {
            Voice::new(
                i,
                Channel::Number(0),
                Key::Number(48 + (i as u8 % 12)),
                i as u64,
                0.8,
                &settings,
            )
        })
        .collect();
    let mut fx = Fx::new(sr);
    let fx_params = FxParams {
        chorus_mix: 0.25,
        delay_mix: 0.25,
        reverb_mix: 0.3,
        reverb_decay: 0.7,
        ..FxParams::default()
    };
    let start = Instant::now();
    let mut peak = 0.0f32;
    for _ in 0..frames {
        let mut left = 0.0;
        let mut right = 0.0;
        for voice in voices.iter_mut() {
            let (l, r) = voice.next_sample(&settings, 1200.0, 0.0, 0.0, sr);
            left += l;
            right += r;
        }
        let (l, r) = fx.process(left, right, &fx_params);
        peak = peak.max(l.abs()).max(r.abs());
    }
    let elapsed = start.elapsed().as_secs_f32();
    println!(
        "{frames} frames / {seconds}s audio in {elapsed:.3}s -> {:.1}x realtime, peak {peak:.3}",
        seconds / elapsed
    );
}
