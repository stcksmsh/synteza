# State 2026-11-25 (handover)

## Goal
synteza: GPL visual runtime and show director. Given music with known structure it produces a varied show whose
changes land on the music; screen first, simulated lights/lasers on the same cue stream later. Standalone, and
kabl V2's video half (kabl depends on synteza, never the reverse). Plan: docs/PLAN.md (2026-11-24); the
2026-11-22 proposal (docs/PLAN_2026-11-22.md) is valid only where the new plan does not contradict it.

## Done
- #17-#33: CI, licence, docs, afekt move, look contact sheets + A/B, Julia spike, TS detectors in hyst-audio.
- #41 plan; #42 WO-1 scripts/rpp.py (Reaper .RPP -> sidecar: source "daw", exact beats, downbeats, lanes[],
  markers[]; measured wav offset); #44 lane check clip; #45 sampler tracks split one lane per note, named from preset.
- #39 sketch scenes (grid/growth/feedback + switcher) merged as test material, off unless SCENE is set.
- #46 WO-2: hyst-core Sidecar `downbeats`; scenes take bars from it. Gate 1 result (owner 2026-11-25): no visible
  difference (grid new?, growth old, feedback old, auto new?); his reason: the sketches are not responsive or clear
  ("growth mostly static with blips", "feedback meh", "grid unclear"). Exact bars stay.

## In flight
- PR #48 claude/wo4-avproof (WO-4), head a645b2b: kabl's av_proof look ported as SCENE=avproof, every element bound
  to Reaper lanes (hyst-core Sidecar gains `lanes`). verify ok, review clean, CI 8/8. NOT merged: a new look merges
  only after the owner's verdict. Clips sent (twice) from /secondary/hyst-data/synteza/wo4/: afterimage-avproof-37-97,
  panicspiral-avproof-100-160, afterimage-avproof-full (+ *-sheet.png, side-by-side-composition-vs-afterimage.png).
  If master moves before he answers, have the branch merge origin/master and re-verify/review/CI.

## Waiting on owner
- WO-4 verdict: follows the music (vs sketches)? keep / change / drop? what is off? Candidates noted to him: the
  12-row step block rising into the core (kabl had 2 rows at the bottom); grey whole-frame flash on section changes
  (exactly kabl's flash*0.05, e.g. panicspiral clip 0:37).
- Does each drum row of /secondary/hyst-data/synteza/wo1b/afterimage-lanes.mp4 light on its drum? (He said the
  earlier lane clips were "ok".)
- notify-site.yml: pings stcksmsh.github.io to rebuild for the old TS npm package; token never set, all 40 recent
  runs failed. Coordinator recommended deleting it; owner asked "no need for site dispatch anymore isn't there?"
  but has not said "delete it". .github/ edits need his explicit word.

## Next
1. Act on the WO-4 verdict: keep -> merge #48, then WO-5 (cue model + switching) with avproof as the scene;
   change -> variants behind a switch, clips side by side; drop -> back to the owner.
2. WO-3 was skipped on the owner's choice (2026-11-25, "1"): sketches stay as test material only.
3. Independent: hyst-audio BeatTracker reads 119.68 BPM on every track (beat.rs:49, :114-115).

## Decisions
- Julia stopped; sketch scenes rejected. Every work order ends in a clip, judged by the owner.
- No visual lead: calibration 2026-11-25 (flash vs beep, his device) he called -100, +50 and +150 ms "ok" and did
  not pick 0 ms, so fine timing cannot be judged from his clips. Lead is a show-time setting.
- A lane is one Reaper track; sampler tracks (DecentSampler, ReaSamplOmatic5000) split one lane per note.
- Automatic first; authoring and live later on the same cue stream. Lights/lasers: simulation only.
- afekt (the arm) is wholly separate; it uses synteza only via SYNTEZA_DIR (scripts/analyze*, beat_pll).

## Hard-won facts
- afterimage, panicspiral, event_horizon: 120 BPM 4/4, no tempo points, no markers/regions; wav offset ~0 ms.
- afterimage lanes: Generator (= kick), Perc 1 · BD/RIM/SN/CLAP/TOM1/OH/TOM3/TOM4 (Linn Drums), Hats, MAIN SYNTH,
  SECONDARY SYNTH/CHRONOS, FX / ONE-SHOTS; BASS, TEXTURE / ATMOS silent. Sidecars with lanes: hyst-data/synteza/wo1b/.
- av_proof reference: kabl/.worktrees/living-rack/tools/examples/av_proof.rs; masters ~/Videos/kabl-av/ (1080p,
  >30 MiB; phone copies in hyst-data/synteza/avproof/). SCENE=avproof needs IBM Plex Mono at
  /usr/share/fonts/truetype/ibm-plex/IBMPlexMono-Regular.ttf (example only).
- Old sketch clips used song1/sidecars/afterimage.s4.sidecar.json (schema 4), not the wo1 sidecar.
- Owner reviews on Remote Control: clips as files (SendUserFile limit 30 MiB). Python env:
  /secondary/hyst-env/analyze-py/bin/python. Merges only on his "merge <n>"; hook needs verify + review + all CI.
- Frozen src/ and tools/ are read-only. ONE arm. Commit messages are plain, no attribution lines.
