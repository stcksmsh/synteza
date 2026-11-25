# State 2026-11-25

## Goal
synteza: GPL visual runtime and show director. Given music with known structure it produces a varied show whose
changes land on the music; screen first, simulated lights/lasers on the same cue stream later. Standalone, and
kabl V2's video half (kabl depends on synteza, never the reverse). Plan: docs/PLAN.md (2026-11-24). The
2026-11-22 proposal is docs/PLAN_2026-11-22.md, valid only where the new plan does not contradict it.

## Done
- #17-#33: CI, licence, docs, afekt move, look contact sheets + A/B, Julia spike, TS detectors in hyst-audio.
- #41: the 2026-11-24 plan landed as docs/PLAN.md.
- #42 WO-1: scripts/rpp.py reads a Reaper .RPP into the sidecar (source "daw", exact beats, downbeats,
  lanes[], markers[]); offset to the wav measured, refuses when ambiguous; scripts/rpp/checkclip.py draws the
  check clip, with --lead-ms. Outputs in /secondary/hyst-data/synteza/wo1/.
- PR #35 (song-aware Julia) closed; stale branch claude/hysteresis-choreography-preview-ri7cqh deleted.

## In flight
- No builder running.
- PR #39 claude/sketch-scenes (draft, CI green, not reviewed): grid/growth/feedback scenes + switcher. Owner
  verdict 2026-11-24: bad as a look. Kept only as test material for WO-2/WO-3.

## Waiting on owner
- WO-1 check clips: he said the flashes look "a tad late". The file is exact (audio lag 0.0 ms, flashes on
  frames 0, 30, 60 ...). 12 blind lead variants sent 2026-11-25 from /secondary/hyst-data/synteza/wo1/lead/
  (key.txt there; one hidden lead-0 repeat per track). His pick sets the default visual lead for WO-2 onward.
- WO-2 base: (1) review and merge #39 as off-by-default test material, or (2) WO-2 builds on that branch and
  is never merged. Coordinator recommends (1).
- Lanes: BASS and TEXTURE / ATMOS have no MIDI items in either project (internal sequencer?); afterimage
  "Generator" (quarter notes, pitch 60): kick or sidechain trigger?
- PLAN.md section 11, seven decisions; soonest: which two av_proof looks for WO-4.

## Next (docs/PLAN.md section 9; stop at each gate)
1. WO-2: re-render the four afterimage sketch clips on the WO-1 grid with the chosen lead, nothing else
   changed; blind A/B vs /secondary/hyst-data/synteza/sketch1/. Gate 1: is "not on beat" gone?
2. WO-3: one sketch scene, 3-4 layers each bound to one lane; blind A/B vs WO-2. Gate 2: still "simple"?
3. WO-4 port two av_proof looks; WO-5 cue model + switching; WO-6 director.
4. Independent: hyst-audio BeatTracker reads 119.68 BPM on every track (beat.rs:49, :114-115).

## Decisions
- Julia stopped; sketch scenes rejected as looks. Structure comes from the best tier available (DAW project >
  offline analysis > live audio) and the sidecar says which. Every work order ends in a clip, judged blind.
- Automatic first; authoring and live later on the same cue stream. Lights/lasers: simulation only.
- afekt (the arm) is wholly separate; it uses synteza only via SYNTEZA_DIR (scripts/analyze*, beat_pll).

## Hard-won facts
- afterimage, panicspiral, event_horizon: TEMPO 120 4/4, no tempo points, NO markers or regions. Measured
  project-to-wav offset: afterimage -0.3 ms, panicspiral 0.2 ms (applied 0). All published projects have
  RENDER_RANGE "entire project"; the offset bound trusts it (open minor: untested for time-selection renders).
- Lanes: afterimage Generator, Perc 1, Hats, MAIN SYNTH, SECONDARY SYNTH/CHRONOS, FX / ONE-SHOTS; panicspiral
  KICK, Clap, Hats, Hiss, MAIN SYNTH, #2, #3, #4 (duplicate names suffixed in project order).
- Rust `Sidecar` (crates/hyst-core/src/sidecar.rs) ignores unknown fields; no Rust reads lanes/downbeats yet.
- Python env for scripts: /secondary/hyst-env/analyze-py/bin/python (pytest).
- Owner reviews on Remote Control: send clips as files (SendUserFile limit 30 MiB), never localhost links.
- Merges: only on the owner's "merge <n>"; the hook needs nonslop verify + review + all CI. slopstop denies
  agents editing .github/ and .nonslop.toml. `gh pr edit` fails; use `gh api -X PATCH`.
- Frozen src/ and tools/ are read-only. ONE arm. Commit messages are plain, no attribution lines.
