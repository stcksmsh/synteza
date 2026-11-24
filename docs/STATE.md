# State 2026-11-24

## Goal
synteza (synteзa): native Rust music visualizer engine, standalone product and kabl's video core (kabl depends on
synteza, never the reverse). The library of looks is the product (docs/PLAN.md, a proposal). Owner: "GOOD and
VARIED hysts"; current scenes to be rethought; Mandelbulb dive dropped. Frozen TS reference in src/, tools/.

## Done
- #17-#31: CI (fmt/clippy/test/render on lavapipe, --test-threads=1), hyst-hw gone, julia steering fix, libm,
  afekt move (afekt#6 + #26), licence GPL-3.0-or-later + CC BY-SA 4.0 (#28), PLAN.md (#27), README/AGENTS (#29),
  REFERENCE.md (#22), THIRD_PARTY_LICENCES.md (#25), scripts/look contact sheets + blind A/B (#30).
- #32 WO-7 look spike = default Julia look (JULIA_LOOK=before keeps the old one). Owner blind verdict on hysteresis:
  "a lot better".
- #33 WO-13 TS build/drop/break detectors in hyst-audio; on real audio drops never fire, break sticks (as in TS).

## In flight
- PR #35 (draft, CI green, review MINOR, not merged): song-aware Julia, JULIA_SONG=on|off (off byte-identical),
  per-song calm (raw RMS dB / range / onset rate), sidecar sections with eased sub-looks, JULIA_FREEZE=<element>,
  JULIA_CRF, raw frames deleted after encode. Builder done; no agent running.
  WAITING on owner's verdict, files already sent to him:
  - flick grid /secondary/hyst-data/synteza/flicks/flicks_grid.mp4 (12 tiles: baseline + one element frozen each:
    jitter beat beam_size c_jumps bloom fold flow wobble palette drive pan). He names the flick culprit.
  - blind pairs (sent copies in song1/small/): afterimage y2 vs y3; event_horizon y1 vs y4.
    Key: /secondary/hyst-data/synteza/song1-key.txt (y1=event_horizon off, y2=afterimage on, y3=afterimage off,
    y4=event_horizon on). Never show the key before he answers.
  After the verdict: send the review minors to a builder (fold flag dead for calm 0.18-0.5 since punch_energy <=
  react; tempo term constant because of the beat-tracker bug; owner_tracks_calm_ordering is a brittle fit lock),
  plus whatever the verdict asks, then `nonslop slice verify` / `slice review`, merge.

## Next            (after #35)
1. Bug: hyst-audio BeatTracker reads 119.68 BPM on all owner tracks: it stays locked near its 120 prior
   (beat.rs:50 start, :114-115 octave bias, :124 integer lags). Diagnose against the sidecar tempo.
2. Whole-song variety beyond one fractal: the PLAN §4.2 library (owner: the whole song must not be
   one-dimensional). WO-11 engine design note (docs/ENGINE.md) before new scenes.
3. Julia repair rows 6-8 (colour/OKLab, relief lighting, AA) as the owner's critique points.
4. Follow-ups: stale AGENTS.md section refs (docs/PLAN.md:44, crates/hyst-choreo/README.md, docs/osc.md,
   SINTEZA_OFFLINE_SSM.md); stale module doc in crates/hyst-render/src/lib.rs; julia re-acquire on smooth deep zoom.
5. WO-9 (competitor "Infinite"), WO-12 (OKLCH palettes), live detector tuning.

## Waiting on owner
- The #35 verdict above.
- Disk: ~24 GB of raw frames in /secondary/hyst-data/synteza/owner1/{event_horizon,panicspiral,afterimage}/
  (agent rm -rf was denied; he runs it himself).
- PLAN §8.3 open: 1.0 platforms, library shape, fate of hyst-choreo/hyst-script/hyst-output, TS Pages demo,
  business model.

## Hard-won facts
- Owner allows the coordinator to `gh pr merge`; the nonslop hook first needs `nonslop slice verify <branch>`,
  `nonslop slice review <branch> <verdict>` and every CI check finished (even text-only PRs).
- Owner reviews on Remote Control: send clips as files (SendUserFile limit 30 MiB), never localhost links. Judge
  looks on his tracks (~/Music/Singles/Published/<n>/<n>.wav; event_horizon 470 s, panicspiral 296 s, afterimage
  171 s), not the synthetic clip. Taste PRs merge only after his blind verdict.
- slopstop denies agents editing .github/ and .nonslop.toml; edit only on the owner's explicit word.
- `gh pr edit` fails (Projects classic); use `gh api -X PATCH repos/stcksmsh/<repo>/pulls/<n>`.
- Render example: width % 64 == 0; JULIA_WIDTH/HEIGHT/SECONDS/OUT_DIR/LOOK/SONG/SIDECAR/FREEZE/CRF; writes a
  video-only mp4, mux audio with ffmpeg. Renders under systemd-run --user, data in /secondary/hyst-data.
- afekt uses synteza only via SYNTEZA_DIR: scripts/analyze* and the hyst-audio beat_pll example.
- Frozen src/ and tools/ are read-only. ONE arm. Commit messages are plain, no attribution lines.
