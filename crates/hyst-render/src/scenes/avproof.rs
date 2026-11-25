//! kabl's `av_proof` look (`tools/examples/av_proof.rs` in the kabl repo),
//! ported to the GPU and driven by the song's own notes.
//!
//! The layout, colours, line weights, feedback, bloom and tonemap follow
//! av_proof's CPU drawing. Its per-frame constants were written for 60 fps;
//! here they are raised to `dt * 60` so any frame rate moves the same way.
//! Geometry is placed in av_proof's 1080-line pixel units and drawn at twice
//! the output size below 1080 lines, then averaged down.
//!
//! What drives each element (kabl's engine taps -> the sidecar):
//! - step rows: one per DAW lane that plays; a cell lights on each note-on
//!   for the note's length, brightness by velocity, then keeps a fading
//!   memory. Cells sit on the bar's sixteenths (beats and downbeats).
//! - playhead cell: the current sixteenth (clock ticks from `beats`, bars
//!   from `downbeats`).
//! - beams and arcs: from each note-on to the core; arcs join lanes that
//!   fire in the same frame. A pitched note's beam ends on the spectrum
//!   ring's line for that pitch.
//! - core pulse and shockwave: the kick lane (else the beats).
//! - envelopes (core size, burst triggers): an attack/release follower of
//!   the pitched lanes' held notes.
//! - lead comet: the lead lane's held note.
//! - palette, flash and section rings: section boundaries.
//! - waveform core and spectrum ring: the WAV.
//! - macros: `energy` is the song's smoothed loudness; `motion`, `space`
//!   and `glow` stay at kabl's default 0.5; the LFO is a sine over four bars.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use hyst_core::sidecar::Sidecar;
use wgpu::util::DeviceExt;

use crate::gpu::{GpuContext, GpuError, OffscreenTarget};

use super::music::{grid_count, SceneInputs};
use super::{glsl_header, run_pass};

/// Scene palettes; a section change moves to the next one with a smooth blend.
const SCENES: [[[f32; 3]; 4]; 5] = [
    [
        [1.00, 0.52, 0.14],
        [0.18, 0.72, 1.00],
        [1.00, 0.86, 0.50],
        [0.85, 0.30, 0.75],
    ], // amber/cyan
    [
        [1.00, 0.24, 0.48],
        [0.38, 0.40, 1.00],
        [1.00, 0.72, 0.40],
        [0.30, 1.00, 0.85],
    ], // rose/indigo
    [
        [0.40, 1.00, 0.55],
        [0.15, 0.55, 1.00],
        [0.95, 1.00, 0.60],
        [1.00, 0.45, 0.25],
    ], // lime/blue
    [
        [0.70, 0.45, 1.00],
        [1.00, 0.40, 0.30],
        [0.60, 0.95, 1.00],
        [1.00, 0.85, 0.35],
    ], // violet/coral
    [
        [1.00, 0.80, 0.35],
        [1.00, 0.35, 0.20],
        [0.55, 0.85, 1.00],
        [0.90, 0.90, 0.95],
    ], // gold/ember
];

/// Above this many lanes the rows get too thin to read; the quietest go.
pub const MAX_ROWS: usize = 12;
/// Samples of the mix the waveform core and spectrum read.
pub const SCOPE: usize = 2048;
/// Release time of the note envelope follower, seconds.
const ENV_RELEASE: f32 = 0.25;
/// A released cell keeps this share of its velocity as a fading memory.
const MEMORY: f32 = 0.5;

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// One cue: a span between section boundaries, with the label of the
/// section that starts there.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f32,
    pub label: Option<String>,
}

