# What afekt uses from synteza (inventory before step S0)

Read-only inventory, 2026-11-23, synteza checked against fccad14, afekt checkout `~/Programming/Github/afekt` (all
afekt paths below are relative to its root). Method: `grep -rn` over afekt `scripts/`, `AGENTS.md`, `.nonslop.toml` for
`synteza|SYNTEZA|crates/|hyst-<crate>|hyst_<crate>|cargo |ensemble_preview|body_study|body_render|score_csv|mind_state|analyze|dance_|author/`;
`grep` of synteza `crates/*/Cargo.toml` and `use hyst_*` in `crates/**/*.rs`. afekt `docs/` (about 100 prompt, brief and
log files) was not read beyond the file listing; they may name more items. Not verified: anything outside those greps.

## Dependency edges

afekt has no Rust at all: `find` finds no `Cargo.toml` and no `*.rs` in it. It depends on synteza by path, not by Cargo.

- One env var, `SYNTEZA_DIR` (default `~/Programming/Github/synteza`): `scripts/synteza_path.py:10-13` (Python),
  `scripts/synteza_dir.sh:4-6` (shell, exports `SYNTEZA`). Both only check that `crates/` exists.
- Python scripts put synteza's `scripts/` on `sys.path` (`synteza_path.add_scripts()`, `scripts/synteza_path.py:17-19`) and
  import from it.
- Shell and Python scripts run synteza binaries, either built by `cargo build` inside the synteza checkout or taken from a
  prebuilt target dir (`$CARGO_TARGET_DIR/release/...`, `/secondary/hyst-data/cargo-target-*`).
- Two Python tests read fixture files from `crates/hyst-compile/tests/fixtures/`.
- afekt mirrors some synteza Rust logic in Python (a copy, not a call): see Types to copy or share.
- No git or path Cargo dependency, no submodule, no vendored synteza code.

## Items afekt uses

| synteza item | afekt use site |
|---|---|
| `hyst-previz` example `ensemble_preview` (`crates/hyst-previz/examples/ensemble_preview.rs`), flags `--joint-csv`, `--implement`, `--ring 6,0.8`, `--score`, `--body layered --clock music` | built: `scripts/body/k5_render.sh:26`, `k5p2_render.sh:31`, `v47_regression.sh:11`, `hf_v47.sh:11`. Run from a target dir: `k5_render.sh:30`, `k5p2_render.sh:28`, `motornet2_render.sh:14`, `motornet_part3_render.sh:14`, `dialstudy_render.sh:18`, `lowpass_render.sh:14`, `retarget_render.sh:16`, `s1_render.sh:12`, `hf_fallback.sh:7`, `hf_v47.sh:12`, `emotion/d3_export.sh:9`, `body/s0_v47_song.sh:35` (`--ring 6,0.8 --score`) |
| `hyst-previz` example `body_render` (`.../examples/body_render.rs`, uses `hyst_compile::body`) | `scripts/emotion/render.py:34` (cargo build), `:46` (binary path, or `HYST_BODY_RENDER`); `emotion/surrogate.py:4` |
| `hyst-previz` example `body_study`, subcommands `ladder`, `html` | `scripts/body_study/render.sh:13,18,21`. Subcommand `regions` is named in `body_study/triplets.py:2` and `body_study/report4.py:2` (docstrings only; whether the code calls it was not traced) |
| `hyst-previz/src/ensemble.html` (template, `include_str!`ed by both examples) | patched as a copy: `scripts/body/retarget_page.py:3`, `armonly_page.py:4`; named in `body/armonly_proof.cjs:3` |
| `hyst-cli` binary `hyst`: `score-csv` (`--hz --mode --agent --body-source`), `check-csv` | `scripts/body/s0_v47_song.sh:36`, `hf_v47.sh:16`, `hf_fallback.sh:10`; `body/test_body.py:19,177` (`HYST_BIN`, `check-csv`); `body/k5p2_eval.py:47`; `body/k5_video_breaches.py:1` |
| `hyst-compile` `mind_state.rs` (sidecar field `mindState`, v1 and v2) | afekt writers: `scripts/emotion/mindstate.py:9`, `brain/brain_mindstate.py:3,4,79`; fixture `mindstate_writer.json` read by `emotion/test_mood.py:19,200` |
| `hyst-compile` `body_track.rs` (sidecar field `bodyTrack`) | afekt writer `scripts/emotion/bodytrack.py:12`; fixture `bodytrack_writer.json` read by `emotion/test_bodytrack.py:19,88` |
| `hyst-compile` `score_csv.rs` (joint CSV format, floor rule, `DISC_M`) | format copy `scripts/body/arm_sim.py:49,124,130`; `body/make_arm_v2.py:15`; `body/BODY_NOTES.md:58`; `body/hf_share.py:5` reads the CSV |
| `hyst-compile` `body.rs` (`descriptors`, `LIMITS`, `DISC_M`, generator) | Python port `scripts/body/bdesc.py:1`, parity test `body/test_bdesc.py:1`; `body/BODY_NOTES.md:4`; `emotion/render.py` via `body_render` |
| `hyst-compile` `ensemble.rs` `Rig::illustrative_five_axis()` | numbers copied into `scripts/body/make_arm.py:1`, `BODY_NOTES.md:4` |
| `hyst-compile` `body_regions.rs` (via `body_study regions`) | `scripts/emotion/test_decoder.py:176` (loads the study regions, source not traced) |
| `hyst-audio` example `beat_pll` (stays in synteza) | `scripts/brain/check_b1.py:45,49` (cargo build, `-p hyst-audio --example beat_pll`), `brain/README.md:22` |
| `scripts/analyze/` package (`analyze.structure.analyze_mix`, `analyze.wav.DecodedWav`) | `brain/prep_corpus.py:41-42,141-142`, `brain/b2.py:225-226`, `body/t0_lanes.py:58-59`, `body/metric_grids.py:28-29`, `body/t1b_cortex.py:76-77`, `body/retarget_run.py:217-218`, `body/motornet_part3.py:41-42`, `body/t0_phantom_bandlimit.py:27-28` |
| `scripts/analyze.py` (run as a program) | `brain/check_b1.py:142`, `body/s0_v47_song.sh:21` |
| `scripts/dance_memory.py` (`regularize_beats`) | import: `brain/check_b1.py:130,132`; run: `body/s0_v47_song.sh:22` |
| `scripts/dance_stems.py`, `dance_structure.py`, `dance_notes.py` | run only: `body/s0_v47_song.sh:32-34`; `emotion/features.py:5` mentions dance_stems |
| `scripts/author/record_ensemble.cjs` | `k5_render.sh:32,34`, `k5p2_render.sh:38,40,45`, `motornet2_render.sh:16`, `motornet_part3_render.sh:16`, `dialstudy_render.sh:20`, `lowpass_render.sh:17`, `retarget_render.sh:19`, `s1_render.sh:15`; Python: `body/jitter_viewing.py:149`, `body/s3_sidecar.py:193`, `body/m3f_clips.py:91` |
| `scripts/author/serve.py` | named in `brain/braindump_view.py:5` |

