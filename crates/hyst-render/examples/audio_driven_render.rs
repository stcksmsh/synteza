//! R2 item 5: drive the render pipeline from real audio, not just static
//! uniforms. Loads a real WAV via `hyst_audio::WavSource`, runs it through
//! `hyst_audio::FeatureExtractor` to get real `SignalBus` hops (no realtime
//! playback needed — just push samples through), maps a couple of signals
//! onto visual params (`energy` -> bloom strength, `sub` -> memory-field
//! decay/flow strength), renders a few seconds of frames, and — if `ffmpeg`
//! is on `PATH` — encodes them into a short mp4 under `test-output/`
//! (gitignored; not a committed artifact).
//!
//! Run: `cargo run -p hyst-render --example audio_driven_render -- <path/to.wav>`
//! (defaults to `~/Music/Singles/Published/hysteresis/hysteresis.wav` if no
//! path is given — a real track on this machine, used to verify this once).
//!
//! `JULIA_WIDTH`/`JULIA_HEIGHT` (default 1024x1024) set the output size; the
//! width must be a multiple of 64 (the renderer's readback alignment).
//! `JULIA_SECONDS` (default 90) sets the clip length.
//!
//! `JULIA_LOOK=before|after` (default `after`) picks the pipeline
//! ([`hyst_render::renderer::Look`]), the Julia driver and the per-frame
//! mapping, so both looks come from the same binary. `before` is the
//! original look, unchanged.
//!
//! In `after`, memory-field decay follows the TS original's formula
//! (`screen-composites.ts`): `max(0.86 + 0.105 * build, 0.86 + 0.125 *
//! suspension) + 0.01 * sin(bar phase)`. `hyst-audio` has no build or
//! break detectors yet, so both come from two energy envelopes: `build`
//! rises while the ~2 s envelope sits above the ~20 s one (energy climbing
//! toward a peak), `suspension` while it sits below (a break). At rest
//! both are zero and decay is 0.86.
//!
//! `JULIA_SONG=off|on` (default `off`, `after` look only). `off` is the
//! after look above, unchanged. `on` first measures the whole track
//! ([`hyst_render::passes::julia::song`]): a calm song gets smaller, slower
//! reactions, loudness is read relative to the song's own loud passages,
//! and with a sidecar (`JULIA_SIDECAR=path`, from `scripts/analyze`) each
//! section gets its own eased sub-look. Without a sidecar, `on` applies
//! only the calmness.
//!
//! `JULIA_FREEZE=<element>` (after look, song off) freezes one fast
//! reaction for diagnosis: `jitter`, `beat`, `beam_size`, `c_jumps`,
//! `bloom`, `fold`, `flow`, `wobble`, `palette`, `drive`, `pan`.
//!
//! After a successful ffmpeg encode the raw frames are deleted unless
//! `JULIA_KEEP_FRAMES=1`. `JULIA_CRF` passes a constant rate factor to
//! ffmpeg (its default otherwise).

use std::io::Write;
use std::path::PathBuf;

use hyst_audio::{FeatureExtractor, WavSource};
use hyst_core::Sidecar;
use hyst_render::passes::beam::Segment;
use hyst_render::passes::julia::song::{
    onset_rate, sections_from_sidecar, Blend, Reactor, SectionTrack, SongCharacter, TrackFeatures,
};
use hyst_render::passes::julia::{CuratedJuliaDriver, JuliaDriver};
use hyst_render::passes::memory_field::MemoryFieldParams;
use hyst_render::renderer::Look;
use hyst_render::{FrameParams, GpuContext, Renderer};

const FPS: f32 = 30.0;

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    match std::env::var(name) {
        Ok(v) => v
            .parse()
            .unwrap_or_else(|_| panic!("{name}={v:?} is not a valid number")),
        Err(_) => default,
    }
}