/// The cues of a sidecar: every section start and end inside the track is a
/// boundary.
pub fn cues(sidecar: &Sidecar) -> Vec<Cue> {
    let duration = sidecar.duration as f32;
    let mut bounds: Vec<f32> = sidecar
        .sections
        .iter()
        .flat_map(|s| [s.start as f32, s.end as f32])
        .filter(|&b| b > 0.05 && b < duration - 0.05)
        .collect();
    bounds.sort_by(f32::total_cmp);
    bounds.dedup_by(|a, b| (*a - *b).abs() < 0.05);
    let label_at = |t: f32| {
        sidecar
            .sections
            .iter()
            .find(|s| (s.start as f32 - t).abs() < 0.05)
            .and_then(|s| s.label.clone())
    };
    std::iter::once(0.0)
        .chain(bounds)
        .map(|start| Cue {
            start,
            label: label_at(start),
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct Note {
    t: f32,
    end: f32,
    pitch: u8,
    vel: f32,
}

struct Row {
    notes: Vec<Note>,
    longest: f32,
    pitched: bool,
    kick: bool,
    lead: bool,
    heat: [f32; 16],
    /// Velocity of the note held now, per cell.
    held: [f32; 16],
    /// Velocity of the loudest note held now, 0 when none.
    lit: f32,
    env: f32,
}

impl Row {
    fn new(name: &str, mut notes: Vec<Note>) -> Self {
        notes.sort_by(|a, b| a.t.total_cmp(&b.t));
        let longest = notes.iter().map(|n| n.end - n.t).fold(0.0, f32::max);
        let first = notes.first().map(|n| n.pitch);
        let lower = name.to_lowercase();
        Self {
            pitched: notes.iter().any(|n| Some(n.pitch) != first),
            kick: lower.contains("kick") || lower.contains("generator"),
            lead: lower.contains("lead") || lower.contains("main synth"),
            longest,
            notes,
            heat: [0.0; 16],
            held: [0.0; 16],
            lit: 0.0,
            env: 0.0,
        }
    }

    /// The notes that start during the frame ending at `t`.
    fn onsets(&self, t: f32, dt: f32) -> &[Note] {
        let lo = self.notes.partition_point(|n| n.t <= t - dt);
        let hi = self.notes.partition_point(|n| n.t <= t);
        &self.notes[lo..hi]
    }

    /// Notes sounding at `t`; a note shorter than a frame sounds for the
    /// frame its note-on falls in.
    fn held_at(&self, t: f32, dt: f32) -> impl Iterator<Item = &Note> {
        let hi = self.notes.partition_point(|n| n.t <= t);
        let lo = self
            .notes
            .partition_point(|n| n.t < t - self.longest - dt - 1e-3);
        self.notes[lo..hi]
            .iter()
            .filter(move |n| t < n.end || n.t > t - dt)
    }
}

struct Ring {
    x: f32,
    y: f32,
    born: f32,
    col: [f32; 3],
    a: f32,
    max_r: f32,
    life: f32,
}

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    col: [f32; 3],
}

/// One additive primitive, in pixels: a soft dot, a line of soft dots, or a
/// soft-edged rectangle.
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Prim {
    /// Dot: centre, unused. Line: both ends. Rect: centre, half size.
    seg: [f32; 4],
    /// Colour times weight, and the Gaussian's `k2`.
    col: [f32; 4],
    /// Kind (0 dot, 1 line, 2 rect), reach in pixels, unused, unused.
    kind: [f32; 4],
}

type Pt = (f32, f32);

#[derive(Default)]
struct Layers {
    /// Drawn into the feedback buffer.
    acc: Vec<Prim>,
    /// Drawn fresh each frame.
    crisp: Vec<Prim>,
}

impl Layers {
    fn get(&mut self, crisp: bool) -> &mut Vec<Prim> {
        if crisp {
            &mut self.crisp
        } else {
            &mut self.acc
        }
    }

    /// av_proof's `dot`: `a * exp(-k2 d^2)` over a `(2r+1)^2` window.
    fn dot(&mut self, crisp: bool, p: Pt, c: [f32; 3], a: f32, r: u32) {
        let k2 = if r <= 1 { 1.6 } else { 0.7 };
        self.get(crisp).push(Prim {
            seg: [p.0, p.1, 0.0, 0.0],
            col: [c[0] * a, c[1] * a, c[2] * a, k2],
            kind: [0.0, r as f32 + 0.5, 0.0, 0.0],
        });
    }

    /// av_proof's `line`: dots every 0.9 px or closer, both ends included,
    /// drawn as their continuous sum at the same density.
    fn line(&mut self, crisp: bool, p0: Pt, p1: Pt, c: [f32; 3], a: f32) {
        let len = ((p1.0 - p0.0).powi(2) + (p1.1 - p0.1).powi(2)).sqrt();
        if len < 1e-3 {
            self.dot(crisp, p0, c, 2.0 * a, 1);
            return;
        }
        let n = (len / 0.9).ceil().max(1.0);
        let w = a * (n + 1.0) / len;
        self.get(crisp).push(Prim {
            seg: [p0.0, p0.1, p1.0, p1.1],
            col: [c[0] * w, c[1] * w, c[2] * w, 1.6],
            kind: [1.0, 1.5, 0.0, 0.0],
        });
    }

    fn rect(&mut self, centre: Pt, half: Pt, c: [f32; 3], a: f32) {
        self.crisp.push(Prim {
            seg: [centre.0, centre.1, half.0, half.1],
            col: [c[0] * a, c[1] * a, c[2] * a, 0.0],
            kind: [2.0, 2.0, 0.0, 0.0],
        });
    }
}

/// The feedback warp for this frame, already scaled to `dt`.
#[derive(Clone, Copy, Debug)]
struct Warp {
    zoom: f32,
    turn: f32,
    twist: f32,
    decay: f32,
    hue: f32,
}

/// av_proof's `Visual` minus the pixels: the musical state and what to draw.
pub struct AvProof {
    w: f32,
    h: f32,
    rows: Vec<Row>,
    /// No kick lane: the beats pulse the core instead.
    beats_pulse: bool,
    beats: Vec<f32>,
    downbeats: Vec<f32>,
    cues: Vec<f32>,
    sample_rate: f32,
    rings: Vec<Ring>,
    parts: Vec<Particle>,
    spec: [f32; 40],
    pal: [[f32; 3]; 4],
    scene: usize,
    spin: f32,
    flash: f32,
    punch: f32,
    rng: u64,
    lead_ang: f32,
    lead_lvl: f32,
    lead_note: f32,
    started: bool,
    layers: Layers,
    warp: Warp,
}

impl AvProof {
    /// `w`/`h` are the drawing size in pixels. Prints to stderr what it
    /// does with the sidecar's lanes.
    pub fn new(sidecar: &Sidecar, w: u32, h: u32, sample_rate: f32) -> Self {
        let beats: Vec<f32> = sidecar.beats.iter().map(|&b| b as f32).collect();
        let mut rows: Vec<(String, Row)> = match &sidecar.lanes {
            Some(lanes) => lanes
                .iter()
                .filter(|l| !l.events.is_empty())
                .map(|l| {
                    let notes = l
                        .events
                        .iter()
                        .map(|e| Note {
                            t: e.t as f32,
                            end: (e.t + e.length.max(0.0)) as f32,
                            pitch: e.pitch,
                            vel: e.velocity as f32 / 127.0,
                        })
                        .collect();
                    (l.name.clone(), Row::new(&l.name, notes))
                })
                .collect(),
            None => {
                eprintln!("avproof: the sidecar has no lanes; one step row follows the beats");
                let notes = beats
                    .iter()
                    .map(|&t| Note {
                        t,
                        end: t,
                        pitch: 60,
                        vel: 1.0,
                    })
                    .collect();
                vec![("beats".to_string(), Row::new("beats", notes))]
            }
        };
        if rows.len() > MAX_ROWS {
            // The kick and the lead keep their rows; then the busiest lanes.
            let mut order: Vec<usize> = (0..rows.len()).collect();
            order.sort_by_key(|&i| {
                let r = &rows[i].1;
                (
                    std::cmp::Reverse(r.kick || r.lead),
                    std::cmp::Reverse(r.notes.len()),
                    i,
                )
            });
            let keep = &order[..MAX_ROWS];
            let dropped: Vec<String> = (0..rows.len())
                .filter(|i| !keep.contains(i))
                .map(|i| format!("{} ({} notes)", rows[i].0, rows[i].1.notes.len()))
                .collect();
            eprintln!(
                "avproof: {} lanes, at most {MAX_ROWS} rows; dropped {}",
                rows.len(),
                dropped.join(", ")
            );
            let mut i = 0;
            let keep = keep.to_vec();
            rows.retain(|_| {
                i += 1;
                keep.contains(&(i - 1))
            });
        }
        // Only the first kick and lead lanes count as such.
        let (mut kick_seen, mut lead_seen) = (false, false);
        for (_, r) in &mut rows {
            r.kick &= !kick_seen;
            kick_seen |= r.kick;
            r.lead &= !lead_seen;
            lead_seen |= r.lead;
        }
        if !lead_seen {
            if let Some((_, r)) = rows
                .iter_mut()
                .filter(|(_, r)| r.pitched)
                .max_by_key(|(_, r)| r.notes.len())
            {
                r.lead = true;
            }
        }
        for (name, r) in &rows {
            let role = match (r.kick, r.lead, r.pitched) {
                (true, _, _) => "; kick: core pulse and shockwave",
                (_, true, _) => "; pitched, lead comet",
                (_, _, true) => "; pitched",
                _ => "",
            };
            eprintln!("avproof: row {name:?}, {} notes{role}", r.notes.len());
        }
        let beats_pulse = !kick_seen;
        if beats_pulse {
            eprintln!("avproof: no kick lane; the beats pulse the core");
        }
        let downbeats = match &sidecar.downbeats {
            Some(d) if d.len() >= 2 => d.iter().map(|&t| t as f32).collect(),
            _ => Vec::new(),
        };
        Self {
            w: w as f32,
            h: h as f32,
            rows: rows.into_iter().map(|(_, r)| r).collect(),
            beats_pulse,
            beats,
            downbeats,
            cues: cues(sidecar).iter().map(|c| c.start).collect(),
            sample_rate,
            rings: Vec::new(),
            parts: Vec::new(),
            spec: [0.0; 40],
            pal: SCENES[0],
            scene: 0,
            spin: 0.0,
            flash: 0.0,
            punch: 0.0,
            rng: 0x9E37_79B9_7F4A_7C15,
            lead_ang: 0.0,
            lead_lvl: 0.0,
            lead_note: 64.0,
            started: false,
            layers: Layers::default(),
            warp: Warp {
                zoom: 1.0,
                turn: 0.0,
                twist: 0.0,
                decay: 0.0,
                hue: 0.0,
            },
        }
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// The bar's sixteenth at `t`: beats count the steps, downbeats reset
    /// them to zero.
    fn column(&self, t: f32) -> usize {
        let beat = grid_count(&self.beats, t, 0.5);
        let i = self.downbeats.partition_point(|&d| d <= t + 1e-4);
        let into_bar = if i == 0 {
            beat
        } else {
            beat - grid_count(&self.beats, self.downbeats[i - 1], 0.5)
        };
        ((into_bar * 4.0 + 1e-3).floor() as i64).rem_euclid(16) as usize
    }

    fn cue_at(&self, t: f32) -> usize {
        self.cues.partition_point(|&c| c <= t).saturating_sub(1)
    }

    fn spectrum(&mut self, scope: &[f32], k: f32) {
        let n = scope.len();
        if n == 0 {
            return;
        }
        let (rise, fall) = (1.0 - 0.4f32.powf(k), 0.88f32.powf(k));
        for b in 0..40 {
            let f = 55.0 * (12000.0f32 / 55.0).powf(b as f32 / 39.0);
            let coef = 2.0 * (TAU * f / self.sample_rate).cos();
            let (mut s1, mut s2) = (0f32, 0f32);
            for (i, &x) in scope.iter().enumerate() {
                let win = 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos();
                let s0 = x * win + coef * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            let p = (s1 * s1 + s2 * s2 - coef * s1 * s2).max(1e-12);
            let db = 10.0 * (p / (n as f32 * n as f32 * 0.25)).log10() + (b as f32 * 0.25);
            let v = ((db + 62.0) / 52.0).clamp(0.0, 1.0);
            let s = &mut self.spec[b];
            *s = if v > *s {
                *s + (v - *s) * rise
            } else {
                *s * fall + v * (1.0 - fall)
            };
        }
    }

    /// Advances to `inp.t` and lays out this frame's primitives. `scope` is
    /// the mono mix ending at `inp.t` (up to [`SCOPE`] samples).
    pub fn step(&mut self, inp: &SceneInputs, scope: &[f32]) {
        let (w, h) = (self.w, self.h);
        let (cx, cy) = (w * 0.5, h * 0.42);
        let u = h / 1080.0;
        let (t, dt) = (inp.t, inp.dt.max(1e-4));
        // av_proof's per-frame constants are for 60 fps.
        let k = dt * 60.0;
        let per_frame = |rate: f32| 1.0 - (1.0 - rate).powf(k);
        let (energy, motion, space, glow) = (inp.energy_slow, 0.5f32, 0.5f32, 0.5f32);
        let lfo = (TAU * inp.bar / 4.0).sin();
        let mut layers = std::mem::take(&mut self.layers);
        layers.acc.clear();
        layers.crisp.clear();

        // Note state: held velocity per lane and cell, and each lane's
        // envelope follower.
        let col_now = self.column(t);
        let mut attack = false;
        let mut row_hits: Vec<Vec<Note>> = Vec::with_capacity(self.rows.len());
        for ri in 0..self.rows.len() {
            let hits = self.rows[ri].onsets(t, dt).to_vec();
            let held: Vec<(usize, f32)> = self.rows[ri]
                .held_at(t, dt)
                .map(|n| (self.column(n.t), n.vel))
                .collect();
            let hit_cols: Vec<(usize, f32)> =
                hits.iter().map(|n| (self.column(n.t), n.vel)).collect();
            let row = &mut self.rows[ri];
            row.held = [0.0; 16];
            for &(c, v) in &held {
                row.held[c] = row.held[c].max(v);
            }
            for &(c, v) in &hit_cols {
                row.heat[c] = row.heat[c].max(v);
            }
            row.lit = held.iter().map(|&(_, v)| v).fold(0.0, f32::max);
            if row.pitched && hits.iter().any(|n| n.vel > 0.75) && row.env < 0.45 {
                attack = true;
            }
            row.env = if row.lit >= row.env {
                row.lit
            } else {
                row.env * (-dt / ENV_RELEASE).exp()
            };
            row_hits.push(hits);
        }
        let pitched = self.rows.iter().filter(|r| r.pitched).count();
        let env = self
            .rows
            .iter()
            .filter(|r| r.pitched)
            .map(|r| r.env)
            .sum::<f32>()
            / pitched.max(1) as f32;

        // Section change: next palette, a flash and a shockwave.
        let cue = self.cue_at(t);
        if !self.started {
            self.started = true;
            self.scene = cue % SCENES.len();
            self.pal = SCENES[self.scene];
        } else if cue != self.cue_at(t - dt) {
            self.scene = (self.scene + 1) % SCENES.len();
            self.flash = 1.0;
            for i in 0..3 {
                self.rings.push(Ring {
                    x: cx,
                    y: cy,
                    born: t + i as f32 * 0.09,
                    col: [1.0, 0.95, 0.85],
                    a: 1.4,
                    max_r: w * 0.7,
                    life: 1.8,
                });
            }
        }
        let target = SCENES[self.scene];
        for (p, &c) in self.pal.iter_mut().zip(&target) {
            *p = mix(*p, c, per_frame(0.025));
        }

        // The kick (or, without one, the beat): a core punch and a shockwave.
        let pulse = if self.beats_pulse {
            let lo = self.beats.partition_point(|&b| b <= t - dt);
            self.beats.partition_point(|&b| b <= t) > lo
        } else {
            self.rows
                .iter()
                .zip(&row_hits)
                .any(|(r, hits)| r.kick && !hits.is_empty())
        };
        if pulse {
            self.punch = 1.0;
            self.rings.push(Ring {
                x: cx,
                y: cy,
                born: t,
                col: self.pal[2],
                a: 0.22,
                max_r: h * 0.55,
                life: 0.7,
            });
        }
        self.punch *= (-9.0 * dt).exp();
        self.flash *= (-2.5 * dt).exp();
        self.spin += dt * (0.12 + 0.5 * motion + 0.4 * lfo);

        self.warp = Warp {
            zoom: (1.0035 + 0.006 * energy + 0.012 * self.punch).powf(k),
            turn: (0.0015 + 0.004 * lfo * motion) * k,
            twist: (0.010 * (motion - 0.3) + 0.004 * (t * 0.21).sin()) * k,
            decay: (0.88 + 0.085 * space).powf(k),
            hue: (0.004 + 0.01 * motion) * k,
        };

        // Core size, needed by the beams' ring.
        let base = h * (0.085 + 0.07 * env) * (1.0 + 0.08 * self.punch);
        let r0 = base * 1.55 + 8.0 * u;

        // Lattice: one row per lane, 16 cells, the bottom band.
        let n = self.rows.len();
        let row_h = (h * 0.22 / n.max(1) as f32).min(30.0 * u);
        let y_top = h * 0.855 - row_h * n as f32;
        let (x0, x1) = (w * 0.12, w * 0.88);
        let cw = (x1 - x0) / 16.0;
        let mut hits: Vec<Pt> = Vec::new();
        for (li, lane_hits) in row_hits.iter().enumerate() {
            let col = self.pal[li % 4];
            let y = y_top + (li as f32 + 0.5) * row_h;
            if let Some(vel) = lane_hits.iter().map(|n| n.vel).reduce(f32::max) {
                let c = self.column(lane_hits[0].t);
                let x = x0 + (c as f32 + 0.5) * cw;
                hits.push((x, y));
                self.rings.push(Ring {
                    x,
                    y,
                    born: t,
                    col,
                    a: 0.9 * vel,
                    max_r: 26.0 * u + 30.0 * u * env,
                    life: 0.6,
                });
            }
            // A beam from each note up into the core; a pitched note lands
            // on the spectrum ring's line for its pitch.
            for note in lane_hits {
                let x = x0 + (self.column(note.t) as f32 + 0.5) * cw;
                let (ang, rad) = if self.rows[li].pitched {
                    let f = 440.0 * 2f32.powf((note.pitch as f32 - 69.0) / 12.0);
                    let b = (39.0 * (f / 55.0).ln() / (12000.0f32 / 55.0).ln()).clamp(0.0, 39.0);
                    let side = if x < cx { -1.0 } else { 1.0 };
                    (
                        -FRAC_PI_2 + side * (b + 0.5) / 40.0 * PI + self.spin * 0.15,
                        r0,
                    )
                } else {
                    (-FRAC_PI_2 + (x - cx) / w * 1.4, h * 0.2)
                };
                let end = (cx + ang.cos() * rad, cy + ang.sin() * rad);
                layers.line(
                    false,
                    (x, y - row_h * 0.4),
                    end,
                    col,
                    (0.10 + 0.12 * energy) * note.vel,
                );
            }
            let row = &mut self.rows[li];
            for c in 0..16 {
                row.heat[c] *= (-dt / 4.0).exp();
                let hv = row.held[c].max(MEMORY * row.heat[c]);
                let x = x0 + (c as f32 + 0.5) * cw;
                let base = if c % 4 == 0 { 0.022 } else { 0.010 };
                let play = if c == col_now { 0.05 } else { 0.0 };
                let a = base + 0.30 * hv * hv + 0.12 * hv + play;
                layers.rect((x, y), (cw * 0.34, row_h * 0.26), col, a);
            }
        }
        // Coincidence: lanes that fire together are joined by a gold arc.
        for pair in hits.windows(2) {
            let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
            let lift = 60.0 * u + (bx - ax).abs() * 0.25;
            let gold = [1.0, 0.85, 0.5];
            let (mx, my) = ((ax + bx) * 0.5, (ay + by) * 0.5 - lift);
            let mut prev = (ax, ay);
            for s in 1..=48 {
                let tt = s as f32 / 48.0;
                let qx = (1.0 - tt).powi(2) * ax + 2.0 * (1.0 - tt) * tt * mx + tt * tt * bx;
                let qy = (1.0 - tt).powi(2) * ay + 2.0 * (1.0 - tt) * tt * my + tt * tt * by;
                layers.line(false, prev, (qx, qy), gold, 0.35);
                prev = (qx, qy);
            }
        }
        // Attacks: particle bursts from the core, sized by energy.
        if attack || (!hits.is_empty() && pitched == 0) {
            let burst = (40.0 + 140.0 * energy) as usize;
            for _ in 0..burst {
                let a = self.rand() * TAU;
                let sp = (1.5 + 6.0 * self.rand()) * u * (0.6 + energy);
                let col = self.pal[(self.rand() * 4.0) as usize % 4];
                let life = 1.0 + self.rand() * 1.5;
                self.parts.push(Particle {
                    x: cx,
                    y: cy,
                    vx: a.cos() * sp,
                    vy: a.sin() * sp,
                    life,
                    col,
                });
            }
        }
        if self.parts.len() > 9000 {
            let extra = self.parts.len() - 9000;
            self.parts.drain(..extra);
        }
        let drag = 0.975f32.powf(k);
        self.parts.retain_mut(|p| {
            p.x += p.vx * k;
            p.y += p.vy * k;
            p.vx *= drag;
            p.vy *= drag;
            // A slow curl around the core.
            let (dx, dy) = (p.x - cx, p.y - cy);
            p.x += -dy * 0.004 * motion * k;
            p.y += dx * 0.004 * motion * k;
            p.life -= dt;
            p.life > 0.0
        });
        for p in &self.parts {
            layers.dot(false, (p.x, p.y), p.col, 0.16 * p.life.min(1.0), 1);
        }

        // Rings: hits, shockwaves, section changes.
        self.rings.retain(|rg| t - rg.born <= rg.life);
        for rg in &self.rings {
            let age = t - rg.born;
            if age < 0.0 {
                continue;
            }
            let rad = 2.0 + rg.max_r * (1.0 - (-age * 4.0 / rg.life).exp());
            let a = rg.a * (1.0 - age / rg.life).powi(2) * 0.5;
            let pts = (rad * 0.9) as usize + 24;
            for p in 0..pts {
                let th = p as f32 / pts as f32 * TAU;
                let at = (rg.x + rad * th.cos(), rg.y + rad * th.sin());
                layers.dot(false, at, rg.col, a, 1);
            }
        }

        // Core: the mix's waveform as a closed polar line, twice,
        // counter-rotating.
        let sc: Vec<f32> = if scope.len() >= 4 {
            scope.to_vec()
        } else {
            vec![0.0; SCOPE]
        };
        let tail = ((self.sample_rate * dt) as usize).clamp(1, sc.len());
        let rms = (sc[sc.len() - tail..].iter().map(|x| x * x).sum::<f32>() / tail as f32).sqrt();
        let start = (1..sc.len() / 2)
            .find(|&j| sc[j - 1] < 0.0 && sc[j] >= 0.0)
            .unwrap_or(0);
        let span = 1024.min(sc.len() - start);
        let gain = 0.3 / rms.max(0.02);
        let raw: Vec<f32> = (0..=480)
            .map(|p| sc[start + (p as f32 / 480.0 * (span - 1) as f32) as usize])
            .collect();
        let pts: Vec<f32> = (0..raw.len())
            .map(|i| {
                let a = &raw[i.saturating_sub(2)..(i + 3).min(raw.len())];
                a.iter().sum::<f32>() / a.len() as f32
            })
            .collect();
        for (ki, sign) in [1.0f32, -1.0].iter().enumerate() {
            let col = mix(self.pal[ki], [1.0, 1.0, 1.0], 0.25);
            let rot = self.spin * sign;
            let mut last = None;
            for (p, &v) in pts.iter().enumerate() {
                let th = p as f32 / 480.0 * TAU + rot;
                let rr = base * (1.0 + 0.55 * (v * gain).clamp(-1.0, 1.0));
                let at = (cx + rr * th.cos(), cy + rr * th.sin());
                if let Some(prev) = last {
                    layers.line(true, prev, at, col, 0.35 + 0.4 * env);
                    layers.line(false, prev, at, col, 0.05 + 0.08 * glow);
                }
                last = Some(at);
            }
        }
        // Spectrum ring around the core, mirrored left/right.
        let window = &sc[sc.len().saturating_sub(SCOPE)..];
        self.spectrum(window, k);
        for b in 0..40 {
            let v = self.spec[b];
            let col = mix(self.pal[0], self.pal[1], b as f32 / 39.0);
            for side in [-1.0f32, 1.0] {
                let th = -FRAC_PI_2 + side * (b as f32 + 0.5) / 40.0 * PI + self.spin * 0.15;
                let (c1, s1) = (th.cos(), th.sin());
                let len = 3.0 * u + v * v * h * 0.13;
                let inner = (cx + c1 * r0, cy + s1 * r0);
                let tip = (cx + c1 * (r0 + len), cy + s1 * (r0 + len));
                layers.line(true, inner, tip, col, 0.18 + 0.4 * v);
                layers.dot(false, tip, col, 0.12 * v, 2);
            }
        }
        // Lead: a comet orbiting the core while a lead note is held; radius
        // from the note.
        if let Some(li) = self.rows.iter().position(|r| r.lead) {
            let held = self.rows[li].held_at(t, dt).map(|n| n.pitch).max();
            let lead_new = !row_hits[li].is_empty();
            let on = held.is_some();
            let rate = if on { 0.25 } else { 0.05 };
            let goal = if on { 1.0 } else { 0.0 };
            self.lead_lvl += (goal - self.lead_lvl) * per_frame(rate);
            if let Some(p) = held {
                self.lead_note = p as f32;
            }
            if self.lead_lvl > 0.02 {
                self.lead_ang += dt * (1.3 + 0.5 * energy);
                let rr = base * 2.3 + (self.lead_note - 64.0) * 9.0 * u;
                let at = (
                    cx + rr * self.lead_ang.cos(),
                    cy + rr * self.lead_ang.sin() * 0.62,
                );
                let col = mix(self.pal[3], [1.0, 1.0, 1.0], 0.35);
                layers.dot(false, at, col, 0.55 * self.lead_lvl, 2);
                layers.dot(true, at, [1.0, 1.0, 1.0], 0.8 * self.lead_lvl, 2);
                if lead_new {
                    self.rings.push(Ring {
                        x: at.0,
                        y: at.1,
                        born: t,
                        col,
                        a: 1.1,
                        max_r: 70.0 * u,
                        life: 1.0,
                    });
                    layers.line(false, at, (cx, cy), col, 0.25);
                }
            }
        }
        self.layers = layers;
    }
}

const PRIM_VERT: &str = r#"
#version 450 core
layout(location = 0) in vec2 aCorner;
layout(location = 1) in vec4 aSeg;
layout(location = 2) in vec4 aCol;
layout(location = 3) in vec4 aKind;
layout(std140, set = 0, binding = 0) uniform View { vec4 size; };
layout(location = 0) out vec4 vSeg;
layout(location = 1) out vec4 vCol;
layout(location = 2) out vec4 vKind;
void main() {
  float e = aKind.y;
  vec2 lo;
  vec2 hi;
  if (aKind.x > 1.5) {
    lo = aSeg.xy - aSeg.zw - e;
    hi = aSeg.xy + aSeg.zw + e;
  } else if (aKind.x > 0.5) {
    lo = min(aSeg.xy, aSeg.zw) - e;
    hi = max(aSeg.xy, aSeg.zw) + e;
  } else {
    lo = aSeg.xy - e - 1.0;
    hi = aSeg.xy + e + 1.0;
  }
  // Pixel centres sit at integer coordinates, as in av_proof.
  vec2 p = mix(lo, hi, aCorner) + 0.5;
  gl_Position = vec4(p.x / size.x * 2.0 - 1.0, 1.0 - p.y / size.y * 2.0, 0.0, 1.0);
  vSeg = aSeg;
  vCol = aCol;
  vKind = aKind;
}
"#;

const PRIM_FRAG: &str = r#"
#version 450 core
layout(location = 0) in vec4 vSeg;
layout(location = 1) in vec4 vCol;
layout(location = 2) in vec4 vKind;
layout(location = 0) out vec4 fragColor;
// Abramowitz and Stegun 7.1.26, error below 1.5e-7.
float erf1(float x) {
  float s = sign(x);
  float a = abs(x);
  float t = 1.0 / (1.0 + 0.3275911 * a);
  float y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
  return s * y;
}
void main() {
  vec2 p = gl_FragCoord.xy - 0.5;
  float k2 = vCol.w;
  float v = 0.0;
  if (vKind.x < 0.5) {
    // A dot covers a square window around its rounded centre.
    vec2 d = abs(p - floor(vSeg.xy + 0.5));
    if (max(d.x, d.y) > vKind.y) discard;
    vec2 f = p - vSeg.xy;
    v = exp(-dot(f, f) * k2);
  } else if (vKind.x < 1.5) {
    // The continuous sum of Gaussian dots along the segment.
    vec2 a = vSeg.xy;
    vec2 dir = vSeg.zw - a;
    float len = length(dir);
    vec2 un = dir / len;
    vec2 q = p - a;
    float x = dot(q, un);
    float y = q.x * un.y - q.y * un.x;
    if (length(q - un * clamp(x, 0.0, len)) > vKind.y) discard;
    float sk = sqrt(k2);
    v = exp(-k2 * y * y) * (0.8862269 / sk) * (erf1(sk * (len - x)) + erf1(sk * x));
  } else {
    vec2 e = max(abs(p - vSeg.xy) - vSeg.zw, vec2(0.0));
    v = clamp(1.0 - length(e) / 2.0, 0.0, 1.0);
  }
  fragColor = vec4(vCol.rgb * v, 0.0);
}
"#;

const FEEDBACK_BODY: &str = r#"
void main() {
  vec2 size = RENDERSIZE;
  vec2 c = size * 0.5;
  vec2 d = (gl_FragCoord.xy - 0.5 - c) / slot[0].x;
  float rr = length(d) / length(c);
  float a = slot[0].y + slot[0].z * rr;
  vec2 s = c + vec2(d.x * cos(a) - d.y * sin(a), d.x * sin(a) + d.y * cos(a));
  if (s.x < 0.0 || s.y < 0.0 || s.x >= size.x - 1.0 || s.y >= size.y - 1.0) {
    fragColor = vec4(0.0, 0.0, 0.0, 1.0);
    return;
  }
  vec3 v = texture(sampler2D(uTex0, uSamp0), (s + 0.5) / size).rgb;
  // Hue rotation about the grey axis.
  float h = slot[0].w;
  float co = cos(h);
  float k = (1.0 - co) / 3.0;
  float q = sqrt(1.0 / 3.0) * sin(h);
  mat3 m = mat3(co + k, k + q, k - q,
                k - q, co + k, k + q,
                k + q, k - q, co + k);
  fragColor = vec4(m * v * slot[1].x, 1.0);
}
"#;

const BRIGHT_BODY: &str = r#"
void main() {
  ivec2 o = ivec2(gl_FragCoord.xy) * 4;
  vec3 v = vec3(0.0);
  for (int dy = 0; dy < 4; dy++) {
    for (int dx = 0; dx < 4; dx++) {
      ivec2 q = o + ivec2(dx, dy);
      vec3 a = texelFetch(sampler2D(uTex0, uSamp0), q, 0).rgb;
      vec3 b = texelFetch(sampler2D(uTex1, uSamp1), q, 0).rgb;
      v += max(a + b - 0.25, vec3(0.0));
    }
  }
  fragColor = vec4(v / 16.0, 1.0);
}
"#;

const BLUR_BODY: &str = r#"
void main() {
  ivec2 o = ivec2(gl_FragCoord.xy);
  ivec2 size = ivec2(RENDERSIZE);
  ivec2 dir = slot[0].x > 0.5 ? ivec2(1, 0) : ivec2(0, 1);
  vec3 v = vec3(0.0);
  float n = 0.0;
  for (int d = -5; d <= 5; d++) {
    ivec2 q = o + dir * d;
    if (q.x < 0 || q.y < 0 || q.x >= size.x || q.y >= size.y) continue;
    v += texelFetch(sampler2D(uTex0, uSamp0), q, 0).rgb;
    n += 1.0;
  }
  fragColor = vec4(v / n, 1.0);
}
"#;

/// Bloom, background, vignette, tonemap, gamma, grain and flash; writes the
/// display value (0..1) into an HDR target.
const COMPOSE_BODY: &str = r#"
void main() {
  vec2 size = RENDERSIZE;
  vec2 p = gl_FragCoord.xy - 0.5;
  ivec2 ip = ivec2(p);
  vec3 acc = texelFetch(sampler2D(uTex0, uSamp0), ip, 0).rgb;
  vec3 crisp = texelFetch(sampler2D(uTex1, uSamp1), ip, 0).rgb;
  vec2 smallSize = slot[0].zw;
  vec3 b = texture(sampler2D(uTex2, uSamp2), (p / 4.0) / smallSize).rgb;
  float nx = p.x / size.x - 0.5;
  float ny = p.y / size.y - 0.5;
  float vig = 1.0 - 0.55 * (nx * nx * 1.2 + ny * ny * 1.6);
  uint x = uint(ip.x);
  uint y = uint(ip.y);
  uint frame = uint(slot[0].y);
  uint hsh = (x * 0x9E3779B1u) ^ (y * 0x85EBCA77u) ^ (frame * 0xC2B2AE3Du);
  hsh ^= hsh >> 15;
  hsh *= 0x2C1B3C6Du;
  hsh ^= hsh >> 12;
  float grain = (float(hsh & 0xFFFFu) / 65535.0 - 0.5) * 0.018;
  vec3 bg = vec3(0.010, 0.011, 0.018) * (1.0 - ny);
  vec3 v = (acc + crisp + 0.9 * b + slot[0].x + bg) * 1.15 * vig;
  vec3 a = (v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14);
  vec3 g = pow(clamp(a, 0.0, 1.0), vec3(1.0 / 2.2)) + grain;
  fragColor = vec4(clamp(g, 0.0, 1.0), 1.0);
}
"#;

/// Averages `ss x ss` blocks of the composed frame down to the output.
const DOWN_BODY: &str = r#"
void main() {
  int ss = int(slot[0].x);
  ivec2 o = ivec2(gl_FragCoord.xy) * ss;
  vec3 v = vec3(0.0);
  for (int dy = 0; dy < ss; dy++) {
    for (int dx = 0; dx < ss; dx++) {
      v += texelFetch(sampler2D(uTex0, uSamp0), o + ivec2(dx, dy), 0).rgb;
    }
  }
  fragColor = vec4(v / float(ss * ss), 1.0);
}
"#;

/// The av_proof look on the GPU.
pub struct AvProofScene {
    pub state: AvProof,
    /// Drawing size over output size.
    ss: u32,
    sampler: wgpu::Sampler,
    acc: [OffscreenTarget; 2],
    cur: usize,
    crisp: OffscreenTarget,
    small: OffscreenTarget,
    small2: OffscreenTarget,
    composed: OffscreenTarget,
    out: OffscreenTarget,
    pipeline: wgpu::RenderPipeline,
    view_bind: wgpu::BindGroup,
    corners: wgpu::Buffer,
    frame: u32,
}

impl AvProofScene {
    /// `width` must be a multiple of 64 (readback alignment). Below 1080
    /// lines the look is drawn at twice the size and averaged down.
    pub fn new(
        ctx: &GpuContext,
        width: u32,
        height: u32,
        sidecar: &Sidecar,
        sample_rate: f32,
    ) -> Self {
        let ss = if height < 1080 { 2 } else { 1 };
        let (w, h) = (width * ss, height * ss);
        let format = wgpu::TextureFormat::Rgba16Float;
        let module = |src: &str, stage| {
            ctx.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("avproof prims"),
                    source: wgpu::ShaderSource::Glsl {
                        shader: src.into(),
                        stage,
                        defines: Default::default(),
                    },
                })
        };
        let vs = module(PRIM_VERT, wgpu::naga::ShaderStage::Vertex);
        let fs = module(PRIM_FRAG, wgpu::naga::ShaderStage::Fragment);
        let bgl = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let size: [f32; 4] = [w as f32, h as f32, 0.0, 0.0];
        let size_buf = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("avproof view"),
                contents: bytemuck::bytes_of(&size),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view_bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: size_buf.as_entire_binding(),
            }],
        });
        const CORNERS: [[f32; 2]; 6] = [
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [0.0, 1.0],
            [1.0, 0.0],
            [1.0, 1.0],
        ];
        let corners = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("avproof corners"),
                contents: bytemuck::cast_slice(&CORNERS),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&bgl],
                push_constant_ranges: &[],
            });
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let attr = |offset, shader_location| wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset,
            shader_location,
        };
        let instance_attrs = [attr(0, 1), attr(16, 2), attr(32, 3)];
        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("avproof prims"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &vs,
                    entry_point: "main",
                    buffers: &[
                        wgpu::VertexBufferLayout {
                            array_stride: 8,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &[wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 0,
                            }],
                        },
                        wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Prim>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &instance_attrs,
                        },
                    ],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &fs,
                    entry_point: "main",
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: add,
                            alpha: add,
                        }),
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });
        Self {
            state: AvProof::new(sidecar, w, h, sample_rate),
            ss,
            sampler: ctx.create_clamp_sampler(),
            acc: [
                OffscreenTarget::new_hdr(ctx, w, h),
                OffscreenTarget::new_hdr(ctx, w, h),
            ],
            cur: 0,
            crisp: OffscreenTarget::new_hdr(ctx, w, h),
            small: OffscreenTarget::new_hdr(ctx, w / 4, h / 4),
            small2: OffscreenTarget::new_hdr(ctx, w / 4, h / 4),
            composed: OffscreenTarget::new_hdr(ctx, w, h),
            out: OffscreenTarget::new(ctx, width, height),
            pipeline,
            view_bind,
            corners,
            frame: 0,
        }
    }

    fn draw(&self, ctx: &GpuContext, target: &OffscreenTarget, prims: &[Prim], clear: bool) {
        let instances = (!prims.is_empty()).then(|| {
            ctx.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("avproof instances"),
                    contents: bytemuck::cast_slice(prims),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("avproof prims"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.view(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if clear {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some(buf) = &instances {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.view_bind, &[]);
                pass.set_vertex_buffer(0, self.corners.slice(..));
                pass.set_vertex_buffer(1, buf.slice(..));
                pass.draw(0..6, 0..prims.len() as u32);
            }
        }
        ctx.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Renders the frame at `inputs.t` and returns it as RGBA8. `scope` is
    /// the mono mix ending at `inputs.t`.
    pub fn render(
        &mut self,
        ctx: &GpuContext,
        inputs: &SceneInputs,
        scope: &[f32],
    ) -> Result<Vec<u8>, GpuError> {
        self.frame = self.frame.wrapping_add(1);
        self.state.step(inputs, scope);
        let s = &self.sampler;
        let wp = self.state.warp;
        let (prev, cur) = (&self.acc[self.cur], &self.acc[1 - self.cur]);
        run_pass(
            ctx,
            cur,
            &(glsl_header(2, 1) + FEEDBACK_BODY),
            &[
                [wp.zoom, wp.turn, wp.twist, wp.hue],
                [wp.decay, 0.0, 0.0, 0.0],
            ],
            &[(prev.view(), s)],
        )?;
        self.draw(ctx, cur, &self.state.layers.acc, false);
        self.draw(ctx, &self.crisp, &self.state.layers.crisp, true);
        run_pass(
            ctx,
            &self.small,
            &(glsl_header(1, 2) + BRIGHT_BODY),
            &[[0.0; 4]],
            &[(cur.view(), s), (self.crisp.view(), s)],
        )?;
        let blur = glsl_header(1, 1) + BLUR_BODY;
        for _ in 0..2 {
            run_pass(
                ctx,
                &self.small2,
                &blur,
                &[[1.0, 0.0, 0.0, 0.0]],
                &[(self.small.view(), s)],
            )?;
            run_pass(
                ctx,
                &self.small,
                &blur,
                &[[0.0; 4]],
                &[(self.small2.view(), s)],
            )?;
        }
        run_pass(
            ctx,
            &self.composed,
            &(glsl_header(1, 3) + COMPOSE_BODY),
            &[[
                self.state.flash * 0.05,
                self.frame as f32,
                self.small.width() as f32,
                self.small.height() as f32,
            ]],
            &[
                (cur.view(), s),
                (self.crisp.view(), s),
                (self.small.view(), s),
            ],
        )?;
        run_pass(
            ctx,
            &self.out,
            &(glsl_header(1, 1) + DOWN_BODY),
            &[[self.ss as f32, 0.0, 0.0, 0.0]],
            &[(self.composed.view(), s)],
        )?;
        self.cur = 1 - self.cur;
        self.out.read_pixels(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 30.0;

    /// Beats every 0.5 s and bars every 2 s over 20 s, with `lanes`.
    fn sidecar(lanes: serde_json::Value) -> Sidecar {
        let beats: Vec<f64> = (0..40).map(|i| i as f64 * 0.5).collect();
        let downbeats: Vec<f64> = (0..10).map(|i| i as f64 * 2.0).collect();
        let mut json = serde_json::json!({
            "schema": 4,
            "duration": 20.0,
            "tempo": 120.0,
            "beats": beats,
            "downbeats": downbeats,
            "sections": [{"start": 0.0, "end": 10.0, "kind": "build", "label": "intro"}],
            "events": [],
            "onsets": [],
            "energyEnvelope": [0.1, 0.2],
            "bandEnvelope": {"sub": [0.1], "low": [0.1], "mid": [0.1], "presence": [0.1], "air": [0.1]},
            "centroidEnvelope": [0.5],
            "flatnessEnvelope": [0.3],
            "envelopeRate": 20.0
        });
        if !lanes.is_null() {
            json["lanes"] = lanes;
        }
        Sidecar::from_json_str(&json.to_string()).unwrap()
    }

    fn lane(name: &str, notes: &[(f64, u8, u8, f64)]) -> serde_json::Value {
        let events: Vec<_> = notes
            .iter()
            .map(|&(t, pitch, velocity, length)| {
                serde_json::json!({"t": t, "pitch": pitch, "velocity": velocity, "length": length})
            })
            .collect();
        serde_json::json!({"name": name, "kind": "midi", "events": events})
    }

    fn inputs(t: f32) -> SceneInputs {
        SceneInputs {
            t,
            dt: DT,
            beat: t * 2.0,
            bar: t * 0.5,
            energy: 0.6,
            energy_slow: 0.5,
            sub: 0.5,
            air: 0.5,
            onsets: Vec::new(),
            kick: 0.0,
        }
    }

    fn scope(t: f32) -> Vec<f32> {
        (0..SCOPE)
            .map(|i| 0.4 * (TAU * 110.0 * (t + i as f32 / 48000.0)).sin())
            .collect()
    }

    /// Steps frames on the 30 fps grid from frame `from` up to frame `to`.
    fn run(av: &mut AvProof, from: usize, to: usize) {
        for i in from..=to {
            let t = i as f32 * DT;
            av.step(&inputs(t), &scope(t));
        }
    }

    /// A step is lit for its note's length and no longer.
    #[test]
    fn steps_follow_notes() {
        let sc = sidecar(serde_json::json!([
            lane("KICK", &[(5.0, 36, 127, 0.25)]),
            lane("Clap", &[(1.0, 50, 100, 0.5)]),
        ]));
        let mut av = AvProof::new(&sc, 1920, 1080, 48000.0);
        // Frame 36 is t = 1.2 s.
        run(&mut av, 0, 36);
        assert!(
            (av.rows[1].lit - 100.0 / 127.0).abs() < 1e-6,
            "lit at 1.2 s: {}",
            av.rows[1].lit
        );
        let col = av.column(1.0);
        assert!(av.rows[1].held[col] > 0.0, "its cell is held");
        // Frame 48 is t = 1.6 s.
        run(&mut av, 37, 48);
        assert_eq!(av.rows[1].lit, 0.0, "unlit at 1.6 s");
        assert_eq!(av.rows[1].held, [0.0; 16]);
    }

    /// A zero-length note lights its step for the frame it falls in.
    #[test]
    fn zero_length_note_lights_one_frame() {
        let sc = sidecar(serde_json::json!([lane("Hats", &[(1.01, 42, 127, 0.0)])]));
        let mut av = AvProof::new(&sc, 1920, 1080, 48000.0);
        // 1.01 s falls in frame 31's window (1.0, 1.0333].
        run(&mut av, 0, 31);
        assert!(av.rows[0].lit > 0.0);
        run(&mut av, 32, 32);
        assert_eq!(av.rows[0].lit, 0.0);
    }

    /// A kick note-on starts a shockwave within one frame; nothing else does.
    #[test]
    fn kick_starts_a_shockwave() {
        let sc = sidecar(serde_json::json!([
            lane("Clap", &[(1.0, 50, 127, 0.1)]),
            lane("KICK", &[(2.01, 36, 127, 0.25)]),
        ]));
        let mut av = AvProof::new(&sc, 1920, 1080, 48000.0);
        let shocks = |av: &AvProof| av.rings.iter().filter(|r| r.max_r == av.h * 0.55).count();
        // Checked every frame: a shockwave lives 0.7 s, so one wrongly
        // started by the Clap at 1.0 s would be gone by frame 60.
        for i in 0..=60 {
            run(&mut av, i, i);
            assert_eq!(shocks(&av), 0, "no shockwave before the kick (frame {i})");
        }
        run(&mut av, 61, 61);
        let born: Vec<f32> = av
            .rings
            .iter()
            .filter(|r| r.max_r == av.h * 0.55)
            .map(|r| r.born)
            .collect();
        assert_eq!(born.len(), 1, "one shockwave");
        assert!(
            born[0] >= 2.01 && born[0] - 2.01 <= DT,
            "shockwave starts at {} for a kick at 2.01 s",
            born[0]
        );
    }

    #[test]
    fn kick_lane_names() {
        for name in [
            "KICK",
            "Kick 2",
            "Generator",
            "Generator 1",
            "Generator (Kick)",
        ] {
            assert!(Row::new(name, Vec::new()).kick, "{name} is a kick lane");
        }
        for name in ["Clap", "Perc 1 · BD", "MAIN SYNTH"] {
            assert!(
                !Row::new(name, Vec::new()).kick,
                "{name} is not a kick lane"
            );
        }
    }

    /// Without lanes the beats drive one row and the shockwave.
    #[test]
    fn without_lanes_the_beats_drive_it() {
        let mut av = AvProof::new(&sidecar(serde_json::Value::Null), 1920, 1080, 48000.0);
        assert_eq!(av.rows.len(), 1);
        assert!(av.beats_pulse);
        run(&mut av, 1, 15);
        assert!(av.rings.iter().any(|r| r.max_r == av.h * 0.55));
    }

    /// Too many lanes: the ones with the fewest notes go, the kick stays,
    /// and the rest keep their order.
    #[test]
    fn too_many_lanes_drop_the_quietest() {
        let mut lanes = vec![lane("KICK", &[(1.0, 36, 127, 0.1)])];
        for i in 0..MAX_ROWS + 1 {
            let notes: Vec<_> = (0..=i).map(|j| (j as f64 * 0.25, 60, 100, 0.1)).collect();
            lanes.push(lane(&format!("L{i}"), &notes));
        }
        let av = AvProof::new(&sidecar(serde_json::json!(lanes)), 1920, 1080, 48000.0);
        assert_eq!(av.rows.len(), MAX_ROWS);
        assert!(av.rows[0].kick);
        // L0 (1 note) and L1 (2 notes) are the quietest; L2 comes next.
        assert_eq!(av.rows[1].notes.len(), 3);
    }

    /// A lane silent for a whole section keeps its row, dim.
    #[test]
    fn silent_lane_keeps_its_row() {
        let sc = sidecar(serde_json::json!([
            lane("KICK", &[(0.5, 36, 127, 0.1), (9.0, 36, 127, 0.1)]),
            lane("Clap", &[(9.5, 50, 127, 0.1)]),
        ]));
        let mut av = AvProof::new(&sc, 1920, 1080, 48000.0);
        run(&mut av, 0, 90);
        assert_eq!(av.rows.len(), 2);
        // Two rows of 16 cells, every one drawn.
        let cells: Vec<&Prim> = av
            .layers
            .crisp
            .iter()
            .filter(|p| p.kind[0] == 2.0)
            .collect();
        assert_eq!(cells.len(), 32);
        assert!(cells[16..].iter().all(|p| p.col[0] > 0.0));
    }

    #[test]
    fn cues_split_at_section_boundaries() {
        let sc = sidecar(serde_json::Value::Null);
        assert_eq!(
            cues(&sc),
            vec![
                Cue {
                    start: 0.0,
                    label: Some("intro".into())
                },
                Cue {
                    start: 10.0,
                    label: None
                },
            ]
        );
    }

    /// The scene renders frames that change and are neither black nor
    /// blown out.
    #[test]
    fn avproof_renders_changing_non_degenerate_frames() {
        let ctx = match GpuContext::new_blocking() {
            Ok(c) => c,
            Err(_) => return,
        };
        let sc = sidecar(serde_json::json!([
            lane("KICK", &[(0.1, 36, 127, 0.25), (0.4, 36, 127, 0.25)]),
            lane("MAIN SYNTH", &[(0.2, 60, 127, 0.3), (0.5, 67, 110, 0.3)]),
        ]));
        let mut r = AvProofScene::new(&ctx, 128, 64, &sc, 48000.0);
        let mean = |p: &[u8]| p.iter().map(|&b| b as f64).sum::<f64>() / p.len() as f64;
        let mut frames = Vec::new();
        for i in 0..20 {
            let t = i as f32 * DT;
            frames.push(r.render(&ctx, &inputs(t), &scope(t)).unwrap());
        }
        let m = mean(&frames[19]);
        assert!(
            m > 1.0 && m < 240.0,
            "frame should be non-degenerate, mean {m}"
        );
        assert_ne!(frames[18], frames[19], "frames should change");
    }
}