Not used by afekt (found no reference): `hyst-core`, `hyst-choreo`, `hyst-output`, `hyst-render`, `hyst-script`, `hyst-hw`,
`scripts/*.ts`, `src/`, `tools/`, `hyst-compile` `director*` as a Rust API (only through the binaries above).

## synteza internal graph

From `crates/*/Cargo.toml` and `use hyst_*` in sources.

| crate | depends on (Cargo) | uses in source |
|---|---|---|
| hyst-core | none of ours | |
| hyst-audio | hyst-core | `hyst_core` |
| hyst-render | hyst-core; dev: hyst-audio | `hyst_core`, `hyst_audio` (examples/tests) |
| hyst-script | hyst-core | `hyst_core` |
| hyst-output | hyst-core | `hyst_core` |
| hyst-choreo, hyst-hw | none | none |
| hyst-compile | hyst-core, serde, serde_json | no `hyst_core` use found in `crates/hyst-compile` (declared but unused; `grep -rn hyst_core crates/hyst-compile` is empty) |
| hyst-previz | dev: hyst-compile, serde_json (no `[dependencies]` section) | `hyst_compile` in `examples/*.rs` |
| hyst-cli | hyst-compile, serde_json | `hyst_compile::{body, ensemble, score_csv}` (`src/main.rs:12-14`) |

Nothing depends on hyst-previz or hyst-cli. Only hyst-previz and hyst-cli depend on hyst-compile. No engine crate
(hyst-core, hyst-audio, hyst-render, hyst-script) depends on hyst-compile, hyst-previz or hyst-cli: the move does not block
on the engine side. hyst-output and hyst-choreo do not use the movers either.

## Files to move

Facts only; `Cargo.toml`/`Cargo.lock` workspace edits are owned by another builder and not listed.

- `crates/hyst-compile/` (`src/{lib,body,body_regions,body_track,director,ensemble,mind_state,score_csv}.rs`,
  `src/director/{figures,layered,workspace}.rs`, `tests/fixtures/{bodytrack_author,bodytrack_writer,mindstate_v2_writer,mindstate_writer}.json`).
  The fixtures are `include_str!`ed by `body_track.rs:224,231` and `mind_state.rs:482,498` and read by afekt Python tests
  (`emotion/test_bodytrack.py:19`, `emotion/test_mood.py:19`); their path changes with the move.
