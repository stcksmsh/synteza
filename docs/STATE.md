# State 2026-11-24

## Goal
synteza: GPL visual runtime and show director. Given music with known structure it produces a varied show whose
changes land on the music; screen first, simulated lights/lasers on the same cue stream later. Standalone, and
kabl V2's video half (kabl depends on synteza, never the reverse). Plan: docs/PLAN.md (2026-11-24, from the
owner's research session). The 2026-11-22 proposal is docs/PLAN_2026-11-22.md: valid only where the new plan
does not contradict it (output chain, art rules A1-A14, licence rules).

## Done
- #17-#31: CI (fmt/clippy/test/render on lavapipe), licence, docs, afekt move, scripts/look contact sheets + A/B.
- #32 WO-7 (old numbering) Julia look spike; #33 TS build/drop/break detectors in hyst-audio.
- claude/state-10: new plan landed as docs/PLAN.md.

## In flight
- WO-1 (new plan), branch claude/wo1-reaper-grid: exact grid and lanes from the Reaper projects, written as
  additive sidecar fields, plus a 60 s check clip of afterimage. Check `nonslop status` for the builder.
- PR #39 claude/sketch-scenes (draft, CI green, not reviewed): grid/growth/feedback scenes + switcher. Owner
  verdict 2026-11-24: bad ("simple and not on beat or too changing presets, not matching the music and very bad
  transitions"). Kept only as test material for WO-2 and WO-3. Do not merge as a look.
- PR #35 claude/song-aware-julia: parked draft, rejected. Julia line is stopped.

## Next (docs/PLAN.md section 9; stop at each gate)
1. WO-1 check clip to the owner: "on the beat?", "are the lanes right?".
2. WO-2: re-render the four afterimage sketch clips on the WO-1 grid, nothing else changed; blind A/B vs
   /secondary/hyst-data/synteza/sketch1/. Gate 1: is "not on beat" gone?
3. WO-3: one sketch scene with 3-4 layers each bound to one lane; blind A/B vs WO-2. Gate 2: still "simple"?
4. WO-4 port two av_proof looks (kabl tools/examples/av_proof.rs); WO-5 cue model + switching; WO-6 director.
5. Independent: hyst-audio BeatTracker reads 119.68 BPM on every track (beat.rs:49, :114-115).

## Waiting on owner
- PLAN.md section 11, seven decisions. Needed soonest: (2) which two av_proof looks for WO-4; (1) kabl V2
  runtime = synteza crates (blocks WO-7/WO-8 only).
- Close PR #35 and delete stale branch claude/hysteresis-choreography-preview-ri7cqh (2026-09-24, no PR)?

## Decisions
- Julia stopped: every form rejected. Sketch scenes rejected as looks.
- Structure comes from the best tier available (DAW project > offline analysis > live audio), and the sidecar
  says which. Taste is never argued in text: every work order ends in a clip, judged blind on his own tracks.
- Automatic first; authoring and live later on the same cue stream. Lights/lasers: simulation only.
- afekt (the arm) is wholly separate; it uses synteza only via SYNTEZA_DIR (scripts/analyze*, beat_pll).

## Hard-won facts
- 2026-11-24, read from the .RPP files: afterimage, panicspiral and event_horizon are all `TEMPO 120 4 4`, with
  no tempo-envelope points and NO markers or regions. So 119.68 BPM is near the true tempo: the suspect is
  phase and drift, not tempo. Section boundaries cannot come from markers; use item layout or analysis.
  Other tracks: 0xC000021A 180, SIGSEGV 140, triple_pendulum 160.
- Projects: ~/Music/Singles/Published/<n>/<n>.RPP beside <n>.wav; read-only. afterimage has stems
  (afterimage_only_{flute,linn_drums,synth}.wav) and tracks Perc 1, Perc 2, Hats, BASS, MAIN SYNTH,
  SECONDARY SYNTH/CHRONOS, TEXTURE / ATMOS, FX / ONE-SHOTS.
- Rust `Sidecar` (crates/hyst-core/src/sidecar.rs) has `beats` but no downbeats; it ignores unknown fields.
- Owner allows the coordinator to `gh pr merge`; the hook needs `nonslop slice verify`, `nonslop slice review`
  and every CI check finished (even text-only PRs).
- Owner reviews on Remote Control: send clips as files (SendUserFile limit 30 MiB), never localhost links.
- slopstop denies agents editing .github/ and .nonslop.toml. `gh pr edit` fails; use
  `gh api -X PATCH repos/stcksmsh/<repo>/pulls/<n>`.
- Render example: width % 64 == 0; SCENE/JULIA_* env; video-only mp4, mux audio with ffmpeg. Renders under
  systemd-run --user, data in /secondary/hyst-data.
- Frozen src/ and tools/ are read-only. ONE arm. Commit messages are plain, no attribution lines.