fn main() {
    let width: u32 = env_or("JULIA_WIDTH", 1024);
    let height: u32 = env_or("JULIA_HEIGHT", 1024);
    let duration_secs: f32 = env_or("JULIA_SECONDS", 90.0);
    assert!(
        width > 0 && height > 0 && width.is_multiple_of(64),
        "JULIA_WIDTH must be a positive multiple of 64 and JULIA_HEIGHT positive, got {width}x{height}"
    );
    assert!(duration_secs > 0.0, "JULIA_SECONDS must be positive");
    let look = match std::env::var("JULIA_LOOK").as_deref() {
        Ok("before") => Look::Before,
        Ok("after") | Err(std::env::VarError::NotPresent) => Look::After,
        Ok(other) => panic!("JULIA_LOOK must be before or after, got {other:?}"),
        Err(e) => panic!("JULIA_LOOK: {e}"),
    };
    let song_on = match std::env::var("JULIA_SONG").as_deref() {
        Ok("on") => true,
        Ok("off") | Err(std::env::VarError::NotPresent) => false,
        Ok(other) => panic!("JULIA_SONG must be off or on, got {other:?}"),
        Err(e) => panic!("JULIA_SONG: {e}"),
    };
    assert!(
        !song_on || look == Look::After,
        "JULIA_SONG=on needs the after look"
    );
    let freeze = std::env::var("JULIA_FREEZE").ok();
    if let Some(f) = &freeze {
        assert!(
            FREEZABLE.contains(&f.as_str()),
            "JULIA_FREEZE must be one of {FREEZABLE:?}, got {f:?}"
        );
        assert!(
            look == Look::After && !song_on,
            "JULIA_FREEZE needs the after look with JULIA_SONG=off"
        );
    }
    let frozen = |name: &str| freeze.as_deref() == Some(name);
    let keep_frames = std::env::var("JULIA_KEEP_FRAMES").as_deref() == Ok("1");
    let crf: Option<u32> = std::env::var("JULIA_CRF").ok().map(|v| {
        v.parse()
            .unwrap_or_else(|_| panic!("JULIA_CRF={v:?} is not a number"))
    });
    let wav_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").expect("HOME must be set to find the default WAV");
            PathBuf::from(home).join("Music/Singles/Published/hysteresis/hysteresis.wav")
        });
    println!("loading {}", wav_path.display());
    let wav = WavSource::load(&wav_path).expect("failed to load WAV");

    // Downmix to mono for the feature extractor (stereo placement/pan isn't
    // ported in hyst-audio yet — see its own module doc).
    let mono: Vec<f32> = if wav.channels == 1 {
        wav.samples.clone()
    } else {
        wav.samples
            .chunks(wav.channels as usize)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    };

    const FFT_SIZE: usize = 2048;
    const HOP_SIZE: usize = 512;
    let mut extractor = FeatureExtractor::new(FFT_SIZE, HOP_SIZE, wav.sample_rate as f32);

    let hops_per_sec = wav.sample_rate as f32 / HOP_SIZE as f32;

    // JULIA_SONG=on: measure the whole track first, with its own extractor,
    // so the render pass below starts from the same state as with it off.
    let mut song = song_on.then(|| {
        let character = measure_track(&mono, wav.sample_rate as f32, FFT_SIZE, HOP_SIZE);
        let track = std::env::var_os("JULIA_SIDECAR").map(|path| {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read JULIA_SIDECAR {path:?}: {e}"));
            let sidecar = Sidecar::from_json_str(&text)
                .unwrap_or_else(|e| panic!("parse JULIA_SIDECAR {path:?}: {e}"));
            let sections = sections_from_sidecar(&sidecar, character.calm);
            println!("sections (start-end s, kind, level, look):");
            for (i, s) in sections.iter().enumerate() {
                println!(
                    "  {i:2} {:6.1}-{:6.1} {:?} level {:.2} {:?}{}",
                    s.start,
                    s.end,
                    s.kind,
                    s.level,
                    s.look,
                    s.repeats.map_or(String::new(), |j| format!(" repeats {j}"))
                );
            }
            SectionTrack::new(sections)
        });
        SongState {
            reactor: Reactor::new(character),
            track,
            beam_phase: 0.0,
        }
    });
    // Per-hop decay of the beat pulse: slower for a calmer song.
    let pulse_decay: f32 = match &song {
        Some(s) => 0.85f32.powf(1.0 / s.reactor.character.slow()),
        None => 0.85,
    };
    let total_frames = (duration_secs * FPS) as usize;

    // `JULIA_OUT_DIR` mirrors the sibling Mandelbulb demo's `MANDELBULB_OUT_DIR`
    // escape hatch — lets a concurrent render dodge sharing `test-output/`'s
    // frame filenames with another in-flight render of this same example.
    let out_dir = std::env::var_os("JULIA_OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-output"));
    std::fs::create_dir_all(&out_dir).expect("create test-output dir");
    // ffmpeg's frame_%04d pattern would pick up leftovers from a longer or
    // larger earlier run.
    remove_frames(&out_dir);

    let ctx = GpuContext::new_blocking().expect("no GPU adapter available — cannot render");
    let mut renderer = Renderer::with_look(&ctx, width, height, look);

    let mut hop_count = 0usize;
    let mut frame_index = 0usize;
    let mut t = 0.0f32;
    let dt = 1.0 / FPS;
    // hyst-audio's SignalBus.beat_pulse isn't computed yet (R1 deferred it —
    // see AGENTS.md); a decaying pulse from beat_phase wraparound, local to
    // this demo, is enough to make the beam visibly react to the beat.
    let mut last_beat_phase = 0.0f32;
    let mut beat_pulse = 0.0f32;
    // The Julia substrate's own motion/color now reacts to real audio via
    // a stateful driver (see `hyst_render::passes::julia::JuliaDriver`'s
    // own doc) — replaces the earlier `JuliaNavState::driven_by_time(t)`
    // call, which was a pure function of elapsed time only and never
    // responded to the music at all (user feedback: "too static... doesn't
    // evolve or react to music").
    let mut julia_driver = JuliaDriver::new();
    let mut curated_driver = CuratedJuliaDriver::new();
    // With sections, `c` moves at section changes instead of energy jumps.
    let sections_drive_c = song.as_ref().is_some_and(|s| s.track.is_some());
    curated_driver.set_energy_jumps(!frozen("c_jumps") && !sections_drive_c);
    curated_driver.set_pan_steer(!frozen("pan"));
    let mut structure = EnergyStructure::default();

    // Feed the whole track's samples through in HOP_SIZE-sized pushes so
    // `push_samples` yields exactly one SignalBus per hop, then render a
    // video frame whenever the audio clock passes the next frame time, until
    // we have enough for duration_secs at FPS.
    'outer: for chunk in mono.chunks(HOP_SIZE) {
        let buses = extractor.push_samples(chunk);
        for bus in buses {
            // Gate on tempo_confidence: before a real kick establishes
            // rhythm, BeatTracker's phase free-runs from a default/drifting
            // estimate with low confidence — pulsing on that phase's
            // wraparound anyway reads as the beam guessing wrong and
            // missing, before the real beat ever starts (reported: "tries
            // to glow to the BPM before the kick starts and misses").
            const TEMPO_LOCK_THRESHOLD: f32 = 0.2;
            if bus.tempo_confidence > TEMPO_LOCK_THRESHOLD && bus.beat_phase < last_beat_phase - 0.5
            {
                beat_pulse = 1.0;
            }
            last_beat_phase = bus.beat_phase;
            beat_pulse *= pulse_decay;

            // Emit a frame once the count of SignalBus hops reaches the next
            // frame's time, so the video does not drift against the track (a
            // rounded fixed hop stride does: 3 hops per frame is 28.7 fps at
            // 44.1 kHz). Hops lag the true sample position by a constant ~3
            // hops (~35 ms) while the extractor's window first fills.
            if hop_count as f32 / hops_per_sec >= frame_index as f32 / FPS {
                let aspect = width as f32 / height as f32;
                let params = match look {
                    Look::Before => {
                        let julia = julia_driver.update(dt, &bus);
                        frame_params(&bus, t, beat_pulse, julia, chunk, aspect)
                    }
                    Look::After => match &mut song {
                        Some(song) => song_frame_params(
                            song,
                            &mut curated_driver,
                            &mut structure,
                            &bus,
                            [t, dt, aspect],
                            beat_pulse,
                            chunk,
                        ),
                        None => {
                            let mut drive_bus = bus.clone();
                            if frozen("palette") {
                                drive_bus.chroma_root_hue = 0.0;
                                drive_bus.centroid = 0.5;
                                drive_bus.flatness = 0.3;
                            }
                            if frozen("drive") {
                                drive_bus.energy = 0.5;
                            }
                            let julia = curated_driver.update(dt, &drive_bus);
                            structure.update(dt, bus.energy);
                            let pulse = if frozen("beat") { 0.0 } else { beat_pulse };
                            let samples: &[f32] = if frozen("jitter") { &[] } else { chunk };
                            let base = frame_params(&bus, t, pulse, julia, samples, aspect);
                            let wobble = if frozen("wobble") { 0.0 } else { 1.0 };
                            let mut p = after_frame_params(base, &bus, &structure, wobble);
                            if frozen("beam_size") {
                                p.beam_segments =
                                    lissajous_segments(t * 0.6, 0.5, samples, JITTER_AMP);
                            }
                            if frozen("bloom") {
                                p.bloom_strength = 0.15 + 0.25 * 0.5;
                            }
                            if frozen("fold") {
                                p.memory_field.mirror_strength = 0.0;
                            }
                            if frozen("flow") {
                                p.memory_field.flow_strength = 0.01 + 0.05 * 0.5;
                            }
                            p
                        }
                    },
                };
                let pixels = renderer
                    .render_frame(&ctx, &params)
                    .expect("render_frame failed");
                write_ppm(
                    &out_dir.join(format!("frame_{frame_index:04}.ppm")),
                    &pixels,
                    width,
                    height,
                );
                frame_index += 1;
                t += dt;
                if frame_index >= total_frames {
                    break 'outer;
                }
            }
            hop_count += 1;
        }
    }
    println!("rendered {frame_index} frames to {}", out_dir.display());

    if which_ffmpeg().is_some() {
        let mp4_path = out_dir.join("audio_driven_render.mp4");
        let mut cmd = std::process::Command::new("ffmpeg");
        cmd.args([
            "-y",
            "-framerate",
            &FPS.to_string(),
            "-i",
            out_dir.join("frame_%04d.ppm").to_str().unwrap(),
            "-pix_fmt",
            "yuv420p",
        ]);
        if let Some(crf) = crf {
            cmd.args(["-crf", &crf.to_string()]);
        }
        let status = cmd
            .arg(mp4_path.to_str().unwrap())
            .status()
            .expect("failed to spawn ffmpeg");
        if status.success() {
            println!("encoded video: {}", mp4_path.display());
            if !keep_frames {
                remove_frames(&out_dir);
            }
        } else {
            eprintln!("ffmpeg exited with {status}");
        }
    } else {
        println!(
            "ffmpeg not found on PATH — leaving raw .ppm frames in {}",
            out_dir.display()
        );
    }
}

