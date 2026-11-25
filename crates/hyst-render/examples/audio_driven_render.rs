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
//! `SCENE=grid|growth|feedback|auto` renders the non-fractal sketch scenes
//! instead (see `run_scenes`); with `SCENE` unset nothing above changes.
//! `SCENE=avproof` renders kabl's av_proof look from the sidecar's lanes,
//! with its text overlay; `SCENE_FPS` (default 30) sets the frame rate.

use std::io::Write;
use std::path::PathBuf;

use hyst_audio::{FeatureExtractor, WavSource};
use hyst_render::passes::beam::Segment;
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
    if let Ok(scene) = std::env::var("SCENE") {
        run_scenes(&scene);
        return;
    }
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
    for entry in std::fs::read_dir(&out_dir).expect("read output dir") {
        let path = entry.expect("read output dir entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("frame_") && name.ends_with(".ppm") {
            std::fs::remove_file(&path).expect("remove stale frame");
        }
    }

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
            beat_pulse *= 0.85;

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
                    Look::After => {
                        let julia = curated_driver.update(dt, &bus);
                        structure.update(dt, bus.energy);
                        let base = frame_params(&bus, t, beat_pulse, julia, chunk, aspect);
                        after_frame_params(base, &bus, &structure)
                    }
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
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-framerate",
                &FPS.to_string(),
                "-i",
                out_dir.join("frame_%04d.ppm").to_str().unwrap(),
                "-pix_fmt",
                "yuv420p",
                mp4_path.to_str().unwrap(),
            ])
            .status()
            .expect("failed to spawn ffmpeg");
        if status.success() {
            println!("encoded video: {}", mp4_path.display());
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

/// `SCENE=grid|growth|feedback|auto`: the sketch scenes instead of Julia.
/// Everything musical comes from the offline sidecar (`SCENE_SIDECAR`,
/// required), whose beats, downbeats, envelopes and onsets drive the scenes;
/// `auto` plays the switcher over the sidecar's sections. Frames sit on the
/// track's clock (`SCENE_START + i / FPS`) and are piped straight to ffmpeg,
/// which muxes the same span of the WAV, so no frames land on disk.
///
/// `SCENE_START` (s, default 0), `SCENE_SECONDS` (default: to the end),
/// `SCENE_WIDTH`/`SCENE_HEIGHT` (default 960x540), `SCENE_CRF` (default 16),
/// `SCENE_OUT` (default `test-output/scene_<scene>.mp4`).
fn run_scenes(scene: &str) {
    use hyst_render::scenes::avproof::{self, AvProofScene};
    use hyst_render::scenes::{switcher, SceneKind, SceneRenderer, SongTrack};

    let avproof = scene == "avproof";
    let fixed = match scene {
        "auto" | "avproof" => None,
        name => Some(SceneKind::from_name(name).unwrap_or_else(|| {
            panic!("SCENE must be grid, growth, feedback, auto or avproof, got {name:?}")
        })),
    };
    let wav_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("SCENE mode needs the WAV path as the first argument");
    let sidecar_path = std::env::var("SCENE_SIDECAR").expect("SCENE mode needs SCENE_SIDECAR");
    let json = std::fs::read_to_string(&sidecar_path)
        .unwrap_or_else(|e| panic!("read {sidecar_path}: {e}"));
    let sidecar = hyst_core::Sidecar::from_json_str(&json)
        .unwrap_or_else(|e| panic!("parse {sidecar_path}: {e}"));
    let width: u32 = env_or("SCENE_WIDTH", 960);
    let height: u32 = env_or("SCENE_HEIGHT", 540);
    assert!(
        width > 0 && height > 0 && width.is_multiple_of(64),
        "SCENE_WIDTH must be a positive multiple of 64 and SCENE_HEIGHT positive, got {width}x{height}"
    );
    let start: f32 = env_or("SCENE_START", 0.0);
    let duration = sidecar.duration as f32;
    let seconds: f32 = env_or("SCENE_SECONDS", duration - start);
    assert!(
        start >= 0.0 && seconds > 0.0 && start + seconds <= duration + 0.5,
        "SCENE_START {start} + SCENE_SECONDS {seconds} must lie inside the track ({duration} s)"
    );
    let crf: u32 = env_or("SCENE_CRF", 16);
    let fps: f32 = env_or("SCENE_FPS", FPS);
    assert!(fps > 0.0, "SCENE_FPS must be positive, got {fps}");
    let out = std::env::var_os("SCENE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("test-output")
                .join(format!("scene_{scene}.mp4"))
        });
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).expect("create output dir");
    }

    let sections = switcher::sections_from_sidecar(&sidecar);
    if fixed.is_none() && !avproof {
        println!("section -> scene");
        for (i, s) in sections.iter().enumerate() {
            println!(
                "  {i:2}  {:6.1}-{:6.1}  {:?}  level {:.2}  {}{}",
                s.start,
                s.end,
                s.kind,
                s.level,
                s.scene.name(),
                s.repeats
                    .map(|j| format!("  (repeats {j})"))
                    .unwrap_or_default()
            );
        }
    }

    let vf = if avproof {
        avproof_text(&sidecar, &wav_path, height, start, seconds)
    } else {
        "null".to_string()
    };
    let mut ffmpeg = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{width}x{height}"), "-r", &fps.to_string()])
        .args([
            "-i",
            "-",
            "-ss",
            &start.to_string(),
            "-t",
            &seconds.to_string(),
        ])
        .arg("-i")
        .arg(&wav_path)
        .args([
            "-map", "0:v", "-map", "1:a", "-vf", &vf, "-c:v", "libx264", "-preset", "medium",
        ])
        .args(["-crf", &crf.to_string(), "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "160k", "-shortest"])
        .arg(&out)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn ffmpeg");
    let mut pipe = ffmpeg.stdin.take().expect("ffmpeg stdin");

    let ctx = GpuContext::new_blocking().expect("no GPU adapter available — cannot render");
    let mut renderer = SceneRenderer::new(&ctx, width, height);
    let mut song = SongTrack::new(&sidecar);
    let dt = 1.0 / fps;
    let frames = (seconds * fps).round() as usize;
    let mut av = avproof.then(|| {
        let wav = WavSource::load(&wav_path).expect("failed to load WAV");
        let ch = wav.channels as usize;
        let mono: Vec<f32> = wav
            .samples
            .chunks(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect();
        let scene = AvProofScene::new(&ctx, width, height, &sidecar, wav.sample_rate as f32);
        (scene, mono, wav.sample_rate as f32)
    });
    for i in 0..frames {
        let t = start + i as f32 * dt;
        let inputs = song.advance(t, dt);
        if let Some((scene, mono, rate)) = &mut av {
            let end = ((t * *rate) as usize).min(mono.len());
            let scope = &mono[end.saturating_sub(avproof::SCOPE)..end];
            let pixels = scene
                .render(&ctx, &inputs, scope)
                .expect("avproof render failed");
            pipe.write_all(&pixels).expect("write frame to ffmpeg");
            if i % 300 == 0 {
                println!("frame {i}/{frames} (t {t:.1} s)");
            }
            continue;
        }
        let weights = match fixed {
            Some(kind) => {
                let mut w = [0.0; 3];
                w[kind as usize] = 1.0;
                w
            }
            None => switcher::weights_at(&sections, t),
        };
        let pixels = renderer
            .render(&ctx, &inputs, weights)
            .expect("scene render failed");
        pipe.write_all(&pixels).expect("write frame to ffmpeg");
        if i % 300 == 0 {
            println!("frame {i}/{frames} (t {t:.1} s)");
        }
    }
    drop(pipe);
    let status = ffmpeg.wait().expect("wait for ffmpeg");
    assert!(status.success(), "ffmpeg exited with {status}");
    println!("encoded {frames} frames to {}", out.display());
}

/// av_proof's text overlay as an ffmpeg filter: a muted line naming the
/// track and what drives the picture, and a label for each named cue. Times
/// are relative to the clip; sizes are av_proof's at 1080 lines, scaled.
fn avproof_text(
    sidecar: &hyst_core::Sidecar,
    wav: &std::path::Path,
    height: u32,
    start: f32,
    secs: f32,
) -> String {
    const MONO_FONT: &str = "/usr/share/fonts/truetype/ibm-plex/IBMPlexMono-Regular.ttf";
    assert!(
        std::path::Path::new(MONO_FONT).is_file(),
        "avproof's text needs {MONO_FONT}"
    );
    let fs = |x: f32| (x * height as f32 / 1080.0) as u32;
    let window = |a: f32, b: f32| {
        format!("if(lt(t,{a}),0,if(lt(t,{a}+0.6),(t-{a})/0.6,if(lt(t,{b}-0.8),1,if(lt(t,{b}),({b}-t)/0.8,0))))")
    };
    let text = |s: &str, size: u32, x: &str, y: &str, alpha: String, col: &str| {
        let s = s.replace(['\'', ':', '\\', '%'], "");
        format!("drawtext=fontfile={MONO_FONT}:text='{s}':fontsize={size}:fontcolor={col}:x={x}:y={y}:alpha='{alpha}'")
    };
    let mut vf = Vec::new();
    let cues = hyst_render::scenes::avproof::cues(sidecar);
    for (k, cue) in cues.iter().enumerate() {
        let Some(name) = &cue.label else { continue };
        let a = (cue.start - start).max(0.0);
        let next = cues.get(k + 1).map_or(secs, |n| n.start - start);
        // A cue that ended before the clip starts gets no label.
        if next <= 0.0 || a >= secs {
            continue;
        }
        let b = next.max(a + 2.0).min(a + 5.0);
        vf.push(text(
            &format!("cue {}  ·  {name}", k + 1),
            fs(30.0),
            &fs(56.0).to_string(),
            &format!("h-{}", fs(72.0)),
            window(a, b),
            "white@0.85",
        ));
    }
    let track = wav
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    vf.push(text(
        &format!(
            "track  {track}   ·   offline render   ·   visuals from notes, beats, bars, sections"
        ),
        fs(18.0),
        &format!("w-text_w-{}", fs(48.0)),
        &fs(40.0).to_string(),
        window(0.0, secs + 1.0),
        "white@0.45",
    ));
    vf.join(",")
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
fn lissajous_segments(t: f32, energy: f32, hop_samples: &[f32]) -> Vec<Segment> {
    // Segments only, no joint geometry (renderer.rs reverted that — see
    // its comment). High point count is safe and correct here: it just
    // shrinks the bend angle between consecutive segments until a flat
    // cap's notch is genuinely imperceptible, without any bead-chain risk
    // (that risk was specific to round joint discs, not segment density).
    const POINTS: usize = 220;
    // Small enough that the base Lissajous shape stays the dominant, easily
    // recognizable read; large enough that the jitter is actually visible as
    // a real trace wobble rather than disappearing into the beam's own
    // ~0.012 half-width.
    const JITTER_AMP: f32 = 0.03;
    let amp = 0.35 + 0.35 * energy;
    let phase = t * 0.6;
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
            base[0] + normal[0] * JITTER_AMP * s,
            base[1] + normal[1] * JITTER_AMP * s,
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
        beam_segments: lissajous_segments(t, bus.energy, hop_samples),
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
) -> FrameParams {
    let bar_wobble = (bus.bar_phase * std::f32::consts::TAU).sin() * 0.01;
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