- `crates/hyst-previz/` (`src/{lib.rs,ensemble.html}`, `examples/{ensemble_preview,body_render,body_study}.rs`, `README.md`,
  `tests/ensemble.cjs`). `ensemble.html` is `include_str!`ed at `examples/ensemble_preview.rs:526` and
  `examples/body_study.rs:397`. `tests/ensemble.cjs:2` defaults to the path `crates/hyst-previz/src/ensemble.html`.
- `crates/hyst-cli/` (`src/main.rs`, bin name `hyst`).
- Docs that name these crates and would go stale (not checked line by line): `AGENTS.md` (§ 2026-10 entries),
  `docs/B_WORKSPACE.md`, `docs/B_WORKSPACE_PROMPT.md`, `docs/DANCE_RESTART_PROMPT.md`, `docs/DANCE_REVIEW.md`,
  `.nonslop.toml:38`.
- CI: `grep -nE 'compile|previz|cli' .github/workflows/*` found no match. `.nonslop.toml` fmt/lint/test commands are
  workspace-wide.

## Cargo workspace inheritance

The three moving crates inherit from the synteza workspace: `version.workspace`, `edition.workspace`, and `serde`,
`serde_json`, `hyst-core` as `.workspace` dependencies (`crates/hyst-compile/Cargo.toml`, `hyst-previz`, `hyst-cli`).
afekt's new workspace must define these, or the three `Cargo.toml` files must be rewritten. hyst-compile's `hyst-core`
dependency is unused (no `use hyst_core` in `crates/hyst-compile`).

## Types to copy or share

afekt has no Rust, so no `hyst-core` type is used by afekt today. `hyst-compile` itself does not use `hyst-core` in source
(see graph), so the moved crates need no shared type from `hyst-core` unless the Cargo dependency is kept. If afekt gets
a Cargo workspace, `hyst-core` is either a path or git dependency or the unused entry is dropped from `hyst-compile`.

What is shared today is a format, not a type. afekt Python writes or reimplements, synteza Rust reads or defines:

- Sidecar JSON fields `mindState` (v1, v2), `bodyTrack`: Rust readers `mind_state.rs`, `body_track.rs`; Python writers
  `emotion/mindstate.py`, `brain/brain_mindstate.py`, `emotion/bodytrack.py`. After the move both sides sit in afekt.
  Guarded by the four JSON fixtures above.
- Joint CSV format: Rust `score_csv.rs`; Python `body/arm_sim.py:124-130`, `body/hf_share.py`.
- Python copies of Rust constants and logic: `body/bdesc.py` (port of `body.rs` `descriptors`), `body/make_arm.py`
  (rig numbers), `body/make_arm_v2.py:15` and `body/arm_sim.py:49` (`DISC_M`, floor rule), `body/k5_video_breaches.py`
  (`hyst check-csv` rules). Reference values for the parity test were exported once by a scratch crate outside the
  repo (`body/test_bdesc.py:1`).
- Sidecar schema: Rust `hyst-core` `Sidecar` (`crates/hyst-core/src/sidecar.rs`) stays in synteza. Whether
  `hyst-compile` parses sidecars with its own code or with that type was not traced (`hyst-compile` has no `hyst_core`
  use).

## Scripts that move, scripts that stay

afekt's use of synteza scripts (table above) is what the move has to preserve.

Depend on the three movers (cargo `-p hyst-previz` / `hyst-cli`, or their binaries), so they would travel with them or
change their path:

- `scripts/author/retune.sh:27` (`cargo build -p hyst-previz --example ensemble_preview`), `author/attach.py:62`
  (`cargo run ... ensemble_preview`). Files that only name the movers: `author/author_core.cjs:115` (comment),
  `author/README.md` (lines 41, 52, 73, 81, 83, 91), `author/test_attach.py:11` (reads `crates/hyst-compile/tests/fixtures/bodytrack_author.json`).
  `author/wo7_renders.sh` only exports `CARGO_TARGET_DIR` (line 9) and `author/tuning_check.sh` has no cargo or
  ensemble_preview reference: neither depends on the movers.
- `scripts/author/record_ensemble.cjs` records an `ensemble_preview` page; it has no cargo call, but afekt uses it
  with pages built by the movers (14 call sites above).

Do not depend on any Rust crate (imports checked in `scripts/dance_*.py`, `scripts/analyze.py`, `scripts/analyze/*.py`):

