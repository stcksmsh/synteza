# State 2026-11-22

## Goal
synteza (synteзa): native Rust music visualizer and the single arm's dance compiler/preview (crates/hyst-*),
plus the frozen TS reference (src/, tools/). Split from Hysteresis on 2026-11-22; the arm's learned body and
brain moved to afekt (~/Programming/Github/afekt), which depends on this repo.

## Done
- Split: history of the old `body` branch minus afekt's paths. cargo test --workspace --exclude hyst-render green (228 tests).

## In flight
- Nothing.

## Next            (ordered)
1. Clippy gate is red on baseline: two manual_is_multiple_of errors in crates/hyst-render/src/passes/julia.rs. Fix first,
   or every slice fails the lint gate.
2. AGENTS.md is the Hysteresis-era log; trim to what synteza needs (keep history in git).
3. Dance version 47 awaits the owner's verdict (docs/DANCE_HANDOFF.md, last entries).

## Waiting on owner
- Nothing.

## Hard-won facts
- hyst-render tests need a GPU; run them (test_render) only when a slice touches it.
- Frozen src/ and tools/ are read-only. scripts/ is live.
- afekt uses hyst-compile body*.rs, mind_state.rs, score_csv.rs, hyst-previz body_render/body_study, hyst-cli,
  scripts/analyze, author, dance_*: keep their interfaces or say so in the PR.
- Commit messages are plain, no attribution lines.