/// Elements `JULIA_FREEZE` can hold still.
const FREEZABLE: [&str; 11] = [
    "jitter",
    "beat",
    "beam_size",
    "c_jumps",
    "bloom",
    "fold",
    "flow",
    "wobble",
    "palette",
    "drive",
    "pan",
];

fn remove_frames(dir: &std::path::Path) {
    for entry in std::fs::read_dir(dir).expect("read output dir") {
        let path = entry.expect("read output dir entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("frame_") && name.ends_with(".ppm") {
            std::fs::remove_file(&path).expect("remove frame");
        }
    }
}

/// The whole track's character: tempo (median of a fresh extractor's
/// confident hops) and its `energy` distribution, plus the onset rate and
/// level measured on the raw signal.
fn measure_track(mono: &[f32], sample_rate: f32, fft: usize, hop: usize) -> SongCharacter {
    let mut extractor = FeatureExtractor::new(fft, hop, sample_rate);
    let (mut energy, mut tempos) = (Vec::new(), Vec::new());
    for chunk in mono.chunks(hop) {
        for bus in extractor.push_samples(chunk) {
            energy.push(bus.energy);
            if bus.tempo_confidence > 0.2 {
                tempos.push(bus.tempo_bpm);
            }
        }
    }
    tempos.sort_by(f32::total_cmp);
    let tempo = tempos.get(tempos.len() / 2).copied().unwrap_or(120.0);
    // The extractor normalises level and flux adaptively, which hides how
    // loud and how busy a whole track is; measure those on the raw signal.
    let rms_db: Vec<f32> = mono
        .chunks_exact(hop)
        .map(|c| {
            let ms = c.iter().map(|s| s * s).sum::<f32>() / hop as f32;
            10.0 * (ms + 1e-12).log10()
        })
        .collect();
    let mut window = hyst_audio::dsp::fft::WindowedFft::new(fft);
    let mut mags = vec![0.0f32; window.bins];
    let mut prev = vec![0.0f32; window.bins];
    let mut flux = Vec::new();
    for start in (0..mono.len().saturating_sub(fft)).step_by(hop) {
        window.transform(&mono[start..start + fft], &mut mags);
        let mut sum = 0.0f32;
        for (m, p) in mags.iter_mut().zip(prev.iter_mut()) {
            // Log magnitude so quiet and loud hits count alike.
            *m = (100.0 * *m).ln_1p();
            sum += (*m - *p).max(0.0);
            *p = *m;
        }
        flux.push(sum);
    }
    let onsets = onset_rate(&flux, sample_rate / hop as f32);
    let features = TrackFeatures::measure(&rms_db, &energy, onsets, tempo);
    let character = SongCharacter::from_features(&features);
    println!("track features {features:?} -> {character:?}");
    character
}

