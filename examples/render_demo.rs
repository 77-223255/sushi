//! Offline demo renderer. Produces `target/demo.wav` so the DSP can be checked
//! without opening a DAW.
//!
//! Run with:
//!
//! ```sh
//! cargo run --release --example render_demo
//! pw-play target/demo.wav
//! ```

use nice_plug::midi::{Channel, Key};
use sushi::dsp::voice::{Voice, VoiceSettings};
use sushi::params::{FilterMode, LfoTarget, Waveform};

const SAMPLE_RATE: f32 = 48_000.0;
const DURATION: f32 = 6.0;

fn main() -> std::io::Result<()> {
    let num_samples = (SAMPLE_RATE * DURATION) as usize;
    let mut output = vec![0.0f32; num_samples];

    // A small A minor arpeggio: A2, C3, E3, A3, then back down.
    let pattern = [45u8, 48, 52, 57, 52, 48];
    let mut events = Vec::new();
    for (step, note) in pattern.iter().cycle().take(12).enumerate() {
        let start = step as f32 * 0.45;
        events.push((start, true, *note));
        events.push((start + 0.38, false, *note));
    }

    // A slightly detuned sawtooth pair with a slow filter sweep and a bit of
    // chorus on top.
    let settings = VoiceSettings {
        osc1: sushi::dsp::voice::OscSettings {
            waveform: Waveform::Saw,
            pulse_width: 0.5,
            level: 0.6,
            semitones: 0.0,
        },
        osc2: sushi::dsp::voice::OscSettings {
            waveform: Waveform::Square,
            pulse_width: 0.5,
            level: 0.25,
            semitones: -12.0,
        },
        noise_level: 0.0,
        filter_mode: FilterMode::Lowpass,
        resonance: 0.35,
        filter_env_octaves: 2.0,
        filter_keytrack: 0.3,
        filter_drive: 0.2,
        unison: 2,
        detune_cents: 12.0,
        spread: 0.6,
        glide_step: 1.0,
        lfo1_amount: 0.15,
        lfo1_target: LfoTarget::Filter,
        // A little drift, so that the voices are not perfectly in tune.
        drift_amount: 0.4,
        velocity_amount: 0.5,
        ..VoiceSettings::default()
    };

    let mut voices: Vec<Voice> = Vec::new();
    let mut next_voice_id = 0u64;
    let mut event_idx = 0;

    for (sample_idx, sample) in output.iter_mut().enumerate() {
        let time = sample_idx as f32 / SAMPLE_RATE;

        while event_idx < events.len() && events[event_idx].0 <= time {
            let (_, is_on, note) = events[event_idx];
            if is_on {
                voices.push(Voice::new(
                    next_voice_id as i32,
                    Channel::Number(0),
                    Key::Number(note),
                    next_voice_id,
                    0.9,
                    &settings,
                ));
                next_voice_id += 1;
            } else if let Some(voice) = voices
                .iter_mut()
                .find(|voice| voice.key() == Key::Number(note) && !voice.is_releasing())
            {
                voice.release();
            }
            event_idx += 1;
        }

        // Slow filter sweep to show off the state variable filter.
        let cutoff = 400.0 * (8_000.0 / 400.0f32).powf(time / DURATION);
        // A slow bipolar modulation, like the step sequencer would produce.
        let step_value = (time * 0.7).sin() * 0.5;

        let mut left = 0.0;
        let mut right = 0.0;
        for voice in voices.iter_mut() {
            let (voice_left, voice_right) =
                voice.next_sample(&settings, cutoff, 0.0, 0.0, step_value, SAMPLE_RATE);
            left += voice_left;
            right += voice_right;
        }
        voices.retain(|voice| !voice.is_finished());

        *sample = ((left + right) * 0.5).tanh();
    }

    std::fs::create_dir_all("target")?;
    write_wav("target/demo.wav", &output, SAMPLE_RATE as u32)
}

fn write_wav(path: &str, samples: &[f32], sample_rate: u32) -> std::io::Result<()> {
    use std::io::Write;

    let data_len = (samples.len() * 2) as u32;
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);

    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_len).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&1u16.to_le_bytes())?; // mono
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&(sample_rate * 2).to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_len.to_le_bytes())?;

    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        file.write_all(&value.to_le_bytes())?;
    }

    Ok(())
}
