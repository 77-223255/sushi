# SUSHI

![SUSHI](docs/panel.png)

A compact, mouse-first polyphonic synthesizer.
**CLAP · VST3 · Standalone**

[![Release](https://img.shields.io/github/v/release/77-223255/sushi?label=release)](https://github.com/77-223255/sushi/releases/tag/v0.0.1)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[Download 0.0.1](https://github.com/77-223255/sushi/releases/tag/v0.0.1)

## Sound

- Two band-limited oscillators, sub oscillator, hard sync, phase FM, unison
- Filter slopes 12 dB / 24 dB OLD (original ladder) / 24 dB (new ZDF ladder
  with working resonance), with drive
- ADAA anti-aliased saturation and drive
- Dual ADSR envelopes, dual LFOs, analog drift
- 8-slot modulation matrix
- Arpeggiator with swing, ratchet, gate, length and latch
- Chorus, ping-pong delay, diffused and modulated FDN reverb
- Modern: precise, high-performance digital path, free to evolve (new ZDF
  24 dB ladder, diffused FDN reverb, new mod sources/destinations)
- Legacy: old-school rough, analog-warm path (Freeverb, cascaded SVF, raw tanh)
  with deterministic per-voice component tolerances, slower drift,
  even-harmonic warmth and a fixed HF loss; old projects keep their patch
  behavior and open in Legacy
- Patch JSON and parameter IDs are unchanged; the same patch works in both
  flavors
- 29 factory presets, 95 patch parameters

The last three factory presets showcase the new engine: **ZDF ACID** uses the
zero-delay-feedback 24 dB ladder, **WHEEL MOTION** routes the mod wheel to the
cutoff without adding pitch vibrato, and **RANDOM PAD** uses the deterministic
per-note Random source for pan and cutoff.

## Interface

- Modern: light SH-101-inspired faceplate; Legacy: warm vintage palette, all vertical faders
- Compact MOD 2.0 table: one header and eight source/dest/amount routes with
  bipolar bars, MOD ENV and AMP ENV kept visible; Mod Wheel and Random sources,
  plus Osc2 Pitch/Fine, Sub, Noise, Detune and Filter Env destinations
- Coarse lane plus a separate fine lane on every fader
- Click the SUSHI wordmark to switch Modern / Legacy
- Full-page JSON patch browser
- Right-click value entry, mouse wheel, double-click reset
- 960 × 600 design, continuous scaling, opens at 1280 × 800
- Built for the mouse; no live MIDI performance required

## Screenshots

### Modulation matrix

![Modulation matrix](docs/mod.png)

### Patch browser

![Patch browser](docs/patches.png)

## Install

Download the release archive and unpack it:

```bash
tar -xzf sushi-0.0.1-linux-x86_64.tar.gz
cp -r sushi.clap ~/.clap/
cp -r sushi.vst3 ~/.vst3/
```

The standalone binary is `sushi`.

## Build

Requires Rust 1.88+ on Linux.

```bash
cargo build --release
cargo nice-plug bundle sushi --release
```

Bundles are written to `target/bundled/`.

## License

MIT. See [LICENSE](LICENSE).