- `scripts/analyze.py` and package `scripts/analyze/` (`brain.py`, `demucs.py`, `dsp.py`, `fftjs.py`, `jsjson.py`,
  `structure.py`, `wav.py`): numpy only, plus a `demucs` subprocess (`analyze/demucs.py`). Its equivalence harness
  `scripts/analyze/equiv/run_equiv.sh:14` runs the frozen TS `scripts/analyze.ts` (needs `node_modules`, `tsx`).
- `scripts/dance_memory.py`, `dance_stems.py`, `dance_structure.py`, `dance_notes.py` (numpy, scipy, soundfile);
  `dance_structure.py:16` imports `dance_memory`. Their tests `test_dance_*.py` sit beside them.
- `scripts/instant_crush.acceptance.json` (named in `AGENTS.md`; not referenced by afekt in the greps).

Stay with the engine (nothing found that afekt needs): `scripts/*.ts` (`analyze.ts`, `ssm.ts`, `repetition.ts`,
`schema4.ts`, `structure.ts`, `demucs.ts`, `wav.ts`, `render-video.ts`, relays), `src/`, `tools/`.

Not decided by the facts: whether `analyze` and `dance_*` move with the crates or stay in synteza. afekt currently
reaches them through `SYNTEZA_DIR`; `hyst-audio`'s `beat_pll` (used by `check_b1.py`) stays in synteza either way.

## Blockers and open questions

1. No engine crate depends on the movers (see graph); no blocker found on the synteza side.
2. afekt has no Cargo workspace. After the move afekt needs one, and its scripts still carry `cargo build ... -p hyst-previz`
   with `cwd` = synteza (`emotion/render.py:34-35`, `brain/check_b1.py:49` for hyst-audio, shells `cd "$SYNTEZA" && cargo build`
   at `k5_render.sh:26`, `k5p2_render.sh:31`, `v47_regression.sh:11`, `hf_v47.sh:11`). Every `cd "$SYNTEZA" && cargo build -p
   hyst-previz|hyst-cli` site breaks on the move. `hyst-audio`'s `beat_pll` build (`check_b1.py:49`) stays valid. `scripts/body_study/render.sh:13` hard-codes
   `$SYNTEZA/target/release/examples/body_study`; it runs a prebuilt binary and builds nothing, so it breaks when that
   path stops existing.
3. `scripts/author/record_ensemble.cjs:8` requires `puppeteer-core` from the old path
   `/secondary/Programming/Github/Hysteresis/node_modules`; afekt `scripts/body_study/test_answer.cjs:5` requires
   `/home/stcksmsh/Programming/Github/Hysteresis/node_modules`. Both point at the pre-split checkout (exists or not: not
   checked). Unrelated to the move itself, but both break if that checkout goes.
4. Hard-coded prebuilt target dirs outside any checkout: `/secondary/hyst-data/cargo-target-wo12/release` (`s0_v47_song.sh:12`,
   `k5p2_eval.py:47`), `/secondary/hyst-env/cargo-target-body/debug/hyst` (`test_body.py:19`),
   `/secondary/hyst-data/cargo-target-dial` (`body/jitter_viewing.py:23`), `cargo-target-wo12` (`body/m3f_clips.py:27`,
   `body/s3_sidecar.py:37`), `cargo-target-tuning`, `cargo-target-wo7`. These keep working only while
   a binary is built into them from the new location.
5. `body/s0_v47_song.sh:17` does `cd "$SYNTEZA"` and runs `scripts/analyze.py` and `scripts/dance_*.py` by relative path:
   breaks if those scripts leave synteza without the `SYNTEZA` variable being repointed (open: where they end up).
6. `ensemble_preview` v47 regression path (`--ring 6,0.8`, mode Off) is the byte-identical reference for the hexagon
   exports (`AGENTS.md`, `scripts/body/v47_regression.sh`, `docs/B_WORKSPACE.md`); the flags are used by afekt
   (`s0_v47_song.sh:35`, `hf_v47.sh`) and must survive the move unchanged.
7. `scripts/author/retune.sh` and `attach.py` depend on `hyst-previz`; `test_attach.py` and `author_core.cjs`/`README.md`
   name hyst-compile paths. Not decided whether they move (see above).
8. The interface list the project asks to keep stable (`.nonslop.toml:38`) matches what was found: `hyst-compile` `body*.rs`,
   `mind_state.rs`, `score_csv.rs`; `hyst-previz` `body_render`/`body_study`; `hyst-cli`. Beyond that list afekt also
   depends on `ensemble_preview` and its flags, `ensemble.html`, `body_track.rs`, `ensemble.rs` `Rig` numbers,
   `scripts/author/record_ensemble.cjs` and the four fixtures.
9. afekt `docs/` was not searched; further users of synteza paths may exist there (docs are prose, not code).
