# AGENTS.md: synteza

Work follows nonslop (`.nonslop.toml`, `/nonslop:coordinator`). Current state: `docs/STATE.md`. Plan: `docs/PLAN.md`.
The full history of this file (the Hysteresis-era session log, the dance and six-arm sections) is in git:
`git show 76453af:AGENTS.md`.

## What this is

synteza is the native Rust music visualizer engine: `hyst-render` (wgpu), `hyst-audio`, `hyst-core`, `hyst-script`,
`hyst-choreo`, `hyst-output`. It is a standalone product and kabl's video core (kabl depends on synteza, never the
reverse). The frozen TypeScript reference lives in `src/` and `tools/`; `docs/LEGACY_TS.md` is its archive.

On 2026-11-22/23 the repo split. The dance compiler and preview, and the arm's learned body and brain, moved to afekt
(`~/Programming/Github/afekt`, github stcksmsh/afekt), which depends on this repo through `SYNTEZA_DIR`: it uses
`scripts/analyze*` and the `hyst-audio` `beat_pll` example. Do not change those interfaces without saying so in the PR.

## Source of truth

- `docs/STATE.md`: where things stand. `docs/PLAN.md`: the plan (a proposal until the owner decides).
- `docs/REFERENCE.md`: reference hardware and tracks. `docs/THIRD_PARTY_LICENCES.md`: licences. `COPYING.md`: GPL-3.0-or-later
  for code, CC BY-SA 4.0 for art and docs.
- `SINTEZA_IMPLEMENTATION_PLAN.md`, `SINTEZA_OFFLINE_SSM.md`, `SINTEZA_CHOREOGRAPHY.md`: older companion specs, background
  only, not verified current design.
- `docs/*.md` about ISF, DMX, MIDI, OSC, ILDA and the patchbay describe TS-era surfaces; use them for protocol detail only.

## Rules

1. Never edit `src/`, `tools/` or `docs/LEGACY_TS.md`. Read and port freely. `scripts/` is live (the offline analysis pipeline).
2. The installation is ONE servo arm. Never build or propose several arms, ensembles or formations.
3. Never play audio on the default output or drive hardware without the owner's word.
4. Sidecar changes are additive only: a published sidecar must keep validating.
5. The owner judges visuals and motion by eye, from real frames. Never claim something looks better, and never judge by
   numbers alone; deliver captures (contact sheets, frames, clips).
6. Stay inside your slice's crate boundary; if you need to cross it, stop and flag it.
7. Commit messages are plain, with no attribution lines. Branches are `claude/<slice>` from origin/master; never merge,
   force-push or push to master.
8. Do not weaken a gate (lint, tests, CI, hook config) to get past it.
9. Build nothing from the deferred lists in the plan; flag instead.

## Commands

`cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace --exclude hyst-render`;
`cargo test -p hyst-render` (needs a GPU; CI uses lavapipe with `--test-threads=1`); `npx vitest run` for the frozen TS tests.
Scratch goes under `.claude/tmp/<job-id>/`. Big data lives in `/secondary/hyst-data`, never on the root disk. Long jobs run
under `systemd-run --user` with `MemoryMax` and `MemorySwapMax=0`.

## Operating notes

- Gap-analyse first: compare the plan against what is in `crates/` before assuming where the last session stopped.
- One coherent slice per session; its "done when" is the bar.
- Verify against real audio, not only synthetic fixtures, wherever a spec says so.
- Known debt: the `hyst-audio` cpal callback allocates and runs the extractor on the audio thread; no resampling (the WAV
  rate must equal the device rate); `hyst-render` rebuilds pipelines per pass and bloom is single-level.
- `allin1` is not wired in (its `natten` dependency needs an hour-long from-source build); the causal beat-grid fallback in
  `scripts/` is what runs. See `scripts/README.md` before retrying.

## Lessons for the renderer

- Never scale a monotonic time or phase by a jittery signal (it snaps). Integrate the rate: `phase += dt * rate(audio)`.
- A green test suite does not prove a render looks right; two legibility bugs passed all tests. Look at real frames.
- The memory field's steady-state gain is intentionally about 7x (`cur_gain = (1-decay)/(1-0.86)`), independent of decay.
  Do not "fix" it; give exposure headroom in composite instead.
- Julia palette bands off the absolute smooth escape count, interior dark; keep the anti-alias fade floored, or detail
  washes to grey.
- Round joint discs on a dense polyline read as a bead chain. Use enough thin segments instead.
- Deep zoom needs the perturbation pass (`render_julia_perturbed`); direct iteration degenerates by about zoom 1e-8.
- Earned symmetry: kaleidoscope strength rises with tension and energy, never a constant filter.
