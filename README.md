# SUSHI

![SUSHI](docs/panel.png)

A compact, mouse-first polyphonic synthesizer.
**CLAP · VST3 · Standalone**

[![Release](https://img.shields.io/github/v/release/77-223255/sushi?label=release)](https://github.com/77-223255/sushi/releases/tag/v0.0.1)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[Download 0.0.1](https://github.com/77-223255/sushi/releases/tag/v0.0.1)

## Sound

- Two band-limited oscillators, sub oscillator, hard sync, phase FM, unison
- 12 / 24 dB TPT filter with drive
- Dual ADSR envelopes, dual LFOs, analog drift
- 8-slot modulation matrix
- Arpeggiator with swing, ratchet, gate, length and latch
- Chorus, delay, reverb
- 14 factory presets, 95 parameters

## Interface

- Light SH-101-inspired faceplate, all vertical faders
- Coarse lane plus a separate fine lane on every fader
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
