# synteza

A native Rust music visualizer engine. The image has memory: a feedback field accumulates and fades the last seconds of
the music, so what you see depends on recent history and on musical structure (builds, drops, breaks), not only on
instantaneous amplitude. synteza is meant to be both a standalone tool for artists and the video core of the kabl
project (kabl depends on synteza, never the reverse).

## Status

Honest summary of what runs today:

- Offline renders. `hyst-render` draws a Julia-set substrate through a memory field, bloom, composite and an
  oscilloscope-style beam on the GPU (wgpu), driven by features that `hyst-audio` extracts from a WAV file. A separate
  Mandelbulb pass renders standalone. Both are exercised by examples that write frames and, if `ffmpeg` is on `PATH`,
  an mp4 under the crate's `test-output/` directory.
- Audio playback and live feature extraction (`hyst-audio` example `play`) on the default output device.
- CI (`.github/workflows/rust.yml`): fmt, clippy, tests, and the render tests on a software Vulkan driver (lavapipe).

There is **no live native visualizer window yet**: nothing opens a window and renders in real time while music plays.
See `docs/PLAN.md` section 1 for where the project stands and where it is going. The frozen TypeScript browser
visualizer in `src/` and `tools/` is reference material only.

## Build and test

Needs a Rust toolchain (CI uses 1.93.0) and, on Linux, ALSA development headers (`libasound2-dev`) and `pkg-config`.
The render tests need a Vulkan adapter.

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --exclude hyst-render    # everything that needs no GPU
cargo test -p hyst-render                       # needs a GPU or lavapipe
npx vitest run                                  # the frozen TypeScript tests (after npm install)
```

## Run the examples

```sh
# offline Julia render of a WAV file (writes under crates/hyst-render/test-output/)
cargo run --release -p hyst-render --example audio_driven_render -- path/to/track.wav
# offline Mandelbulb render
cargo run --release -p hyst-render --example mandelbulb_render -- path/to/track.wav
# plays the file on the default output device and prints live features (makes sound)
cargo run -p hyst-audio --example play -- path/to/track.wav [latency_ms]
```

## Crates

- `hyst-core`: signal bus types, sidecar schema, project loading, the `VizOutput` trait, errors.
- `hyst-audio`: playback, audio clock, FFT/bands/chroma features, beat tracking (PLL); examples `play` and `beat_pll`.
- `hyst-render`: wgpu renderer, `.hyst` shader format parser and ISF translation, Julia, Mandelbulb, memory field,
  bloom, composite and beam passes.
- `hyst-script`: Lua scripting runtime (`mlua`) with an instruction budget and fault isolation.
- `hyst-choreo`: choreography primitives (curves, moves, formations); pure math, no hardware.
- `hyst-output`: physical-output plumbing built on `VizOutput` (dense element fields with diff-only transmission and failure handling).

`scripts/` holds the Node/TypeScript offline analysis pipeline (`npm run analyze -- master.wav`), which emits the
sidecar JSON. The dance compiler and the arm's learned body and brain moved to
[afekt](https://github.com/stcksmsh/afekt), which depends on this repo.

## Documents

- `docs/PLAN.md`: the project plan (a proposal, not a commitment).
- `docs/STATE.md`: current state, what is in flight and what is next.
- `docs/REFERENCE.md`: reference hardware and tracks.
- `docs/THIRD_PARTY_LICENCES.md`: third-party licences.
- `AGENTS.md`: rules for agents working in this repo.
- `docs/LEGACY_TS.md`: the frozen TypeScript system, for porting reference.

## Licence

Code (Rust crates, scripts, shaders, hyst files): GPL-3.0-or-later, see `LICENSE`. Art and documentation (renders,
images, video, written docs): CC BY-SA 4.0, see `LICENSE-CC-BY-SA-4.0.txt`. Details in `COPYING.md`. Third-party
material keeps its own licence.
