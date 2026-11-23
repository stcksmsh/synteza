# State 2026-11-23

## Goal
synteza (synteзa): native Rust music visualizer, standalone product and kabl's video core (kabl depends on
synteza, never the reverse). Plus the single arm's dance compiler/preview (to move to afekt) and the frozen TS
reference (src/, tools/). Split from Hysteresis on 2026-11-22.

## Done
- Split: history of the old `body` branch minus afekt's paths. cargo test --workspace --exclude hyst-render green (228 tests).

## In flight
- Nothing. No builders running.

## Proposed plan (not owner-reviewed)
- `docs/PLAN.md` on branch `ccr-1adff24f-zzq23d` (from the planning session; evidence in docs/plan-evidence/ there).
  Read: `git fetch origin ccr-1adff24f-zzq23d` then `git show FETCH_HEAD:docs/PLAN.md`. §8.3 = owner decisions,
  §9 = first-30-days work orders WO-1..WO-14. Do not act on §8 decisions or merge the branch without the owner.

## Next            (ordered, pending owner OK)
1. WO-4: fix two manual_is_multiple_of clippy errors in crates/hyst-render/src/passes/julia.rs, delete hyst-hw,
   add Rust CI (fmt, clippy -D warnings, tests, render tests on lavapipe). Lint gate is red on master until fixed.
2. WO-2 (parallel, read-only): inventory of everything afekt uses from synteza; feeds WO-3 (S0 move of
   hyst-compile/hyst-previz/hyst-cli to afekt, one coordinated change in both repos).
3. WO-9 (test competitor "Infinite"), WO-10 (read third-party licence texts): need no decisions.
4. WO-8: README/AGENTS.md trim after WO-3.

## Waiting on owner
- OK to dispatch WO-4 + WO-2.
- Licence (PLAN §8.3 decision 1; plan suggests MIT OR Apache-2.0 for engine crates). Blocks WO-11, S1.
- Other §8.3 decisions: business model, reference laptop + tracks, 1.0 platforms, Mandelbulb verdict, library
  shape, fate of hyst-choreo/hyst-script/hyst-output, TS Pages demo repair or retire.
- Merge or not: plan branch `ccr-1adff24f-zzq23d`.
- PR #15 (keys-decide, version 27, draft from 2026-10-04, pre-split): close or keep.
- Dance version 47 verdict (docs/DANCE_HANDOFF.md, last entries) — may be moot once dance moves to afekt.

## Hard-won facts
- hyst-render tests need a GPU; run them (test_render) only when a slice touches it.
- Frozen src/ and tools/ are read-only. scripts/ is live.
- afekt uses hyst-compile body*.rs, mind_state.rs, score_csv.rs, hyst-previz body_render/body_study, hyst-cli,
  scripts/analyze, author, dance_*: keep their interfaces or say so in the PR.
- Commit messages are plain, no attribution lines.
