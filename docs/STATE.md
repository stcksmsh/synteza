# State 2026-11-23

## Goal
synteza (synteзa): native Rust music visualizer, standalone product and kabl's video core (kabl depends on
synteza, never the reverse). The dance compiler/preview is moving to afekt. Frozen TS reference in src/, tools/.

## Done
- #17 STATE + clippy fix. #18 docs/AFEKT_MOVE.md. #19 hyst-hw deleted, fmt, julia steering fix.
- #20 .github/workflows/rust.yml (fmt, clippy, CPU tests, render tests on lavapipe; owner OK'd coordinator edit).
- #22 docs/REFERENCE.md: reference laptop (i7-13700H, Iris Xe), owner's tracks ~/Music/Singles/Published.
- #23 hyst-compile on pure-Rust libm; golden digests re-recorded (owner's decision), machine-independent.
- PR #15 closed (owner).

## In flight
- WO-3 (S0 move): hyst-compile/hyst-previz/hyst-cli + scripts/dance_*, scripts/author/ → afekt. Two PRs:
  synteza claude/s0-move-out, afekt claude/s0-move-in; merge together, afekt first. Builder (opus).
  Coordinator assumptions: analyze.py, hyst-choreo, hyst-output stay in synteza; copy files (no subtree).
- WO-10 third-party licence texts → docs/THIRD_PARTY_LICENCES.md, branch claude/wo10-licences. Builder (sonnet).

## Next            (ordered)
1. Review and merge WO-3 (both repos) and WO-10.
2. WO-8: README/AGENTS.md trim after WO-3.
3. WO-6: contact-sheet + blind A/B tool, "before" renders (scenes will be rethought; Mandelbulb dive dropped).
4. WO-9: hands-on test of competitor "Infinite".
5. Follow-up (#19 review minor): julia re-acquire also fires on smooth deep-zoom boundary; untested.

## Waiting on owner
- Licence. Owner wants: nobody takes it closed and sells it without credit. Proposed: GPL-3.0-or-later for code
  and hysts, CC BY-SA 4.0 for art/docs. Awaiting confirmation. Blocks WO-11, S1.
- Merge plan branch `ccr-1adff24f-zzq23d` (docs/PLAN.md)? Owner was given §1, §4.2, §8.1, §9 to read.
- OK to add `-- --test-threads=1` to the render job (lavapipe SIGSEGV seen 3 times)?
- Still open from PLAN §8.3: 1.0 platforms, library shape (owner: rethink all scenes), fate of
  hyst-choreo/hyst-script/hyst-output, TS Pages demo, business model.

## Hard-won facts
- The owner allows the coordinator to `gh pr merge` at any time (2026-11-23).
- The slopstop hook denies agents writing .github/workflows; edit only on the owner's explicit word.
- `gh pr edit` fails (GraphQL Projects classic); use `gh api -X PATCH repos/stcksmsh/synteza/pulls/<n>`.
- glibc hypot differs by 1 ULP between local Intel and the CI AMD runner; hence libm in hyst-compile.
- afekt has no Rust code (until WO-3); it uses synteza via SYNTEZA_DIR, built binaries, scripts, fixtures.
- hyst-render tests need a GPU locally; run them only when a slice touches it.
- Frozen src/ and tools/ are read-only. ONE arm; ensemble code is the v47 reference only.
- Commit messages are plain, no attribution lines.
