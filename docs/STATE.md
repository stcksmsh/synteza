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
- No agent running.
- PR #39 (draft, CI green, NOT yet reviewed, not merged), branch claude/sketch-scenes: three rough non-fractal
  scenes (grid, growth = Gray-Scott, feedback = video-synth) + section switcher with 4 s cross-dissolve;
  SCENE=grid|growth|feedback|auto with SCENE_SIDECAR; frames piped to ffmpeg (no raw frames on disk).
  Eight clips sent to the owner as files on 2026-11-24 from /secondary/hyst-data/synteza/sketch1/
  (auto-afterimage, auto-panicspiral, and grid/growth/feedback on each track). AWAITING his verdict:
  (1) does switching scenes with the song work, (2) keep/change/drop per scene, (3) what is missing.
  Known: in auto, feedback takes most of the time (loud "Other" sections map to it); beat-locked motion may be off
  (sidecars report tempo 119.68, likely a fallback grid).
  When he answers: run a nonslop:reviewer on #39, then merge it as a sketch base or park it, as the verdict says.
- PR #35 (song-aware Julia) is parked as a draft; the owner rejected it. Do not merge it as a look.
- Product research runs in a separate chat between the owner and a web agent, from
  /secondary/hyst-data/synteza/brief/synteza-brief.zip (BRIEF.md inside; already sent to him). The owner will bring
  its plan to the next coordinator.

## Direction (owner, 2026-11-24)
- The Julia line is stopped: every form rejected ("too simple for music", "one-dimensional"). No version ever
  looked good to him.
- Goal: one system for both mass-appeal rave/techno shows (screen + lasers, lighting, smoke) and smaller abstract
  art-scene shows. Variety across a song and seamless, music-following switching matter most.
- Automatic first (music drives the show); per-song authoring and live performance later as layers on the same
  cue stream. First uses: videos for his tracks, then an installation, and the visual half of kabl v2 (developed
  in parallel). Generative first, footage later. Lights/lasers: simulation and preview only (he owns no gear).
- afekt (the arm) is wholly separate.
- His ratings of 12 reference visuals are in BRIEF.md (liked: Alva Noto grids, Max Cooper emergent growth,
  404.zero, Bicep CHROMA, Weirdcore, FlyLo Layer 3; disliked: Ikeda strobing).

## Next
1. Take in the web agent's plan when the owner brings it: save it as docs/PLAN.md (replacing the old proposal
   where they differ), then slice work orders from it. Each slice ends in a clip the owner judges.
2. The owner's verdict on the sketch clips (above) feeds the same plan.
3. Independent bugs: hyst-audio BeatTracker reads 119.68 BPM on every track (beat.rs:50, :114-115, :124); offline
   sidecar tempo looks like a fallback too; live drop/break detectors do not fire on real audio.

## Waiting on owner
- Sketch verdict; the web agent's plan.

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
