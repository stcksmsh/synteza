# State 2026-11-23

## Goal
synteza (synteзa): native Rust music visualizer engine, standalone product and kabl's video core (kabl depends on
synteza, never the reverse). The library of looks is the product (docs/PLAN.md). Frozen TS reference in src/, tools/.

## Done
- #17-#20: state, clippy fix, afekt inventory, hyst-hw deleted, fmt, julia steering fix, Rust CI on lavapipe.
- #22 docs/REFERENCE.md (owner's laptop i7-13700H + Iris Xe; owner's tracks ~/Music/Singles/Published).
- #23 libm in hyst-compile, golden digests machine-independent (now lives in afekt).
- #25 docs/THIRD_PARTY_LICENCES.md (WO-10).
- WO-3 / S0: afekt#6 + synteza #26. hyst-compile/hyst-previz/hyst-cli, scripts/dance_*, scripts/author, dance docs
  moved to afekt. afekt got Rust CI and cargo gates. v47 output byte-identical before/after.
- #27 docs/PLAN.md merged as a proposal. #29 README/AGENTS.md rewrite (WO-8).
- #30 scripts/look/: contact sheets, blind A/B page + loopback server; before sheets in docs/plan-evidence/before/;
  render example follows the audio clock (WO-6).
- #28 Licence: GPL-3.0-or-later for code and hysts, CC BY-SA 4.0 for art/docs (COPYING.md); render tests
  run with --test-threads=1 against the lavapipe SIGSEGV.
- #32 WO-7 look spike: HDR targets, hue-keeping tonemap + dither, mip bloom, TS decay range, fbm flow, beam into
  field, direct Julia + curated boundary c. Default look; old one behind JULIA_LOOK=before. Owner blind A/B on his
  track hysteresis: after "a lot better". Synthetic clip: undecidable (owner: unknown song, judge on his tracks).
- #33 WO-13: TS build/drop/break detectors ported to hyst-audio, bus fields filled. On real audio drops never fire
  and break sticks (faithful to TS); not used by the renderer.

## In flight
- Song-aware Julia: identify the "flicks", scale reactions to the song's own character, sub-looks per sidecar
  section with eased transitions, JULIA_SONG=off|on; example deletes raw frames after encode. Branch
  claude/song-aware-julia, builder (opus). Ends in flick diagnostic sheets + full-song blind pairs (afterimage,
  event_horizon) sent to the owner as files.
  Owner's verdict on the new look on his tracks: good on panicspiral; too hard on slow songs (afterimage,
  event_horizon), "flicks" too much; "I just dont like the idea of the entire song being this one-dimensional".

## Next            (ordered, PLAN §9)
1. Owner picks the flick culprit and judges the song-aware blind pairs.
2. Follow-up: stale AGENTS.md section refs in docs/PLAN.md:44, crates/hyst-choreo/README.md, docs/osc.md,
   SINTEZA_OFFLINE_SSM.md; stale module doc in crates/hyst-render/src/lib.rs.
3. WO-11: S1 engine design note (docs/ENGINE.md); licence now settled.
4. WO-9 (test competitor "Infinite"), WO-12 (OKLCH palettes); live detector tuning (drops/breaks on real audio).
5. Follow-up (#19 review minor): julia re-acquire also fires on a smooth deep-zoom boundary; untested.

## Waiting on owner
- PLAN §8.3 still open: 1.0 platforms, library shape (owner: rethink all scenes; §4.2 is the proposal), fate of
  hyst-choreo/hyst-script/hyst-output, TS Pages demo repair or retire, business model.
- External v47 reference HTMLs (music-arm folder) are stale; afekt's concern now.
- Shadertoy terms could not be fetched automatically; owner reads in a browser if Shadertoy code is ever wanted.

## Hard-won facts
- The owner allows the coordinator to `gh pr merge` at any time (2026-11-23). The nonslop merge hook needs
  `nonslop slice verify <branch>` and `nonslop slice review <branch> <verdict>` first.
- The owner reviews over Remote Control: send clips as files (no localhost); judge on his own tracks, not synthetic.
- The slopstop hook denies agents writing .github/ and .nonslop.toml; edit only on the owner's explicit word.
- `gh pr edit` fails (GraphQL Projects classic); use `gh api -X PATCH repos/stcksmsh/<repo>/pulls/<n>`.
- glibc hypot differs by 1 ULP between local Intel and the CI AMD runner: keep deterministic math where digests matter.
- afekt uses synteza only via SYNTEZA_DIR: scripts/analyze* and the hyst-audio beat_pll example.
- hyst-render tests need a GPU locally; CI runs them on lavapipe.
- Frozen src/ and tools/ are read-only. ONE arm.
- Commit messages are plain, no attribution lines.
