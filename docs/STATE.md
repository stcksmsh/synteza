# State 2026-11-23

## Goal
synteza (synteзa): native Rust music visualizer, standalone product and kabl's video core (kabl depends on
synteza, never the reverse). Plus the single arm's dance compiler/preview (to move to afekt) and the frozen TS
reference (src/, tools/). Split from Hysteresis on 2026-11-22.

## Done
- Split from Hysteresis (228 CPU tests green).
- #17 STATE.md + julia.rs clippy fix. #18 docs/AFEKT_MOVE.md (WO-2 inventory, reviewed minor, fixed).
- #20 .github/workflows/rust.yml: fmt, clippy, CPU tests, render tests on lavapipe (toolchain 1.93.0).
  Added by the coordinator at the owner's instruction; the slopstop hook flags agent edits to it.
- #19 WO-4: hyst-hw deleted, cargo fmt of two files, julia steer_toward_boundary fix (re-acquire on low
  escape-count spread instead of low best count; fixes driver_keeps_the_boundary_framed_through_a_deep_dive).

## In flight
- Golden digest CI mismatch: hyst-compile director::figures::tests::golden_digests_of_compile_ensemble,
  "ring"/"mirrored" digests differ on ubuntu-latest, pass locally. Single-arm digests match. Branch
  claude/golden-digest-ci, builder (opus). If the cause is platform libm, the owner decides (constants change).

## Next            (ordered)
1. Merge the golden-digest fix so master CI is fully green.
2. WO-3: the S0 move of hyst-compile/hyst-previz/hyst-cli to afekt (input: docs/AFEKT_MOVE.md). Touches two
   repos; afekt build scripts hard-code synteza paths (see its blockers).
3. WO-9 (test competitor "Infinite"), WO-10 (third-party licence texts): no decisions needed.
4. WO-8: README/AGENTS.md trim after WO-3.
5. Follow-up (reviewer minor on #19): the new julia re-acquire also fires on a smooth deep-zoom boundary
   (all counts ~100-115), causing extra target moves. Untested. Judge by eye or add a test.

## Waiting on owner
- Licence (PLAN §8.3 decision 1; plan suggests MIT OR Apache-2.0 for engine crates). Blocks WO-11, S1.
- Other §8.3 decisions: business model, reference laptop + tracks, 1.0 platforms, Mandelbulb verdict, library
  shape, fate of hyst-choreo/hyst-script/hyst-output, TS Pages demo repair or retire.
- Merge or not: plan branch `ccr-1adff24f-zzq23d` (docs/PLAN.md, proposal, read with git show).
- PR #15 (keys-decide, version 27, pre-split): close or keep.

## Hard-won facts
- The owner allows the coordinator to `gh pr merge` at any time (2026-11-23).
- The slopstop hook denies agents writing .github/workflows; only touch it on the owner's explicit word.
- `gh pr edit` fails (GraphQL Projects classic); use `gh api -X PATCH repos/stcksmsh/synteza/pulls/<n>`.
- CI render tests on lavapipe work; one intermittent SIGSEGV seen in 2 runs. If it recurs, try
  `-- --test-threads=1` (workflow edit, owner's word).
- afekt has no Rust code; it uses synteza through SYNTEZA_DIR, built binaries, scripts, fixture JSONs.
- hyst-render tests need a GPU locally; run them (test_render) only when a slice touches it.
- Frozen src/ and tools/ are read-only. scripts/ is live. ONE arm; ensemble code is the v47 reference only.
- Commit messages are plain, no attribution lines.