struct SongState {
    reactor: Reactor,
    track: Option<SectionTrack>,
    /// Lissajous phase, integrated at a rate set by the song's calm.
    beam_phase: f32,
}

/// Fold edges in relative loudness: the fold only shows in the song's own
/// loudest passages.
const SONG_FOLD_EDGES: [f32; 2] = [0.85, 1.0];

/// The `after` mapping with JULIA_SONG=on: every fast reaction read through
/// the song's [`Reactor`], and the section's eased sub-look on top.
/// `clock` is `[t, dt, aspect]`.
fn song_frame_params(
    song: &mut SongState,
    driver: &mut CuratedJuliaDriver,
    structure: &mut EnergyStructure,
    bus: &hyst_core::SignalBus,
    clock: [f32; 3],
    beat_pulse: f32,
    hop_samples: &[f32],
) -> FrameParams {
    let [t, dt, aspect] = clock;
    let character = song.reactor.character;
    let react = character.react();
    let r = song.reactor.update(dt, bus);
    let blend = match &mut song.track {
        Some(track) => {
            let (blend, entry) = track.update(t, dt);
            driver.pin_entry(entry);
            blend
        }
        None => Blend {
            hue: 0.0,
            tint: [1.0; 3],
            decay: 0.0,
            beam: 1.0,
            jitter: 1.0,
            fold: 1.0,
        },
    };
    let mut drive_bus = bus.clone();
    drive_bus.energy = r.drive_energy;
    drive_bus.chroma_root_hue = r.chroma_hue;
    drive_bus.centroid = r.centroid;
    drive_bus.flatness = r.flatness;
    let mut julia = driver.update(dt, &drive_bus);
    julia.hue_shift = (julia.hue_shift + blend.hue).rem_euclid(1.0);
    for (a, k) in julia.accent.iter_mut().zip(blend.tint) {
        *a *= k;
    }
    structure.update(dt, bus.energy);
    song.beam_phase += dt * 0.6 * (1.0 - 0.5 * character.calm);

    let base = frame_params(bus, t, 0.0, julia, &[], aspect);
    let mut p = after_frame_params(base, bus, structure, react);
    p.memory_field.decay = p.memory_field.decay.max(blend.decay).clamp(0.0, 0.999);
    p.memory_field.flow_strength = 0.01 + 0.05 * r.sub;
    p.memory_field.mirror_strength =
        smoothstep(SONG_FOLD_EDGES[0], SONG_FOLD_EDGES[1], r.punch_energy) * 0.55 * blend.fold;
    p.bloom_strength = 0.15 + 0.25 * r.punch_energy;
    p.beam_segments = if blend.beam > 0.0 {
        lissajous_segments(
            song.beam_phase,
            r.beam_energy,
            hop_samples,
            JITTER_AMP * react * blend.jitter,
        )
    } else {
        Vec::new()
    };
    p.beam_intensity = (0.5 + 2.0 * react * beat_pulse) * blend.beam;
    p
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn which_ffmpeg() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("ffmpeg"))
            .find(|full_path| full_path.is_file())
    })
}

