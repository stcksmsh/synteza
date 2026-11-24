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
- Nothing running. PR #35 (song-aware Julia) is parked as a draft: the owner rejected it. Do not merge it as a
  look. It holds reusable tooling (JULIA_FREEZE, JULIA_CRF, raw-frame cleanup) to salvage later.

## Owner verdict 2026-11-24 (stops the Julia line)
- Flick grid: "all are bad"; only frozen pan is calm "but it's ugly".
- Blind pairs: afterimage and event_horizon, song-aware on and off: "both shit" on both songs.
- "it might be the julia in general ... it's also too simple for music, we'd probably need patch switching or smth
  more"; "the julia might work for some songs and some sections but not for others, we need to rethink the ENTIRE
  project basically I think".
- So: no more Julia tuning. No engine work (S1, WO-11) until the direction is re-decided with the owner.

## Next
1. Re-decide the direction with the owner (questions asked 2026-11-24: reference visuals he likes; whether any
   earlier version ever looked right; several different scenes switched per section vs something else; whether
   the rethink covers the product or only the visuals).
2. Only then slice. Taste is judged on clips of his tracks sent as files, never argued in text.
3. Independent of direction: hyst-audio BeatTracker reads 119.68 BPM on all owner tracks (locked near its 120
   prior; beat.rs:50, :114-115, :124); live drop/break detectors do not work on real audio.

## Waiting on owner
- The direction questions above.
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
