# SUSHI

A compact, mouse-first polyphonic synthesizer.
**CLAP · VST3 · Standalone**

[![Release](https://img.shields.io/github/v/release/77-223255/sushi?label=release)](https://github.com/77-223255/sushi/releases/tag/v0.0.1)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[Download 0.0.1](https://github.com/77-223255/sushi/releases/tag/v0.0.1)

## Sound

Sushi runs the original **Legacy** engine only. The editor has no engine
switch; the `legacy` host parameter is kept for state compatibility, and old
projects keep their patch behavior.

- Two PolyBLEP band-limited oscillators (sine, triangle, saw, square, pulse),
  a square sub oscillator one octave below VCO 1, hard sync, phase FM, ring
  modulation, optional phase reset at note-on, and unison up to 4 voices with
  detune and stereo spread
- Up to 16 voices in poly, mono or legato mode, with glide, velocity, drift and
  deterministic per-voice component tolerances
- Four-mode (lowpass, highpass, bandpass, notch) cascaded state-variable filter
  at 12 or 24 dB/oct, with cutoff, resonance, envelope amount, keytrack and
  drive; the `24 dB OLD` and `24 dB` labels are aliases kept for patch
  compatibility
- Global non-resonant 12 dB/oct highpass on the voice sum; its 20 Hz default is
  an exact bypass
- Two ADSR envelopes with curve shaping, and two LFOs with free or host-synced
  rates, phase, curve and retrigger
- 8-slot modulation matrix with bipolar amounts. Sources are LFO 1, LFO 2, mod
  envelope, amp envelope, velocity, key, macros 1–3, mod wheel and a
  deterministic per-note random source. Destinations include pitch, pulse
  width, cutoff, resonance, drive, pan, volume, both oscillators' pitch and
  fine tuning, sub level, noise level, detune, filter envelope, filter drive,
  filter keytrack and ring mod. LFO routing lives in the matrix; the old direct
  LFO target/amount is only used when a patch has no matrix route from that LFO
- Tempo-synced arpeggiator with Up, Down, Up-Down, Random and As Played
  patterns; 1/4–1/32 rate, 1–3 octaves, swing, gate, ratchet, length, latch,
  chord (octave, fifth, minor, major), 0–80 ms strum and probability
- Eight-stage stereo FX chain in any of 8! = 40,320 orders: Drive, Phaser,
  Chorus, Delay, Reverb, EQ, Compressor and Tremolo. The default order is
  Drive → Phaser → Chorus → Delay → Reverb → EQ → Compressor → Tremolo.
  Movable chain tiles, six named chain presets, per-stage bypass through an
  8-bit mask and a global wet mix are included. Drive Tone, Phaser Feedback,
  Chorus Width, Delay Damping and Reverb Tone are part of the stage controls;
  the compressor is stereo-linked and the tremolo can run as auto-pan
- 140 patch parameters and 29 factory presets; patches are JSON and parameter
  IDs are stable

The LFO and arpeggiator divisions are 1/4, 1/8, 1/16 and 1/32. At 120 BPM a
quarter note runs at 2 Hz, and a synced LFO glides briefly when the tempo or
division changes. Retrigger restarts a shared LFO on the first note after all
voices are silent, so a held chord or arpeggio keeps one phase.

## Interface

- Warm Legacy faceplate with flat 1 px controls, the embedded Silkscreen pixel
  font and vertical faders that have a coarse lane plus a separate fine lane
- The main page is four compact bands:
  - `VCO 1 | VCO 2 | PULSE WIDTH | SOURCE MIXER`
  - `VCF | MOD ENV | AMP ENV`
  - `ARP | VOICE`
  - `MACROS | VCA`

  The VCF contains Cut, HPF, Res, Env, Key and Drv; VCO 2 adds Sync, Reset, FM
  and Ring; both envelopes have A/D/S/R/Curve; the ARP has chord, strum and
  probability; the VCA has Volume, Tone, Width and Pan
- The MOD page puts the routing table and Target inspector on the left and a
  vertical column of LFO 1, LFO 2, MOD ENV, AMP ENV and MACROS on the right.
  Selecting a route updates the inspector, which shows the target's current
  value, modulation range, live contribution and a range bar. Per-voice sources
  report `per-voice` instead of a global value. LFO routing stays in the table
- The FX page has a chain column on the left with the eight movable tiles
  (drag or use the arrows), per-stage power buttons, GLOBAL WET and CHAIN
  PRESET; the fixed 2 × 4 module grid on the right holds the stage controls.
  Named chain presets are STANDARD, EQ FIRST, DRIVE LAST, AMBIENT, WIDE ECHO and
  GLUE; any other order is shown as CUSTOM
- The header contains the preset LCD (opens the full-page patch browser),
  previous/next preset, MUTATE, the MOD and FX tabs, the voice meter, the
  oscilloscope and the ACT/CLIP lights. Clicking the active MOD or FX tab
  returns to the main page
- The patch browser has a folder field, search, user and factory patch lists, a
  JSON preview, and a name field with LOAD and SAVE
- UNDO and REDO sit in the footer, with Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z or Ctrl+Y;
  a whole fader drag is one undo step. Right-click a control to type a value,
  scroll to change it, Shift+scroll for fine steps, and double-click to reset
- 960 × 600 design with continuous scaling; the editor opens at 1280 × 800 and
  shows a minimum-size hint below 960 × 600

## Install

Download the 0.0.1 release archive and unpack it:

```bash
tar -xzf sushi-0.0.1-linux-x86_64.tar.gz
cp -r sushi.clap ~/.clap/
cp -r sushi.vst3 ~/.vst3/
```

The standalone binary is `sushi`.

## Build

Requires Rust 1.88+ on Linux. The plugin bundles also need
`cargo-nice-plug`.

```bash
cargo test
cargo build --release
cargo nice-plug bundle sushi --release
```

The standalone binary is written to `target/release/sushi`; bundles are written
to `target/bundled/`.

## License

MIT. See [LICENSE](LICENSE).