/// Small enough that the base Lissajous shape stays the dominant, easily
/// recognizable read; large enough that the jitter is actually visible as
/// a real trace wobble rather than disappearing into the beam's own
/// ~0.012 half-width.
const JITTER_AMP: f32 = 0.03;

/// A 3:2 Lissajous trace (`src/render/worker/scenes/julia/lissajous.ts` —
/// the real design intent for the beam's idle path), phase-drifting with
/// `t`, amplitude breathing with `energy`, base shape kept exactly as
/// before (user: "keep the current lissajous shape").
///
/// **Oscilloscope jitter (Task 4)**: on top of that unchanged base curve,
/// each point gets a small *perpendicular* displacement driven by the
/// track's own real per-hop mono samples (`hop_samples`, the demo's own raw
/// sample chunk for this hop — the most real/available signal, actual
/// waveform values rather than an amplitude-only stand-in), mapped one
/// sample per curve point (nearest-index resample from `HOP_SIZE` samples
/// down to `POINTS`). Perpendicular, not radial or tangential, because
/// that's the actual look of a real oscilloscope trace riding a Lissajous
/// path — the beam wobbles off the ideal curve rather than breathing
/// uniformly or bunching along it. Displacement is the tangent's normal
/// (analytic derivative of the parametric curve, not a finite-difference
/// approximation) scaled by a small fixed amount so the underlying shape
/// stays legible per the user's explicit ask.
fn lissajous_segments(
    phase: f32,
    energy: f32,
    hop_samples: &[f32],
    jitter_amp: f32,
) -> Vec<Segment> {
    // Segments only, no joint geometry (renderer.rs reverted that — see
    // its comment). High point count is safe and correct here: it just
    // shrinks the bend angle between consecutive segments until a flat
    // cap's notch is genuinely imperceptible, without any bead-chain risk
    // (that risk was specific to round joint discs, not segment density).
    const POINTS: usize = 220;
    let amp = 0.35 + 0.35 * energy;
    let point = |theta: f32| -> [f32; 2] {
        [
            amp * (3.0 * theta + phase).sin(),
            amp * (2.0 * theta + phase * 0.5).sin(),
        ]
    };
    // Analytic d/dtheta of `point` above, used to build the curve's own
    // perpendicular (normal) direction at each theta.
    let tangent = |theta: f32| -> [f32; 2] {
        [
            amp * 3.0 * (3.0 * theta + phase).cos(),
            amp * 2.0 * (2.0 * theta + phase * 0.5).cos(),
        ]
    };
    let sample_at = |i: usize| -> f32 {
        if hop_samples.is_empty() {
            0.0
        } else {
            let idx = i * hop_samples.len() / POINTS;
            hop_samples[idx.min(hop_samples.len() - 1)]
        }
    };
    let wobbled = |i: usize, theta: f32| -> [f32; 2] {
        let base = point(theta);
        let tang = tangent(theta);
        let tang_len = (tang[0] * tang[0] + tang[1] * tang[1]).sqrt().max(1e-6);
        let normal = [-tang[1] / tang_len, tang[0] / tang_len];
        let s = sample_at(i);
        [
            base[0] + normal[0] * jitter_amp * s,
            base[1] + normal[1] * jitter_amp * s,
        ]
    };
    (0..POINTS)
        .map(|i| {
            let theta0 = i as f32 / POINTS as f32 * std::f32::consts::TAU;
            let theta1 = (i + 1) as f32 / POINTS as f32 * std::f32::consts::TAU;
            Segment {
                p0: wobbled(i, theta0),
                p1: wobbled(i + 1, theta1),
            }
        })
        .collect()
}

