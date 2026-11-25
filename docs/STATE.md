# State 2026-11-25

## Goal
synteza: GPL visual runtime and show director. Given music with known structure it produces a varied show whose
changes land on the music; screen first, simulated lights/lasers on the same cue stream later. Standalone, and
kabl V2's video half (kabl depends on synteza, never the reverse). Plan: docs/PLAN.md (2026-11-24); the
2026-11-22 proposal (docs/PLAN_2026-11-22.md) is valid only where the new plan does not contradict it.

## Done
- #17-#33: CI, licence, docs, afekt move, look contact sheets + A/B, Julia spike, TS detectors in hyst-audio.
- #41 plan; #42 WO-1 scripts/rpp.py (Reaper .RPP -> sidecar: source "daw", exact beats, downbeats, lanes[],
  markers[]; measured wav offset, refuses when ambiguous); #44 check clip shows note length/overlap, only lanes and
  stripes used in the window; #45 sampler tracks split into one lane per note, named from the preset.
- #39 sketch scenes (grid/growth/feedback + switcher) merged as test material, off unless SCENE is set.
- #46 WO-2: hyst-core Sidecar gains `downbeats: Option<Vec<f64>>`; scenes take bars from it when present.

## In flight
- No builder running. WO-2 blind kit sent 2026-11-25: /secondary/hyst-data/synteza/wo2/ab/{grid,growth,feedback,
  auto,extra}-{X,Y}.mp4, key.txt there (extra = hidden repeat of grid, sides swapped).

## Waiting on owner
- Gate 1 (WO-2): per pair, which is more on the beat, X / Y / no difference.
- Does each drum row of /secondary/hyst-data/synteza/wo1b/afterimage-lanes.mp4 light on its drum?
- Repo secret SITE_DISPATCH_TOKEN: the "notify site" workflow fails on every master push since at least
  2026-11-23 with HTTP 401 Bad credentials. Only the owner can set secrets.
- PLAN.md section 11; soonest: which two av_proof looks for WO-4.

## Next (docs/PLAN.md section 9; stop at each gate)
1. WO-3 after Gate 1 is answered: one sketch scene, 3-4 layers each bound to one lane (kick = afterimage
   Generator / panicspiral KICK; Perc 1 · SN, Perc 1 · OH, a synth); blind A/B vs WO-2. Gate 2: still "simple"?
   Only if Gates 1 and 2 both fail are the scenes the problem: drop the sketches, go to WO-4 (PLAN.md section 9).
2. WO-4 port two av_proof looks; WO-5 cue model + switching; WO-6 director.
3. Independent: hyst-audio BeatTracker reads 119.68 BPM on every track (beat.rs:49, :114-115).

## Decisions
- Julia stopped; sketch scenes rejected as looks. Structure comes from the best tier available (DAW project >
  offline analysis > live audio) and the sidecar says which. Every work order ends in a clip, judged blind.
- No visual lead baked in: owner's device accepts -100..+150 ms as "together" (calibration 2026-11-25), so fine
  timing cannot be judged from his clips. Lead is a show-time setting (checkclip.py still defaults to 83 ms).
- A lane is one Reaper track; sampler tracks (DecentSampler, ReaSamplOmatic5000) split one lane per note.
- Automatic first; authoring and live later on the same cue stream. Lights/lasers: simulation only.
- afekt (the arm) is wholly separate; it uses synteza only via SYNTEZA_DIR (scripts/analyze*, beat_pll).

## Hard-won facts
- afterimage, panicspiral, event_horizon: 120 BPM 4/4, no tempo points, no markers or regions; wav offset ~0 ms.
- afterimage lanes: Generator (= kick, sidechains MAIN SYNTH), Perc 1 · BD 36/RIM 37/SN 38/CLAP 39/TOM1 41/OH 46/
  TOM3 47/TOM4 48 (Linn Drums kit), Hats, MAIN SYNTH, SECONDARY SYNTH/CHRONOS, FX / ONE-SHOTS. BASS and TEXTURE /
  ATMOS have no items (silent). panicspiral: KICK, Clap, Hats, Hiss, MAIN SYNTH, #2, #3, #4.
- The old sketch clips used /secondary/hyst-data/synteza/song1/sidecars/afterimage.s4.sidecar.json (schema 4), not
  the wo1 sidecar. WO-2 control: wo2/afterimage.s4-exact.sidecar.json = s4 with exact beats, tempo, downbeats.
- Python env for scripts: /secondary/hyst-env/analyze-py/bin/python (pytest).
- Owner reviews on Remote Control: send clips as files (SendUserFile limit 30 MiB), never localhost links.
- Merges: only on the owner's "merge <n>"; the hook needs nonslop verify + review + all CI. slopstop denies
  agents editing .github/ and .nonslop.toml. `gh pr edit` fails; use `gh api -X PATCH`.
- Frozen src/ and tools/ are read-only. ONE arm. Commit messages are plain, no attribution lines.
