# synteza: direction and first work orders

Date: 2026-11-24. From: the product-research session with the owner. To: the synteza coordinator.
This is the plan. The 2026-11-22 proposal is kept as `docs/PLAN_2026-11-22.md` (it stays
valid only where this file does not contradict it; its output-chain diagnosis, art rules A1-A14, licence rules and
shared building blocks still stand). Then slice work orders from section 9. **Start with WO-1.**

Labels: **Owner** = said by the owner today. **Fact** = checked in the repos (path given) on 2026-11-24, by reading,
nothing was run. **Evidence** = from outside sources, with how well it was checked. **Rec** = recommendation, the
owner may change it. Taste is never argued in text: every work order ends in a clip the owner judges.

---

## 1. What changed today

**Owner**
- Verdict on the sketch clips (PR #39): bad. "simple and not on beat or too changing presets, not matching the music
  and very bad transitions between them".
- afekt is the arm only and wholly separate. Old "hysteresis" decisions about arms, lasers or scope do not bind synteza.
- kabl gets the synteza suite as its V2 video half. synteza is also a standalone program.
- The agent may read the Reaper project files itself. They are in the same folders as the wavs
  (`~/Music/Singles/Published/<name>/`).

**Fact**
1. The sidecar beat grid is produced by the same tracker as the live one. `scripts/structure.ts:268` calls
   `beatTracker.update(novelty, tNow)` and writes its averaged tempo (`:291`, `:339`). The Rust tracker starts at
   120 BPM with an octave bias around its previous estimate (`crates/hyst-audio/src/beat.rs:49`, `:114-115`).
   STATE reports 119.68 BPM on every track, live and in sidecars. The exact mechanism of the lock is not pinned down.
2. The sketch scenes take beat and bar from that sidecar (`scenes/music.rs` on `claude/sketch-scenes`). So every clip
   the owner has judged, Julia included, ran on a wrong grid.
3. The sketch switcher merges sections down to an 8 s minimum, picks the scene from the section's loudness rank by
   fixed thresholds, and uses one 4 s cross-dissolve for every change (`scenes/switcher.rs`: `MIN_SECTION_SECS`,
   `choose_scene`, `BLEND_SECS`).
4. The owner has liked exactly one visual of his own: kabl's `tools/examples/av_proof.rs`. `kabl/docs/av-audit/
   KABL_V2_PLAN.md` (2026-11-19) records "It works" and "It looks good". It was driven by exact patch structure
   (steps, gates, envelope attacks, clock, cues, notes), each visual element tied to one source. The plan's own
   conclusion: "An FFT of the mix cannot show which lane fired."
5. That kabl plan builds its own wgpu runtime (`new crates/visual`) and its decision D-24 says "Sinteza/Afekt
   boundary: no code link found ... out of scope for 1.1". This conflicts with the owner's statement above (see 8).
6. synteza is on wgpu 22 (`crates/hyst-render/Cargo.toml:11`); kabl uses wgpu through egui-wgpu (brief: wgpu 29).

**Working hypothesis (Rec, untested).** What the owner's eye needs is exact timing plus visible cause and effect:
several elements, each driven by one musical source. The sketches gave him one layer reacting to a mixed signal on a
wrong grid. WO-1 and WO-2 test the timing half at almost no cost; WO-3 tests the cause-and-effect half.

---

## 2. Product definition (Rec)

synteza is a GPL visual runtime and show director. Given music with known structure, it produces a varied show whose
changes land on the music: screen visuals first, simulated lights and lasers on the same cue stream later. It runs
unattended on an integrated-GPU Linux laptop. It has two hosts: its own player (videos of the owner's tracks, then an
installation) and kabl V2 (live, where structure is known exactly).

Users, in order: the owner making videos for his tracks; an unattended installation; kabl V2; later small live shows.

It must beat what exists on three things:
1. **Changes that follow musical structure**, not loudness. (Evidence: projectM's hard cuts are documented as
   triggered when the music gets louder; reference shows such as Max Cooper, Flying Lotus Layer 3 and Prydz HOLO are
   people performing or pre-programmed, per artist and crew interviews found in the research.)
2. **Legible cause and effect**: an element per musical lane, not one image reacting to a mix.
3. **One cue stream** for screen, lights and lasers, previewed on screen, Linux-native.

Not in 1.0: a patch editor (kabl is the editor), any dependency on kabl or afekt, claims of live section
understanding, real laser or DMX hardware.

---

## 3. Where musical structure comes from

Use the best source available and say which one is in use. Never present a lower tier as a higher one.

| Tier | Source | Gives | Status |
|---|---|---|---|
| 1 | **DAW project** (Reaper `.rpp`) for the owner's tracks | exact tempo map, bars, markers/regions, notes per instrument | WO-1 |
| 1 | **kabl telemetry** (live) | exact clock, steps, gates, envelopes, cues, notes, with sample positions | kabl V2 plan R1-R5 |
| 2 | **Offline analysis** of unknown tracks | beats and downbeats (good), sections (moderate) | later, WO-10 |
| 3 | **Live audio only** | onsets, band energy, loudness; beat with errors; no bars, no sections | last |

**Evidence for tiers 2-3** (from the research report; papers not re-opened by this session unless stated):
- Online beat tracking: real-time PLP F1 about 75 %; BeatNet+ beat F1 about 81 %, downbeat F1 about 57 %. So live
  downbeats are wrong nearly half the time. Live section detection: no usable method found.
- Offline: Beat This! (MIT code and weights, owner-verified in `THIRD_PARTY_LICENCES.md`). Section boundaries within
  0.5 s: roughly 0.6-0.7 on pop benchmarks; electronic music is less reliable.
- **Licence conflict to resolve before use:** the research report lists All-In-One as MIT; the old PLAN §6.2 says it
  depends on madmom, whose model weights are CC BY-NC-SA. Treat All-In-One as private evaluation only until someone
  reads its repository and dependencies by hand. BeatNet+ and SongFormer weights licences: not verified.
- Commercial automatic lighting (rekordbox Lighting, SoundSwitch autoscript) relies on offline phrase analysis.

**Sidecar change (Rec).** Additive, as the schema rule requires: `source` ("daw" | "analysis"), beats and downbeats
from the tempo map when the source is "daw", `lanes[]` (name, kind, events with time, pitch, velocity, length),
`markers[]` (name, start, end). The lane shape should match what kabl's telemetry will carry, so one scene binding
works for both hosts.

---

## 4. The show model (Rec)

- **Scene**: a generative renderer with parameters, its own persistent state, declared inputs, and rig hints
  (palette, motion family, strobe allowance for the simulated lights).
- **Layer**: one visual element inside a scene, bound to one lane or one clock. A scene has several.
- **Look**: a named parameter preset of a scene.
- **Cue**: a timestamped instruction on the musical clock: target (screen | rig | laser | all) and action
  (go to look with a transition, set parameter with a ramp, accent, intensity, blackout, hold). A cue carries no
  audio features.
- **Show**: the ordered cue list for one track plus global rules (palette family, intensity budget, flash limit).

One cue stream, three producers, built in this order:
1. **Director** (automatic): reads the sidecar and writes the cue list before playback.
2. **Authoring**: the same list as an editable file; the director's output is the first draft.
3. **Live**: a control surface or kabl's Perform view inserts cues, quantised to the next beat, bar or phrase.

Continuous reaction inside a scene (a lane's note lights its layer) is not cued; it is the scene's bindings, scaled
by the cue's intensity. Rave shows and art shows are the same model with different rule sets (rotation rate,
contrast, strobe budget, rig activity).

---

## 5. Timing and switching rules

**Two timing budgets.**

| Kind | Examples | Budget | Basis |
|---|---|---|---|
| Light | flash, cut, strobe, laser hit | tight: must not trail the sound by more than about 45 ms | old PLAN rule A9 (ITU-R BT.1359, snippet-level, **unverified**) |
| Motion | sweeps, growth, camera, anything that "lands" | loose: landing within about ±150 ms; slightly early is better than late | see below |

**Evidence (page read 2026-11-24, first 100k of 136k characters):** Takehana, Uehara and Sakaguchi, PLOS ONE 2019,
doi 10.1371/journal.pone.0221584. Observers judged whether a person's movement was in sync with music. The window
in which it still read as in sync averaged about 300-450 ms total width. They judged from where a movement lands
(hand low points, reach ends), never from where it starts, and predicted the landing. In most exercises sync was
judged best with the motion slightly ahead of the sound. Limits: natural video of calisthenics, 15 students;
transfer to abstract visuals is an inference.

Consequences (Rec): schedule motion from the known grid so it lands on the beat; drive light from the event itself;
correct tempo matters more than exact phase; acceptance for any tracker is "right tempo and landings inside the
window", judged by a flash clip, not an F-measure.

**Switching (Rec).** Fixes the three sketch complaints at their source.
- Change scene only at a marker/region boundary, or every 16 or 32 bars inside a long section. Never on a loudness
  threshold. A repeated section gets its earlier scene back with a variation.
- Scene family follows the section's role and energy; within a section only looks and intensity move.
- Transitions, chosen per boundary: (a) cut on the downbeat, for drops; (b) morph through the shared feedback
  buffer over 1-4 bars, so the outgoing image seeds the incoming one; (c) slow dissolve in OKLab, for quiet
  sections only. Transitions complete on the boundary, they do not start there. No fixed-length dissolve.
- Changes must be structural. (Owner's own lesson from the arm work: small changes are invisible, structural
  changes read. Sub-looks of one scene did not read as change in the song-aware Julia.)
- At most three full-field flashes per second (old PLAN A10).

---

## 6. Scenes (Rec)

Cost figures are estimates for 1080p on Iris Xe. **Nothing is measured**; WO-7 adds the timing overlay.

**First: the looks he already accepted.** Port the elements of kabl's `av_proof.rs` to synteza as GPU layers, each
bound to a lane: lattice, beams/arcs, waveform core, spectrum ring, particles, shockwave, feedback, palette scene
(list from `KABL_V2_PLAN.md` V1.1). Both repos are the owner's and GPL-3.0-or-later.

**Then, by his reference ratings** (keep only what passes his eye):

| Scene | Reference he rated | Technique | Est. cost | Driven by |
|---|---|---|---|---|
| raster | Alva Noto "Nice" | analytic lines, grids, moiré; black/white + one accent | < 1 ms | beat: step; bar: pattern; lane: line per note |
| life / coral | Max Cooper, "complex yet simple" | cellular automata; Gray-Scott at half resolution | 1-6 ms | beat: generation step; note: seed; section: rule |
| signal | LZX "100% doable" | oscillators, keyer, colouriser, feedback transform | 1-2 ms | bar: feedback transform; section: patch |
| scope | Fenderson (kabl A/V later) | phosphor beam; also the laser preview path | < 1 ms | lane or stereo signal |
| mycelium | 404.zero | Physarum agents in compute | 3-8 ms | beat: deposit; section: agent count |
| glitch (processor) | Weirdcore | datamosh-style smear, pixel sort, slit-scan on any scene | 1-3 ms | onset: tear; section: on/off |
| glyph (layer) | Weirdcore | MSDF text over any scene | < 1 ms | beat: step; marker: text |
| pointcloud | Weirdcore, FlyLo | points sampled from SDF/noise surfaces | 3-6 ms | bar: camera; onset: dropout |

The three existing sketches (grid, growth, feedback) are kept as test material for WO-2 and WO-3 only. Whether they
survive is decided by those clips. The Julia line stays stopped.

---

## 7. Lights, lasers, smoke: simulation only (Rec)

- A `rig.json` describes a stage: screen, fixtures (wash, beam, strobe, laser), DMX patch.
- Cues become DMX universes (`[u8; 512]`). The simulator renders from those bytes, so the preview and any later real
  output cannot disagree.
- Render: dark stage, beams as cones marched through haze at half resolution, added into HDR before bloom. Lasers
  reuse the scope beam renderer.
- Later output, crates as listed on crates.io (licences reported there, **not hand-checked**): `sacn` (MIT OR
  Apache-2.0), `artnet_protocol` (MIT), `ether-dream` and its DAC emulator (MIT OR Apache-2.0), `laser-dac` (MIT).
  Avoid `etherdream` (BSD-4-Clause, not GPL-compatible). The frozen TS encoders are reference only.
- No audience-scanning features. Real laser use needs local regulation checked first (not researched).

---

## 8. Relation to kabl V2

**Owner:** kabl V2's video half is the synteza suite; synteza is also standalone.

**Rec (needs the owner's yes, because it changes kabl's V2 plan):**
- synteza provides the visual runtime: GPU layer, frame graph, scenes, post chain, cue stream, director, player.
- kabl provides the telemetry bus (its R1-R5), bindings, the editor and Perform cues. kabl depends on synteza, never
  the reverse.
- kabl's planned `crates/visual` is not built; kabl's M1 live proof is built on synteza crates. D-24 is reopened.
- For that, synteza must: move to kabl's wgpu major, accept a host-created Device/Queue, have no global state and no
  panics (kabl builds with `panic = "abort"`).
- Until kabl 1.0 is tagged, nothing lands in kabl's master for this; kabl's audio-first rule stands.

Parallel now: kabl 1.0 work; synteza WO-1 to WO-6. Joined later: WO-8.

---

## 9. Work orders, in order

Each ends in a clip or capture for the owner. Stop at each gate.

**WO-1. Exact grid and lanes from the Reaper projects.**
Input: `~/Music/Singles/Published/<name>/` (`.rpp` beside the `.wav`). Read-only; never write to or re-save a
project. afterimage first, then panicspiral.
1. Extract tempo, time signature, tempo map; markers and regions; per Reaper track: name, MIDI notes (time, pitch,
   velocity, length), audio item starts, mute state.
2. Find the offset between project time and the published wav (the render may not start at 0). Confirm by
   correlating the grid with the wav's onset envelope; report offset and residual in ms.
3. Write the sidecar fields of section 3.
4. If a project is missing, has several versions, or the offset is ambiguous: stop and report, do not guess.
   Parts recorded as audio have no notes: use item starts, or the afterimage stems, and say so.
Ends in: a 60 s check clip of afterimage with audio: flash per beat, another colour on downbeats, one row of ticks
per lane. Owner answers "on the beat?" and "are the lanes right?".
Note: the `.rpp` format is expected to be plain text with tempo, markers, items and inline MIDI; that is from
general knowledge and is confirmed or refuted by step 1.

**WO-2. Same sketches, exact grid.** Re-render the four afterimage sketch clips (grid, growth, feedback, auto) with
the WO-1 grid and nothing else changed. Blind A/B against the old clips, hidden repeats.
**Gate 1:** is "not on beat" gone?

**WO-3. One element per lane.** In one sketch scene, bind three or four layers each to one lane (for example kick,
bass, lead, hats). Blind A/B against the WO-2 version of the same scene.
**Gate 2:** does it stop reading as "simple"? If gates 1 and 2 both fail, the scenes are the problem: go to WO-4
and drop the sketches.

**WO-4. Port two av_proof looks** as lane-bound GPU scenes (owner picks which two from the kabl clips).
Ends in: afterimage and panicspiral excerpts, A/B against the best sketch.

**WO-5. Cue model and switching.** Cue types, cue-list file, player; switching rules and the three transitions of
section 5. Ends in: one hand-written cue list on afterimage; A/B dissolve against feedback morph on the same boundaries.

**WO-6. Director v1.** Cue list generated from markers, bars and energy. Ends in: full-length panicspiral and
event_horizon, A/B against the WO-5 hand-written list, plus one swap test (the track with its own cues against the
cues of a tempo-matched other track; the owner should pick his own).

**WO-7. Engine step.** wgpu to kabl's major; host-provided Device/Queue; live preview window; per-scene GPU time
overlay at 1080p on the reference laptop; confirm compute shaders on Iris Xe (Vulkan). Ends in: screen capture with
the ms readout; the cost column of section 6 replaced by measurements.

**WO-8. kabl join (blocked on the decision in section 8 and on kabl's own gates).** kabl M1 live proof on synteza
crates, one scene driven by kabl telemetry. Ends in: the owner plays it with a keyboard.

**WO-9. Rig simulator v0.** `rig.json`, cues to DMX bytes, haze beams. Ends in: one drop with screen plus eight
simulated beams.

**WO-10. Unknown tracks.** Offline beats and downbeats (Beat This!), sections from the existing SSM pipeline snapped
to the bar grid; replace the live tracker (real-time PLP, MIT, written from the paper) only when the installation
needs live input. Ends in: flash check clips on three tracks that are not the owner's.

---

## 10. How clips are judged (reused from the afekt work)

- Blind, coded clips, side and order randomised; about 20 % hidden repeats.
- If more than 40 % of repeats flip, the session is unreliable: repeat it once with fresh excerpts. (The owner's
  blind judgements flipped in about 10 of 35 cases between sessions.)
- A scene joins the default set only after a "keep" in two separate sessions on two tracks.
- Always judged with the music, on his own tracks.
- Nothing is tuned against a numeric score for "fits the music". Numbers passed and the eye failed three times on
  the arm. Numeric checks remain only as stop rules (flash limit, frame time, clipping).
- No new mechanism is proposed from a measurement alone; only for a defect the owner named in a viewing.

---

## 11. Open decisions for the owner

1. kabl V2's visual runtime is the synteza crates, replacing `crates/visual` in the kabl plan and reopening D-24: yes?
2. Which two av_proof looks to port first (WO-4)?
3. Default rotation: scene change only at markers, or also every 16/32 bars inside long sections?
4. Is the "Weirdcore side" (glitch, glyph, pointcloud) inside the first round of scenes or after WO-6?
5. First simulated rig: a club (screen, 8 beams, 4 strobes, 1 laser) or a festival stage?
6. May offline analysis for unknown tracks stay in Python on his machine (never shipped), or must it be Rust?
7. Is "structure required" acceptable for 1.0 (DAW sidecar, kabl, or offline analysis), with live-only audio as a
   later, weaker mode?

---

## 12. Not verified

- Every GPU cost in this file.
- The cause of the 119.68 BPM lock (only that both paths share the tracker).
- The `.rpp` contents until WO-1 reads real files.
- The 45 ms "light" budget; the transfer of the motion-sync study to abstract visuals.
- Licences of All-In-One (conflict above), BeatNet+, SongFormer weights, and all show-control crates beyond their
  crates.io listing.
- Competitor status and prices (Resolume, TouchDesigner, Synesthesia and others) were not re-checked.
- How Bicep's CHROMA show is synchronised: no source found. Dorćoleta's visuals: nothing found online.
- The research session did not run any synteza or kabl code.