/// `sub` drives memory-field decay/flow (a bassier moment holds and moves
/// the feedback field more); `energy` drives bloom strength and the beam's
/// Lissajous amplitude; `beat_pulse` drives beam brightness; `julia` is
/// this frame's already-computed `JuliaDriver::update` output (the
/// substrate's own audio-reactive zoom/orbit/color — see that struct's
/// doc). Just enough real-signal wiring to prove the pipeline responds to
/// audio, not a final mix.
fn frame_params(
    bus: &hyst_core::SignalBus,
    t: f32,
    beat_pulse: f32,
    julia: hyst_render::passes::julia::JuliaNavState,
    hop_samples: &[f32],
    aspect: f32,
) -> FrameParams {
    FrameParams {
        julia,
        memory_field: MemoryFieldParams {
            // Lower baseline than the first cut (was 0.85-0.95): trail
            // length scales like 1/(1-decay), so held-near-max decay for a
            // full 45s compounded into an ever-softening blur. 0.55-0.75
            // still gives real memory/smear, just shorter-lived.
            decay: 0.55 + 0.2 * bus.sub,
            aspect,
            flow_uv: [t * 0.03, 0.0],
            flow_scale: 2.0,
            flow_strength: 0.01 + 0.05 * bus.sub,
            fold_count: 4.0,
            // "Earned symmetry": user feedback on the first cut (0.15+0.5*
            // energy) was "too frequent" — energy is above the old 0.3
            // threshold a lot of a real track, so symmetry was visible
            // most of the time, not rare/earned. Gate it: near-zero below
            // 0.65 energy, ramping only in the loudest moments.
            mirror_strength: smoothstep(0.65, 0.9, bus.energy) * 0.55,
        },
        persistence_decay: 0.8,
        // Task 3: real luminance-histogram stats taken this session (256x256
        // preview, full pipeline, 90 synthetic frames) found the old
        // threshold=0.3/strength=0.4-1.2/exposure=0.08 combo left a floor of
        // 38/255 (15%) that never dropped for the whole clip — bloom
        // spreading from nearly every pixel above a low threshold, compounded
        // by the memory field's documented fixed ~7x steady-state gain,
        // reads as a persistent haze over everything ("washed/murky").
        // Raising the threshold (fewer, genuinely-bright pixels feed bloom),
        // lowering bloom_strength's baseline, and lowering exposure measured
        // the floor down to 24/255 in the same synthetic probe — a real,
        // measured improvement, not a guess.
        bloom_threshold: 0.55,
        bloom_strength: 0.2 + 0.5 * bus.energy,
        exposure: 0.035,
        beam_segments: lissajous_segments(t * 0.6, bus.energy, hop_samples, JITTER_AMP),
        beam_color: [1.0, 1.0, 1.0],
        beam_intensity: 0.5 + 2.0 * beat_pulse,
    }
}

