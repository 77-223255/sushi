# SUSHI

A polyphonic software synthesizer. **CLAP / Linux**.

## Install

```bash
cargo build --release
install -m 755 target/release/libsushi.so ~/.clap/sushi.clap
```

Or `cargo nice-plug bundle sushi --release`, which writes `target/bundled/`.
Then `Options → Preferences → Plug-ins → CLAP → Re-scan` in REAPER.

## Interface

A black plate, one orange accent and green for anything that reads as data:
flat 1 px outlines everywhere, no gradients or shadows, 120 fps.

- **Header**: preset LCD (click it for the patch browser), `<` `>`, `MUTATE`,
  the `MOD` / `FX` tabs, the level meter (a row of cells that turn green,
  orange and red with the level) and the oscilloscope (the trace only, no grid)
- **Main page**, four bands: `VCO 1 | VCO 2 | PULSE WIDTH | SOURCE MIXER`,
  `VCF | MOD ENV | AMP ENV`, `ARP | VOICE`, `MACROS | VCA`
- **MOD page**: the routing table fills the left column — under the controls of
  every route runs the recorded history of its source (what the modulation
  really did, block by block, live) and, on the right, `NOW`: the centre value
  of the destination in green and the value it reads right now in orange, both
  in the same units — while LFO 1 / LFO 2 / MOD ENV / AMP ENV / MACROS sit
  down the right
- **FX page**: eight draggable chain tiles (any of the 8! orders) plus
  `GLOBAL WET` and `CHAIN PRESET` on the left, a fixed 2 × 4 module grid on the
  right
- **Bottom row**: the info panel on the left (the readout of the last
  parameter on one line with its value, and the number of voices) and the keys
  on the right — click or drag to play, velocity follows how high on the key
  you click

| Input | Effect |
|---|---|
| Drag | change a value |
| Wheel / Shift+wheel | change / fine-tune |
| Right-click a control | type a value |
| Double-click | reset |
| **Right-click the background** | menu: `Zoom` (50–300 %, the parent panel stays open) and `Shortcuts` (every key of the panel) |

The panel always snaps to one of the zoom levels — it never stretches with the
window. Picking a level also asks the host to resize the plugin window to that
level, so the level is reached immediately instead of only after a manual
resize. The panel opens at 150 %.

## Sound

The Legacy engine, and only that one.

- Two PolyBLEP band-limited oscillators (sine, triangle, saw, square, pulse), a
  square sub one octave down, hard sync, phase FM, ring modulation and up to
  four unison voices
- 16 voices, poly / mono / legato, with glide, velocity, drift and deterministic
  per-voice component tolerances
- 12 / 24 dB cascaded state-variable filter (lowpass, highpass, bandpass,
  notch) with cutoff, resonance, envelope amount, keytrack and drive, plus a
  global 12 dB highpass on the voice sum
- Two ADSR envelopes with curve shaping, two LFOs (free or host-synced) and an
  8-slot bipolar modulation matrix
- Arpeggiator: Up, Down, Up-Down, Random, As Played; 1/4–1/32, 1–3 octaves,
  swing, gate, ratchet, length, latch, chord, strum and probability
- Eight-stage stereo FX chain in **any order** (Drive, Phaser, Chorus, Delay,
  Reverb, EQ, Compressor, Tremolo), six chain presets and a global wet mix
- 140 parameters, 11 factory presets; patches are JSON and parameter IDs are
  stable

## Patches

User patches do not live in this repository. They live in the REAPER project
repository:

    ~/Music/all_my_reaper/presets/Sushi/patches

Point the patch browser at that folder, or set `patch_dir` in
`~/.config/sushi/config.json`. Only content that has to ship with the plugin
belongs in `factory/`.

## Develop

```bash
cargo test          # 327 tests: DSP, geometry, no overlapping text
cargo build --release
```

## License

MIT. The bundled Quicksand and JetBrains Mono fonts are SIL OFL 1.1.

## Looking at the panel without a host

The look of the panel can be checked offline: the tests can rasterize one frame
of the real `egui` output — including the glyphs of the font atlas — into a PNG.

```sh
SUSHI_PREVIEW=/tmp/sushi.png cargo test --release render_preview
SUSHI_PAGE=mod SUSHI_PREVIEW=/tmp/mod.png cargo test --release render_preview
SUSHI_MENU=zoom SUSHI_PREVIEW=/tmp/menu.png cargo test --release render_preview
SUSHI_HOVER=40,150 SUSHI_PREVIEW=/tmp/hover.png cargo test --release render_preview
```
