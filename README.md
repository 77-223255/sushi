# SUSHI

![SUSHI](docs/panel.png)

A compact, mouse-first polyphonic synthesizer.
**CLAP · VST3 · Standalone**

[![Release](https://img.shields.io/github/v/release/77-223255/sushi?label=release)](https://github.com/77-223255/sushi/releases/tag/v0.0.1)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[Download 0.0.1](https://github.com/77-223255/sushi/releases/tag/v0.0.1)

## Sound

- Two band-limited oscillators, sub oscillator, hard sync, phase FM, unison
- Filter slopes 12 dB / 24 dB, both through the Legacy cascaded SVF, with
  drive and a bypassable HPF; the `24 dB OLD` / `24 dB` labels remain for patch
  compatibility
- Dual ADSR envelopes, dual LFOs with host tempo sync and retrigger, analog drift
- 8-slot modulation matrix; the LFOs are sources there, so all LFO routing
  lives in MOD (the old direct LFO target/amount is kept only for old patches)
- Arpeggiator with swing, ratchet, gate, length, latch, chord, strum and
  probability
- Drive, phaser, chorus, ping-pong delay, Freeverb-style reverb and a
  three-band EQ, with per-effect bypass, chain rotation and global wet
- Legacy engine only: old-school rough, analog-warm path (Freeverb, cascaded
  SVF, raw tanh) with deterministic per-voice component tolerances, slower
  drift, even-harmonic warmth and a fixed HF loss; old projects keep their
  patch behavior
- Patch JSON and parameter IDs are unchanged
- 29 factory presets, all played through the Legacy engine, 126 patch parameters

The LFO sync divisions are 1/4, 1/8, 1/16 and 1/32. A quarter note at 120 BPM
runs at 2 Hz, and rate changes follow the host tempo with a short glide. The
retrigger switch restarts the shared LFO on the first note after all voices
have gone silent, so a held chord or an arpeggiated phrase keeps one stable
phase instead of jittering between its notes.

The last three factory presets are the showcase presets: **ZDF ACID** is the
acid lead (the name is retained from the earlier engine), **WHEEL MOTION**
routes the mod wheel to the cutoff without adding pitch vibrato, and **RANDOM
PAD** uses the deterministic per-note Random source for pan and cutoff.

## Interface

- Warm vintage faceplate with all vertical faders
- Compact MOD 2.0 table: one header and eight source/dest/amount routes with
  bipolar bars, MOD ENV and AMP ENV kept visible; Mod Wheel and Random sources,
  plus Osc2 Pitch/Fine, Sub, Noise, Detune and Filter Env destinations
- Live MOD feedback: LFO, macro and mod-wheel routes show their current value
  on the amount bar, and the Target inspector shows the selected route’s
  current value, modulation range and live contribution
- Coarse lane plus a separate fine lane on every fader
- LFO 1/2 editors live on the MOD page below the matrix: wave, sync,
  retrigger, a free-rate fader and the synced division; their routing stays in
  the MOD table
- The main page keeps ARP / VOICE and MACROS / VCA in two wide lower bands;
  VCA adds Width and Pan, envelopes add Curve, and VCO2 adds Ring and RESET
- MOD and FX page tabs; the FX tab opens a full-panel two-row rack of drive,
  phaser, chorus, delay, reverb and EQ, with chain order, per-effect bypass,
  global wet and the signal order printed above it
- Click the preset LCD to open the full-page JSON patch browser
- Undo/Redo in the footer, with Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z; a whole fader
  drag is one step
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