/// Stand-in for the TS build and break detectors: two energy envelopes
/// whose gap says whether energy is climbing (build) or has fallen away
/// (suspension). Both are plain EMAs of a bounded value, not phases.
#[derive(Default)]
struct EnergyStructure {
    mid: f32,
    long: f32,
}

impl EnergyStructure {
    fn update(&mut self, dt: f32, energy: f32) {
        let e = energy.clamp(0.0, 1.0);
        self.mid += (e - self.mid) * (dt / 2.0).min(1.0);
        self.long += (e - self.long) * (dt / 20.0).min(1.0);
    }

    fn build(&self) -> f32 {
        smoothstep(0.02, 0.12, self.mid - self.long)
    }

    fn suspension(&self) -> f32 {
        smoothstep(0.05, 0.25, self.long - self.mid)
    }
}

const AFTER_EXPOSURE: f32 = 0.12;
const AFTER_BLOOM_THRESHOLD: f32 = 8.0;

/// The `after` mapping over the `before` one. Decay is the TS formula over
/// [`EnergyStructure`]; exposure and the bloom threshold are in field
/// units, where a steadily lit pixel sits near `1 / (1 - 0.86)` (about 7)
/// whatever the decay.
fn after_frame_params(
    mut params: FrameParams,
    bus: &hyst_core::SignalBus,
    structure: &EnergyStructure,
    wobble: f32,
) -> FrameParams {
    let bar_wobble = (bus.bar_phase * std::f32::consts::TAU).sin() * 0.01 * wobble;
    let decay = ((0.86 + 0.105 * structure.build()).max(0.86 + 0.125 * structure.suspension())
        + bar_wobble)
        .clamp(0.0, 0.999);
    params.memory_field.decay = decay;
    params.bloom_threshold = AFTER_BLOOM_THRESHOLD;
    params.bloom_strength = 0.15 + 0.25 * bus.energy;
    params.exposure = AFTER_EXPOSURE;
    params
}

fn write_ppm(path: &std::path::Path, rgba: &[u8], width: u32, height: u32) {
    let mut file = std::fs::File::create(path).expect("create ppm frame");
    write!(file, "P6\n{width} {height}\n255\n").unwrap();
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for px in rgba.chunks(4) {
        rgb.extend_from_slice(&px[0..3]);
    }
    file.write_all(&rgb).unwrap();
}
