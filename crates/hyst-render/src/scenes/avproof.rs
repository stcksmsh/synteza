//! kabl's `av_proof` look (`tools/examples/av_proof.rs` in the kabl repo),
//! ported to the GPU and driven by the song's own notes.
//!
//! The waveform core, spectrum ring, feedback, bloom and tonemap follow
//! av_proof's CPU drawing. Its per-frame constants were written for 60 fps;
//! here they are raised to `dt * 60` so any frame rate moves the same way.
//! Geometry is placed in av_proof's 1080-line pixel units and drawn at twice
//! the output size below 1080 lines, then averaged down.
//!
//! Each lane drives the scene by its character ([`lanes::classify`]):
//! - sustained lanes (long or legato notes with a moving pitch) get a form,
//!   at most [`MAX_RIBBONS`], each in its own colour, weight and place:
//!   - a melody is a ribbon. Its head's height is the pitch, rising with it
//!     over the lane's own range in the lane's band ([`MELODY_BANDS`]),
//!     gliding to each new note over [`GLIDE`] seconds and still while a
//!     note is held. Its trail flows away to the left and drifts slowly up.
//!   - a chord lane ([`lanes::Character::Chordal`]) is a band of dotted
//!     rings around the core, one per chord tone, spaced by the voicing and
//!     moving to each new chord over [`CHORD_GLIDE`] seconds.
//!
//!   Both live while their notes are held and fade with the phrase.
//! - percussive lanes push what is already there: each hit is an impulse on
//!   the drifting dust, the melody trails (lightly) and the core, scaled by velocity
//!   and decaying with drag. Kicks push outward, snares twist, hats shiver,
//!   toms shove from a side, anything else pushes out from the spectrum ring
//!   at its pitch.
//! - core pulse and shockwave: the first kick lane (else the beats).
//! - core size: an attack/release follower of the pitched lanes' notes.
//! - lead comet: the lead lane's held note, riding its ribbon when it has one.
//! - palette and section rings: section boundaries.
//! - waveform core and spectrum ring: the WAV.
//! - macros: `energy` is the song's smoothed loudness; `motion` stays at
//!   kabl's default 0.5; the LFO is a sine over four bars.
//!
//! [`Mood`] picks the palette and how hard everything moves.

mod lanes;

use std::collections::VecDeque;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use hyst_core::sidecar::Sidecar;
use wgpu::util::DeviceExt;

use crate::gpu::{GpuContext, GpuError, OffscreenTarget};

use self::lanes::{Character, Drum, Note};
use super::music::SceneInputs;
use super::{glsl_header, run_pass};

/// Bright palettes; a section change moves to the next one with a smooth blend.
const BRIGHT: [[[f32; 3]; 4]; 5] = [
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

/// Dark palettes: muted, cold, low.
const DARK: [[[f32; 3]; 4]; 3] = [
    [
        [0.26, 0.36, 0.55],
        [0.20, 0.44, 0.47],
        [0.50, 0.46, 0.60],
        [0.56, 0.34, 0.32],
    ], // slate/teal
    [
        [0.36, 0.28, 0.50],
        [0.22, 0.36, 0.52],
        [0.52, 0.44, 0.46],
        [0.30, 0.46, 0.44],
    ], // plum/steel
    [
        [0.24, 0.40, 0.42],
        [0.42, 0.30, 0.40],
        [0.46, 0.50, 0.56],
        [0.50, 0.38, 0.28],
    ], // moss/mauve
];

/// The scene's mood: a manual switch until the director chooses it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mood {
    /// av_proof's own palette and punch.
    #[default]
    Bright,
    /// Muted dark palette, long soft trails, softer impulses, no flash.
    Dark,
}

impl Mood {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "bright" => Some(Self::Bright),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

/// What a mood sets.
struct Look {
    palettes: &'static [[[f32; 3]; 4]],
    /// Feedback decay per 60 fps frame.
    decay: f32,
    /// Feedback zoom per 60 fps frame: base, energy, punch.
    zoom: [f32; 3],
    hue: f32,
    /// Section-change flash, added to the whole frame.
    flash: f32,
    /// Scales every percussive impulse.
    impulse: f32,
    /// Scales what is drawn into the feedback buffer, so a longer decay does
    /// not pile up into a brighter frame.
    acc_gain: f32,
    crisp_gain: f32,
    /// Seconds a ribbon's trail lives.
    trail: f32,
    /// Ribbon release outside a phrase, seconds.
    release: f32,
    shock: f32,
    dust: f32,
}

const BRIGHT_LOOK: Look = Look {
    palettes: &BRIGHT,
    decay: 0.9225,
    zoom: [1.0035, 0.006, 0.012],
    hue: 0.009,
    flash: 0.05,
    impulse: 1.0,
    acc_gain: 1.0,
    crisp_gain: 1.0,
    trail: 1.6,
    release: 0.4,
    shock: 0.22,
    dust: 0.10,
};

const DARK_LOOK: Look = Look {
    palettes: &DARK,
    decay: 0.958,
    zoom: [1.0015, 0.003, 0.006],
    hue: 0.003,
    flash: 0.0,
    impulse: 0.5,
    acc_gain: 0.55,
    crisp_gain: 0.75,
    trail: 3.2,
    release: 1.0,
    shock: 0.16,
    dust: 0.07,
};

impl Mood {
    fn look(self) -> &'static Look {
        match self {
            Self::Bright => &BRIGHT_LOOK,
            Self::Dark => &DARK_LOOK,
        }
    }
}

/// At most this many sustained lanes get a ribbon, the busiest first.
pub const MAX_RIBBONS: usize = 4;
/// Samples of the mix the waveform core and spectrum read.
pub const SCOPE: usize = 2048;
/// Release time of the note envelope follower, seconds.
const ENV_RELEASE: f32 = 0.25;
/// A gap between notes shorter than this keeps a ribbon's phrase alive.
const PHRASE_GAP: f32 = 1.5;
/// Ribbon release inside a phrase, seconds.
const PHRASE_RELEASE: f32 = 2.0;
/// Dust particles always present for the hits to push.
const DUST: usize = 1400;
/// Melody bands, busiest melody first: centre height and span, as shares of
/// the frame height. A melody's head height is its pitch.
const MELODY_BANDS: [(f32, f32); MAX_RIBBONS] =
    [(0.40, 0.42), (0.78, 0.18), (0.10, 0.10), (0.92, 0.08)];
/// Seconds a melody head takes to glide to a new note.
const GLIDE: f32 = 0.10;
/// Seconds a chord band takes to move to a new voicing.
const CHORD_GLIDE: f32 = 0.35;
/// Chord bands: inner radius of the first, the step to the next, and the
/// radius per semitone of voicing, shares of the frame height.
const CHORD_R0: f32 = 0.22;
const CHORD_STEP: f32 = 0.15;
const CHORD_SEMI: f32 = 0.008;
/// Share of the frame width a melody trail crosses over its life.
const TRAIL_SPAN: f32 = 0.42;
/// A melody trail's upward drift, share of the frame height per second.
const TRAIL_RISE: f32 = 0.035;
/// Share of a drum's push a melody trail takes; hats give none.
const RIBBON_PUSH: f32 = 0.05;

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

/// A trail point of a melody ribbon: it flows away from the head and
/// drifts a little when hits push it.
struct TrailPt {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    age: f32,
    /// The ribbon's level when the point was laid.
    a: f32,
}

/// A portamento: from `from` to `to` along a smoothstep over `dur` seconds,
/// so it never overshoots or turns back.
#[derive(Clone, Copy, Debug)]
struct Glide {
    from: f32,
    to: f32,
    g: f32,
    dur: f32,
}

impl Glide {
    fn new(v: f32, dur: f32) -> Self {
        Self {
            from: v,
            to: v,
            g: dur,
            dur,
        }
    }

    fn value(&self) -> f32 {
        let s = (self.g / self.dur).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * s * s * (3.0 - 2.0 * s)
    }

    /// Starts a glide from where it is now, unless already heading to `to`.
    fn retarget(&mut self, to: f32) {
        if to != self.to {
            self.from = self.value();
            self.to = to;
            self.g = 0.0;
        }
    }
}

/// What a sustained lane is drawn as, and its place: the melody band or
/// the chord ring it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Melody(usize),
    Chord(usize),
}

/// A sustained lane's form.
struct Ribbon {
    form: Form,
    /// Pitch range of the lane.
    lo: f32,
    hi: f32,
    level: f32,
    /// Melody: head height.
    y: Glide,
    /// Chord: inner and outer radius of the band, before the swell.
    r_lo: Glide,
    r_hi: Glide,
    /// Chord: the pitches of the latest chord.
    tones: Vec<u8>,
    /// Slow sway (melody) or turn (chord), radians.
    phase: f32,
    head: Pt,
    trail: VecDeque<TrailPt>,
    /// Segments drawn last frame.
    drawn: usize,
}

struct Lane {
    name: String,
    notes: Vec<Note>,
    /// Latest note end among `notes[..=i]`.
    ends: Vec<f32>,
    longest: f32,
    character: Character,
    drum: Drum,
    pitched: bool,
    kick: bool,
    lead: bool,
    env: f32,
    hits: u32,
    ribbon: Option<Ribbon>,
}

impl Lane {
    fn new(name: &str, mut notes: Vec<Note>) -> Self {
        notes.sort_by(|a, b| a.t.total_cmp(&b.t));
        let longest = notes.iter().map(|n| n.end - n.t).fold(0.0, f32::max);
        let first = notes.first().map(|n| n.pitch);
        let ends = notes
            .iter()
            .scan(f32::NEG_INFINITY, |m, n| {
                *m = m.max(n.end);
                Some(*m)
            })
            .collect();
        let lower = name.to_lowercase();
        Self {
            name: name.to_string(),
            character: lanes::classify(name, &notes),
            drum: lanes::drum(name),
            pitched: notes.iter().any(|n| Some(n.pitch) != first),
            // The pulse lane: a name with a word starting "kick" or
            // "generator". "BD" pushes like a kick but does not pulse.
            kick: lower
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w.starts_with("kick") || w.starts_with("generator")),
            lead: lower.contains("lead") || lower.contains("main synth"),
            longest,
            ends,
            notes,
            env: 0.0,
            hits: 0,
            ribbon: None,
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

    /// Between two notes less than [`PHRASE_GAP`] apart.
    fn in_phrase(&self, t: f32) -> bool {
        let i = self.notes.partition_point(|n| n.t <= t);
        match (i.checked_sub(1), self.notes.get(i)) {
            (Some(p), Some(next)) => next.t - self.ends[p] <= PHRASE_GAP,
            _ => false,
        }
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

/// A dust mote: always there, drifting, for the hits to push.
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    age: f32,
    life: f32,
    col: usize,
}

/// One hit's push this frame.
#[derive(Clone, Copy, Debug)]
struct Impulse {
    drum: Drum,
    origin: Pt,
    /// Direction of a twist, +1 or -1.
    sign: f32,
    /// Strength, pixels per 60 fps frame.
    s: f32,
}

impl Impulse {
    /// The velocity change at `p`. `c` is the core, `h` the frame height,
    /// `rnd` two values in -1..1 for a shiver.
    fn at(&self, p: Pt, c: Pt, h: f32, rnd: Pt) -> Pt {
        let away = |from: Pt, reach: f32| {
            let (dx, dy) = (p.0 - from.0, p.1 - from.1);
            let d = (dx * dx + dy * dy).sqrt().max(1.0);
            let f = self.s / (1.0 + (d / reach).powi(2));
            (dx / d * f, dy / d * f)
        };
        match self.drum {
            Drum::Kick => away(c, 0.35 * h),
            Drum::Snare => {
                let (rx, ry) = away(c, 0.45 * h);
                (-ry * self.sign, rx * self.sign)
            }
            Drum::Hat => (rnd.0 * self.s, rnd.1 * self.s),
            Drum::Tom | Drum::Pitched => away(self.origin, 0.2 * h),
        }
    }
}

/// One additive primitive, in pixels: a soft dot or a line of soft dots.
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Prim {
    /// Dot: centre, unused. Line: both ends.
    seg: [f32; 4],
    /// Colour times weight, and the Gaussian's `k2`.
    col: [f32; 4],
    /// Kind (0 dot, 1 line), reach in pixels, unused, unused.
    kind: [f32; 4],
}

type Pt = (f32, f32);

struct Layers {
    /// Drawn into the feedback buffer.
    acc: Vec<Prim>,
    /// Drawn fresh each frame.
    crisp: Vec<Prim>,
    /// Weight of the feedback and the fresh layer.
    gain: [f32; 2],
}

impl Layers {
    fn get(&mut self, crisp: bool) -> (&mut Vec<Prim>, f32) {
        if crisp {
            (&mut self.crisp, self.gain[1])
        } else {
            (&mut self.acc, self.gain[0])
        }
    }

    /// av_proof's `dot`: `a * exp(-k2 d^2)` over a `(2r+1)^2` window.
    fn dot(&mut self, crisp: bool, p: Pt, c: [f32; 3], a: f32, r: u32) {
        let k2 = if r <= 1 { 1.6 } else { 0.7 };
        let (v, g) = self.get(crisp);
        let a = a * g;
        v.push(Prim {
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
        let (v, g) = self.get(crisp);
        let w = a * g * (n + 1.0) / len;
        v.push(Prim {
            seg: [p0.0, p0.1, p1.0, p1.1],
            col: [c[0] * w, c[1] * w, c[2] * w, 1.6],
            kind: [1.0, 1.5, 0.0, 0.0],
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
    mood: Mood,
    lanes: Vec<Lane>,
    /// No kick lane: the beats pulse the core instead.
    beats_pulse: bool,
    beats: Vec<f32>,
    cues: Vec<f32>,
    sample_rate: f32,
    rings: Vec<Ring>,
    parts: Vec<Particle>,
    spec: [f32; 40],
    pal: [[f32; 3]; 4],
    scene: usize,
    spin: f32,
    /// Extra spin from snares, radians per second; decays.
    spin_v: f32,
    /// The core's displacement from its rest and its velocity (toms).
    core_off: Pt,
    core_vel: Pt,
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
    pub fn new(sidecar: &Sidecar, w: u32, h: u32, sample_rate: f32, mood: Mood) -> Self {
        let beats: Vec<f32> = sidecar.beats.iter().map(|&b| b as f32).collect();
        let mut lanes: Vec<(String, Lane)> = match &sidecar.lanes {
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
                    (l.name.clone(), Lane::new(&l.name, notes))
                })
                .collect(),
            None => {
                eprintln!("avproof: the sidecar has no lanes; the beats act as one kick lane");
                let notes = beats
                    .iter()
                    .map(|&t| Note {
                        t,
                        end: t,
                        pitch: 36,
                        vel: 1.0,
                    })
                    .collect();
                let mut lane = Lane::new("beats", notes);
                lane.drum = Drum::Kick;
                lane.kick = true;
                vec![("beats".to_string(), lane)]
            }
        };
        // Only the first kick and lead lanes count as such; a chord lane
        // leads nothing.
        let (mut kick_seen, mut lead_seen) = (false, false);
        for (_, l) in &mut lanes {
            l.kick &= !kick_seen && l.character == Character::Percussive;
            kick_seen |= l.kick;
            l.lead &= !lead_seen && l.character != Character::Chordal;
            lead_seen |= l.lead;
        }
        if !lead_seen {
            if let Some((_, l)) = lanes
                .iter_mut()
                .filter(|(_, l)| l.pitched && l.character != Character::Chordal)
                .max_by_key(|(_, l)| l.notes.len())
            {
                l.lead = true;
            }
        }
        // Forms for the busiest sustained lanes: a melody band each for
        // single-note lines, a ring each for chords.
        let mut sustained: Vec<usize> = (0..lanes.len())
            .filter(|&i| lanes[i].1.character != Character::Percussive)
            .collect();
        sustained.sort_by_key(|&i| (std::cmp::Reverse(lanes[i].1.notes.len()), i));
        if sustained.len() > MAX_RIBBONS {
            let dropped: Vec<String> = sustained[MAX_RIBBONS..]
                .iter()
                .map(|&i| format!("{} ({} notes)", lanes[i].0, lanes[i].1.notes.len()))
                .collect();
            eprintln!(
                "avproof: {} sustained lanes, at most {MAX_RIBBONS} ribbons; dropped {}",
                sustained.len(),
                dropped.join(", ")
            );
        }
        let (wf, hf) = (w as f32, h as f32);
        let (mut melodies, mut chords) = (0, 0);
        for &i in sustained.iter().take(MAX_RIBBONS) {
            let l = &mut lanes[i].1;
            let lo = l.notes.iter().map(|n| n.pitch).min().unwrap_or(60) as f32;
            let hi = l.notes.iter().map(|n| n.pitch).max().unwrap_or(60) as f32;
            let form = if l.character == Character::Chordal {
                chords += 1;
                Form::Chord(chords - 1)
            } else {
                melodies += 1;
                Form::Melody(melodies - 1)
            };
            let mut rb = Ribbon {
                form,
                lo,
                hi,
                level: 0.0,
                y: Glide::new(0.0, GLIDE),
                r_lo: Glide::new(0.0, CHORD_GLIDE),
                r_hi: Glide::new(0.0, CHORD_GLIDE),
                tones: Vec::new(),
                phase: 0.0,
                head: (0.0, 0.0),
                trail: VecDeque::new(),
                drawn: 0,
            };
            match form {
                Form::Melody(b) => {
                    rb.y = Glide::new(melody_y(&rb, (lo + hi) * 0.5, hf), GLIDE);
                    rb.head = (melody_x(b, wf), rb.y.value());
                }
                Form::Chord(c) => {
                    let r = chord_r(c, hf, 0.0);
                    rb.r_lo = Glide::new(r, CHORD_GLIDE);
                    rb.r_hi = Glide::new(r, CHORD_GLIDE);
                }
            }
            l.ribbon = Some(rb);
        }
        for (name, l) in &lanes {
            let role = match (&l.ribbon, l.character) {
                (Some(r), _) => format!("{:?}", r.form),
                (None, Character::Percussive) => format!("percussive: {:?} push", l.drum),
                (None, c) => format!("{c:?}: no form"),
            };
            let extra = match (l.kick, l.lead) {
                (true, _) => "; core pulse and shockwave",
                (_, true) => "; lead comet",
                _ => "",
            };
            eprintln!(
                "avproof: lane {name:?}, {} notes, {role}{extra}",
                l.notes.len()
            );
        }
        let beats_pulse = !kick_seen;
        if beats_pulse {
            eprintln!("avproof: no kick lane; the beats pulse the core");
        }
        let look = mood.look();
        let mut av = Self {
            w: w as f32,
            h: h as f32,
            mood,
            lanes: lanes.into_iter().map(|(_, l)| l).collect(),
            beats_pulse,
            beats,
            cues: cues(sidecar).iter().map(|c| c.start).collect(),
            sample_rate,
            rings: Vec::new(),
            parts: Vec::with_capacity(DUST),
            spec: [0.0; 40],
            pal: look.palettes[0],
            scene: 0,
            spin: 0.0,
            spin_v: 0.0,
            core_off: (0.0, 0.0),
            core_vel: (0.0, 0.0),
            flash: 0.0,
            punch: 0.0,
            rng: 0x9E37_79B9_7F4A_7C15,
            lead_ang: 0.0,
            lead_lvl: 0.0,
            lead_note: 64.0,
            started: false,
            layers: Layers {
                acc: Vec::new(),
                crisp: Vec::new(),
                gain: [look.acc_gain, look.crisp_gain],
            },
            warp: Warp {
                zoom: 1.0,
                turn: 0.0,
                twist: 0.0,
                decay: 0.0,
                hue: 0.0,
            },
        };
        for _ in 0..DUST {
            let p = av.new_particle();
            // Spread the first motes' ages so they do not all fade together.
            let age = av.rand() * p.life;
            av.parts.push(Particle { age, ..p });
        }
        av
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    fn new_particle(&mut self) -> Particle {
        let a = self.rand() * TAU;
        let r = self.h * 0.12 + self.rand() * self.w * 0.5;
        Particle {
            x: self.w * 0.5 + a.cos() * r,
            y: self.h * 0.42 + a.sin() * r * 0.7,
            vx: 0.0,
            vy: 0.0,
            age: 0.0,
            life: 4.0 + 5.0 * self.rand(),
            col: (self.rand() * 4.0) as usize % 4,
        }
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
        let look = self.mood.look();
        let (w, h) = (self.w, self.h);
        let u = h / 1080.0;
        let (t, dt) = (inp.t, inp.dt.max(1e-4));
        // av_proof's per-frame constants are for 60 fps.
        let k = dt * 60.0;
        let per_frame = |rate: f32| 1.0 - (1.0 - rate).powf(k);
        let (energy, motion) = (inp.energy_slow, 0.5f32);
        let lfo = (TAU * inp.bar / 4.0).sin();
        self.layers.acc.clear();
        self.layers.crisp.clear();

        // Section change: next palette; under the bright mood a flash and a
        // shockwave, under the dark one a single soft ring.
        let (rest_x, rest_y) = (w * 0.5, h * 0.42);
        let cue = self.cue_at(t);
        if !self.started {
            self.started = true;
            self.scene = cue % look.palettes.len();
            self.pal = look.palettes[self.scene];
        } else if cue != self.cue_at(t - dt) {
            self.scene = (self.scene + 1) % look.palettes.len();
            self.flash = 1.0;
            let n = if self.mood == Mood::Dark { 1 } else { 3 };
            for i in 0..n {
                self.rings.push(Ring {
                    x: rest_x,
                    y: rest_y,
                    born: t + i as f32 * 0.09,
                    col: mix(self.pal[2], [1.0, 0.95, 0.85], 0.5),
                    a: 1.4 * look.acc_gain,
                    max_r: w * 0.7,
                    life: 1.8,
                });
            }
        }
        let target = look.palettes[self.scene];
        for (p, &c) in self.pal.iter_mut().zip(&target) {
            *p = mix(*p, c, per_frame(0.025));
        }

        // Notes: each lane's envelope follower, and this frame's hits.
        let mut impulses: Vec<Impulse> = Vec::new();
        let mut kick_hit = false;
        let (cx, cy) = (rest_x + self.core_off.0, rest_y + self.core_off.1);
        let r0_rest = h * 0.085 * 1.55 + 8.0 * u;
        for li in 0..self.lanes.len() {
            let hits = self.lanes[li].onsets(t, dt).to_vec();
            let lit = self.lanes[li]
                .held_at(t, dt)
                .map(|n| n.vel)
                .fold(0.0, f32::max);
            let lane = &mut self.lanes[li];
            lane.env = if lit >= lane.env {
                lit
            } else {
                lane.env * (-dt / ENV_RELEASE).exp()
            };
            if lane.character != Character::Percussive {
                continue;
            }
            kick_hit |= lane.kick && !hits.is_empty();
            for note in &hits {
                lane.hits += 1;
                let sign = if lane.hits.is_multiple_of(2) {
                    1.0
                } else {
                    -1.0
                };
                let origin = match lane.drum {
                    Drum::Tom => (rest_x + sign * w * 0.45, rest_y),
                    Drum::Pitched => {
                        let f = 440.0 * 2f32.powf((note.pitch as f32 - 69.0) / 12.0);
                        let b =
                            (39.0 * (f / 55.0).ln() / (12000.0f32 / 55.0).ln()).clamp(0.0, 39.0);
                        let ang = -FRAC_PI_2 + sign * (b + 0.5) / 40.0 * PI + self.spin * 0.15;
                        (cx + ang.cos() * r0_rest, cy + ang.sin() * r0_rest)
                    }
                    _ => (cx, cy),
                };
                let strength = match lane.drum {
                    Drum::Kick => 4.0,
                    Drum::Snare => 3.0,
                    Drum::Hat => 0.5,
                    Drum::Tom => 3.5,
                    Drum::Pitched => 2.0,
                };
                impulses.push(Impulse {
                    drum: lane.drum,
                    origin,
                    sign,
                    s: strength * u * note.vel * look.impulse,
                });
            }
        }
        let pitched = self.lanes.iter().filter(|l| l.pitched).count();
        let env = self
            .lanes
            .iter()
            .filter(|l| l.pitched)
            .map(|l| l.env)
            .sum::<f32>()
            / pitched.max(1) as f32;

        // The kick (or, without one, the beat): a core punch and a shockwave.
        let pulse = if self.beats_pulse {
            let lo = self.beats.partition_point(|&b| b <= t - dt);
            self.beats.partition_point(|&b| b <= t) > lo
        } else {
            kick_hit
        };
        if pulse {
            self.punch = 1.0;
            self.rings.push(Ring {
                x: cx,
                y: cy,
                born: t,
                col: self.pal[2],
                a: look.shock,
                max_r: h * 0.55,
                life: 0.7,
            });
            if self.beats_pulse {
                impulses.push(Impulse {
                    drum: Drum::Kick,
                    origin: (cx, cy),
                    sign: 1.0,
                    s: 4.0 * u * look.impulse,
                });
            }
        }
        // The core takes the snares as spin and the toms as a shove.
        for imp in &impulses {
            match imp.drum {
                Drum::Snare => self.spin_v += imp.sign * imp.s / u * 0.25,
                Drum::Tom => {
                    let d = if imp.origin.0 < rest_x { 1.0 } else { -1.0 };
                    self.core_vel.0 += d * imp.s * 12.0;
                }
                _ => {}
            }
        }
        // A damped spring brings the core home.
        let om = 5.0;
        for (off, vel) in [
            (&mut self.core_off.0, &mut self.core_vel.0),
            (&mut self.core_off.1, &mut self.core_vel.1),
        ] {
            *vel += (-om * om * *off - 2.0 * om * *vel) * dt;
            *off += *vel * dt;
        }
        self.punch *= (-9.0 * dt).exp();
        self.flash *= (-2.5 * dt).exp();
        self.spin_v *= (-3.0 * dt).exp();
        self.spin += dt * (0.12 + 0.5 * motion + 0.4 * lfo + self.spin_v);

        self.warp = Warp {
            zoom: (look.zoom[0] + look.zoom[1] * energy + look.zoom[2] * self.punch).powf(k),
            turn: (0.0015 + 0.004 * lfo * motion) * k,
            twist: (0.010 * (motion - 0.3) + 0.004 * (t * 0.21).sin()) * k * look.impulse,
            decay: look.decay.powf(k),
            hue: look.hue * k,
        };

        // Core size, needed by the spectrum ring.
        let base = h * (0.085 + 0.07 * env) * (1.0 + 0.08 * self.punch);
        let r0 = base * 1.55 + 8.0 * u;

        // Dust: drifts and curls around the core, takes every hit.
        let drag = 0.95f32.powf(k);
        for pi in 0..self.parts.len() {
            let rnd = (self.rand() * 2.0 - 1.0, self.rand() * 2.0 - 1.0);
            let p = &mut self.parts[pi];
            for imp in &impulses {
                let (dx, dy) = imp.at((p.x, p.y), (cx, cy), h, rnd);
                p.vx += dx;
                p.vy += dy;
            }
            p.x += p.vx * k;
            p.y += p.vy * k;
            p.vx *= drag;
            p.vy *= drag;
            let (dx, dy) = (p.x - cx, p.y - cy);
            p.x += -dy * 0.004 * motion * k;
            p.y += dx * 0.004 * motion * k;
            p.age += dt;
            let out = p.x < -20.0 || p.y < -20.0 || p.x > w + 20.0 || p.y > h + 20.0;
            if p.age >= p.life || out {
                self.parts[pi] = self.new_particle();
            }
        }
        for p in &self.parts {
            let fade = (PI * p.age / p.life).sin();
            self.layers
                .dot(false, (p.x, p.y), self.pal[p.col], look.dust * fade, 1);
        }

        // Sustained lanes: melodies as ribbons whose head height is the
        // pitch, chords as bands of rings around the core.
        let mut comet: Option<(Pt, f32)> = None;
        for li in 0..self.lanes.len() {
            let held: Vec<Note> = self.lanes[li].held_at(t, dt).copied().collect();
            let in_phrase = self.lanes[li].in_phrase(t);
            let lead = self.lanes[li].lead;
            let Some(rb) = self.lanes[li].ribbon.as_mut() else {
                continue;
            };
            let release = if in_phrase {
                PHRASE_RELEASE
            } else {
                look.release
            };
            match rb.form {
                Form::Melody(band) => {
                    // The latest note sounding is the one the line is on.
                    let (goal, tau) = match held.last() {
                        Some(n) => {
                            rb.y.retarget(melody_y(rb, n.pitch as f32, h));
                            (0.55 + 0.45 * n.vel, 0.12)
                        }
                        None => (0.0, release),
                    };
                    rb.level = goal + (rb.level - goal) * (-dt / tau).exp();
                    rb.y.g += dt;
                    rb.phase += dt * 0.25;
                    rb.head = (melody_x(band, w) + 0.01 * w * rb.phase.sin(), rb.y.value());
                    let flow = TRAIL_SPAN * w / look.trail;
                    for pt in rb.trail.iter_mut() {
                        for imp in impulses.iter().filter(|i| i.drum != Drum::Hat) {
                            let (dx, dy) = imp.at((pt.x, pt.y), (cx, cy), h, (0.0, 0.0));
                            pt.vx += dx * RIBBON_PUSH;
                            pt.vy += dy * RIBBON_PUSH;
                        }
                        pt.x += pt.vx * k - flow * dt;
                        pt.y += pt.vy * k - TRAIL_RISE * h * dt;
                        pt.vx *= drag;
                        pt.vy *= drag;
                        pt.age += dt;
                    }
                    while rb.trail.front().is_some_and(|p| p.age > look.trail) {
                        rb.trail.pop_front();
                    }
                    if rb.level > 0.01 {
                        rb.trail.push_back(TrailPt {
                            x: rb.head.0,
                            y: rb.head.1,
                            vx: 0.0,
                            vy: 0.0,
                            age: 0.0,
                            a: rb.level,
                        });
                    }
                    let col = mix(self.pal[MELODY_COLOUR[band]], [1.0, 1.0, 1.0], 0.3);
                    rb.drawn = draw_melody(&mut self.layers, rb, band, col, look.trail, dt, u);
                    if lead {
                        comet = Some((rb.head, rb.level));
                    }
                }
                Form::Chord(ring) => {
                    // The chord: the notes sounding that started together
                    // with the latest one.
                    let latest = held.iter().map(|n| n.t).fold(f32::NEG_INFINITY, f32::max);
                    let mut tones: Vec<u8> = held
                        .iter()
                        .filter(|n| n.t >= latest - lanes::ONSET_TOGETHER)
                        .map(|n| n.pitch)
                        .collect();
                    tones.sort_unstable();
                    let (goal, tau) = match (tones.first(), tones.last()) {
                        (Some(&a), Some(&b)) => {
                            let vel = held.iter().map(|n| n.vel).fold(0.0, f32::max);
                            rb.r_lo.retarget(chord_r(ring, h, a as f32 - rb.lo));
                            rb.r_hi.retarget(chord_r(ring, h, b as f32 - rb.lo));
                            rb.tones = tones;
                            (0.5 + 0.5 * vel, 0.15)
                        }
                        _ => (0.0, release),
                    };
                    rb.level = goal + (rb.level - goal) * (-dt / tau).exp();
                    rb.r_lo.g += dt;
                    rb.r_hi.g += dt;
                    let dir = if ring % 2 == 0 { 1.0 } else { -1.0 };
                    rb.phase += dt * 0.10 * dir;
                    rb.head = (cx, cy - rb.r_hi.value() * 0.62);
                    let col = self.pal[CHORD_COLOUR[ring % CHORD_COLOUR.len()]];
                    rb.drawn = draw_chord(&mut self.layers, rb, (cx, cy), col);
                }
            }
        }

        // Rings: shockwaves and section changes.
        self.rings.retain(|rg| t - rg.born <= rg.life);
        for ri in 0..self.rings.len() {
            let rg = &self.rings[ri];
            let age = t - rg.born;
            if age < 0.0 {
                continue;
            }
            let rad = 2.0 + rg.max_r * (1.0 - (-age * 4.0 / rg.life).exp());
            let a = rg.a * (1.0 - age / rg.life).powi(2) * 0.5;
            let pts = (rad * 0.9) as usize + 24;
            let (x, y, col) = (rg.x, rg.y, rg.col);
            for p in 0..pts {
                let th = p as f32 / pts as f32 * TAU;
                let at = (x + rad * th.cos(), y + rad * th.sin());
                self.layers.dot(false, at, col, a, 1);
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
                    self.layers.line(true, prev, at, col, 0.35 + 0.4 * env);
                    self.layers.line(false, prev, at, col, 0.09);
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
                self.layers.line(true, inner, tip, col, 0.18 + 0.4 * v);
                self.layers.dot(false, tip, col, 0.12 * v, 2);
            }
        }
        // Lead: a comet. It rides the lead's ribbon when the lead has one;
        // otherwise it orbits the core while a lead note is held, radius
        // from the note.
        let comet = comet.or_else(|| {
            let li = self.lanes.iter().position(|l| l.lead)?;
            let held = self.lanes[li].held_at(t, dt).map(|n| n.pitch).max();
            let on = held.is_some();
            let rate = if on { 0.25 } else { 0.05 };
            let goal = if on { 1.0 } else { 0.0 };
            self.lead_lvl += (goal - self.lead_lvl) * per_frame(rate);
            if let Some(p) = held {
                self.lead_note = p as f32;
            }
            self.lead_ang += dt * (1.3 + 0.5 * energy);
            let rr = base * 2.3 + (self.lead_note - 64.0) * 9.0 * u;
            let at = (
                cx + rr * self.lead_ang.cos(),
                cy + rr * self.lead_ang.sin() * 0.62,
            );
            Some((at, self.lead_lvl))
        });
        if let Some((at, lvl)) = comet.filter(|&(_, l)| l > 0.02) {
            let col = mix(self.pal[3], [1.0, 1.0, 1.0], 0.35);
            self.layers.dot(false, at, col, 0.55 * lvl, 2);
            self.layers.dot(true, at, [1.0, 1.0, 1.0], 0.8 * lvl, 2);
        }
    }

    /// The flash added to the whole frame this frame.
    fn flash(&self) -> f32 {
        self.flash * self.mood.look().flash
    }

    /// Each sustained lane's name and where to write it, in drawing pixels:
    /// right of a melody band's head, above its band; at the upper right of
    /// a chord's outermost ring.
    pub fn labels(&self) -> Vec<(String, Pt)> {
        let (w, h) = (self.w, self.h);
        let u = h / 1080.0;
        self.lanes
            .iter()
            .filter_map(|l| {
                let rb = l.ribbon.as_ref()?;
                let at = match rb.form {
                    Form::Melody(b) => {
                        let (centre, span) = MELODY_BANDS[b];
                        (melody_x(b, w) + 24.0 * u, h * (centre - span * 0.5))
                    }
                    Form::Chord(c) => {
                        let r = chord_r(c, h, rb.hi - rb.lo);
                        let d = FRAC_PI_4;
                        (
                            w * 0.5 + r * d.cos(),
                            h * 0.42 - r * d.sin() * 0.62 - 24.0 * u,
                        )
                    }
                };
                Some((l.name.clone(), at))
            })
            .collect()
    }

    /// Each melody ribbon's lane name, head position in drawing pixels and
    /// level.
    pub fn melody_heads(&self) -> Vec<(&str, Pt, f32)> {
        self.lanes
            .iter()
            .filter_map(|l| {
                let rb = l.ribbon.as_ref()?;
                matches!(rb.form, Form::Melody(_)).then_some((l.name.as_str(), rb.head, rb.level))
            })
            .collect()
    }
}

/// Palette entry of each melody band and chord ring, so lanes on screen
/// together differ in colour.
const MELODY_COLOUR: [usize; MAX_RIBBONS] = [3, 0, 2, 1];
const CHORD_COLOUR: [usize; 2] = [1, 2];

/// A melody band's head position across the frame.
fn melody_x(band: usize, w: f32) -> f32 {
    w * (0.84 - 0.05 * band as f32)
}

/// A melody head's height for `pitch`: up with pitch, over the lane's own
/// range, never narrower than [`lanes::MELODY_MIN_RANGE`].
fn melody_y(rb: &Ribbon, pitch: f32, h: f32) -> f32 {
    let Form::Melody(band) = rb.form else {
        unreachable!("melody_y on a chord")
    };
    let (centre, span) = MELODY_BANDS[band];
    let range = (rb.hi - rb.lo).max(lanes::MELODY_MIN_RANGE);
    let mid = (rb.hi + rb.lo) * 0.5;
    h * (centre - (pitch - mid) / range * span)
}

/// A chord ring's radius `semis` semitones above the lane's lowest note.
fn chord_r(ring: usize, h: f32, semis: f32) -> f32 {
    h * (CHORD_R0 + CHORD_STEP * ring as f32 + CHORD_SEMI * semis)
}

/// Draws a melody's trail as one smooth line, thicker for the first bands.
/// Returns the segments drawn.
fn draw_melody(
    layers: &mut Layers,
    rb: &Ribbon,
    band: usize,
    col: [f32; 3],
    life: f32,
    dt: f32,
    u: f32,
) -> usize {
    let strands: &[(f32, f32)] = match band {
        0 => &[(0.0, 0.34), (1.8, 0.16), (-1.8, 0.16)],
        1 => &[(0.0, 0.30), (1.2, 0.12)],
        _ => &[(0.0, 0.26)],
    };
    let mut drawn = 0;
    for (p0, p1) in rb.trail.iter().zip(rb.trail.iter().skip(1)) {
        // A gap where the ribbon was silent is not bridged.
        if p0.age - p1.age > 2.5 * dt {
            continue;
        }
        let f = (1.0 - p1.age / life).max(0.0).powf(1.5) * p1.a;
        if f < 0.004 {
            continue;
        }
        let (dx, dy) = (p1.x - p0.x, p1.y - p0.y);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (nx, ny) = (-dy / len * u, dx / len * u);
        for &(off, a) in strands {
            let q0 = (p0.x + nx * off, p0.y + ny * off);
            let q1 = (p1.x + nx * off, p1.y + ny * off);
            layers.line(true, q0, q1, col, a * f);
        }
        layers.line(false, (p0.x, p0.y), (p1.x, p1.y), col, 0.06 * f);
        drawn += 1;
    }
    drawn
}

/// Draws a chord as a band around the core `c`: a dotted ring per chord
/// tone, spaced by the voicing, and a soft haze between the outer ones. It
/// swells with the chord's level. Returns the dots drawn.
fn draw_chord(layers: &mut Layers, rb: &Ribbon, c: Pt, col: [f32; 3]) -> usize {
    if rb.level < 0.004 || rb.tones.is_empty() {
        return 0;
    }
    let swell = 0.94 + 0.06 * rb.level;
    let (r_lo, r_hi) = (rb.r_lo.value() * swell, rb.r_hi.value() * swell);
    let (p_lo, p_hi) = (rb.tones[0] as f32, rb.tones[rb.tones.len() - 1] as f32);
    let at = |r: f32, th: f32| (c.0 + r * th.cos(), c.1 + r * th.sin() * 0.62);
    let mut drawn = 0;
    for (i, &p) in rb.tones.iter().enumerate() {
        let r = r_lo + (p as f32 - p_lo) / (p_hi - p_lo).max(1.0) * (r_hi - r_lo);
        let n = (r * 0.22) as usize + 20;
        let turn = rb.phase * (1.0 + 0.3 * i as f32) + i as f32 * 0.9;
        for j in 0..n {
            let th = turn + j as f32 * TAU / n as f32;
            layers.dot(true, at(r, th), col, 0.30 * rb.level, 1);
            drawn += 1;
        }
    }
    if r_hi - r_lo > 1.0 {
        for j in 0..160 {
            let r = r_lo + (j as f32 * 0.618_034).fract() * (r_hi - r_lo);
            let th = rb.phase * 0.7 + j as f32 * 2.399_963;
            layers.dot(false, at(r, th), col, 0.05 * rb.level, 2);
            drawn += 1;
        }
    }
    drawn
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
  if (aKind.x > 0.5) {
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
  } else {
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
        mood: Mood,
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
            state: AvProof::new(sidecar, w, h, sample_rate, mood),
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

    /// [`AvProof::labels`] in output pixels.
    pub fn labels(&self) -> Vec<(String, (f32, f32))> {
        let s = self.ss as f32;
        self.state
            .labels()
            .into_iter()
            .map(|(n, (x, y))| (n, (x / s, y / s)))
            .collect()
    }

    /// [`AvProof::melody_heads`] in output pixels.
    pub fn melody_heads(&self) -> Vec<(&str, (f32, f32), f32)> {
        let s = self.ss as f32;
        self.state
            .melody_heads()
            .into_iter()
            .map(|(n, (x, y), l)| (n, (x / s, y / s), l))
            .collect()
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
                self.state.flash(),
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

    /// Beats every 0.5 s and bars every 2 s over 20 s, one section ending at
    /// 10 s, with `lanes`.
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

    /// Notes as (t, pitch, velocity, length).
    fn lane(name: &str, notes: &[(f64, u8, u8, f64)]) -> serde_json::Value {
        let events: Vec<_> = notes
            .iter()
            .map(|&(t, pitch, velocity, length)| {
                serde_json::json!({"t": t, "pitch": pitch, "velocity": velocity, "length": length})
            })
            .collect();
        serde_json::json!({"name": name, "kind": "midi", "events": events})
    }

    /// A held line: one-second notes running into each other.
    fn flute(from: f64, n: usize) -> Vec<(f64, u8, u8, f64)> {
        (0..n)
            .map(|i| (from + i as f64, [60, 64, 67, 62][i % 4], 100, 1.0))
            .collect()
    }

    /// Lanes shaped like afterimage's: a kick, drum hits, hats, two held
    /// synth lines and long one-shots.
    fn afterimage_like() -> serde_json::Value {
        let every = |step: f64, len: f64, pitch: u8| -> Vec<(f64, u8, u8, f64)> {
            (0..(8.0 / step) as usize)
                .map(|i| (i as f64 * step, pitch, 110, len))
                .collect()
        };
        serde_json::json!([
            lane("Generator", &every(0.5, 0.25, 36)),
            lane("Perc 1 · BD", &every(1.0, 0.06, 36)),
            lane("Perc 1 · SN", &every(1.0, 0.06, 38)),
            lane("Perc 1 · OH", &every(0.5, 0.06, 46)),
            lane("Perc 1 · TOM1", &every(2.0, 0.06, 45)),
            lane("Hats", &every(0.125, 0.06, 42)),
            lane("MAIN SYNTH", &flute(0.0, 8)),
            lane("SECONDARY SYNTH/CHRONOS", &flute(0.5, 7)),
            lane(
                "FX / ONE-SHOTS",
                &[(0.0, 48, 90, 3.5), (4.0, 55, 90, 3.5), (6.0, 52, 90, 1.5)]
            ),
        ])
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

    fn new(sc: &Sidecar) -> AvProof {
        AvProof::new(sc, 1920, 1080, 48000.0, Mood::Bright)
    }

    fn ribbon(av: &AvProof, name_notes: usize) -> &Ribbon {
        av.lanes
            .iter()
            .find(|l| l.notes.len() == name_notes)
            .and_then(|l| l.ribbon.as_ref())
            .expect("a ribbon")
    }

    /// Mean dust speed, pixels per 60 fps frame.
    fn dust_speed(av: &AvProof) -> f32 {
        av.parts
            .iter()
            .map(|p| (p.vx * p.vx + p.vy * p.vy).sqrt())
            .sum::<f32>()
            / av.parts.len() as f32
    }

    /// No lane is drawn as a row: no rectangles at all, and no dozen
    /// primitives lined up on one horizontal line.
    #[test]
    fn no_lane_is_drawn_as_a_row() {
        let mut av = new(&sidecar(afterimage_like()));
        for i in 0..=240 {
            run(&mut av, i, i);
            for layer in [&av.layers.acc, &av.layers.crisp] {
                assert!(
                    layer.iter().all(|p| p.kind[0] < 1.5),
                    "only dots and lines (frame {i})"
                );
                let mut ys: Vec<i64> = layer
                    .iter()
                    .filter(|p| p.kind[0] < 0.5 || (p.seg[1] - p.seg[3]).abs() < 0.01)
                    .map(|p| (p.seg[1] * 100.0).round() as i64)
                    .collect();
                ys.sort_unstable();
                let longest = ys
                    .chunk_by(|a, b| a == b)
                    .map(|c| c.len())
                    .max()
                    .unwrap_or(0);
                assert!(longest < 12, "{longest} primitives on one row (frame {i})");
            }
        }
        // The synth lines became ribbons, the drums did not.
        let ribbons: Vec<usize> = av
            .lanes
            .iter()
            .filter(|l| l.ribbon.is_some())
            .map(|l| l.notes.len())
            .collect();
        assert_eq!(ribbons.len(), 3, "MAIN, SECONDARY and FX: {ribbons:?}");
    }

    /// A held line keeps its ribbon present and moving on every frame: the
    /// head stays on the held pitch, so the trail is what flows.
    #[test]
    fn held_notes_keep_the_ribbon_alive_and_moving() {
        let sc = sidecar(serde_json::json!([lane("MAIN SYNTH", &flute(1.0, 4))]));
        let mut av = new(&sc);
        run(&mut av, 0, 35);
        // Frames 36..147 are 1.2 s to 4.9 s, inside the four notes.
        for i in 36..=147 {
            let last = ribbon(&av, 4).trail.back().map(|p| (p.x, p.y)).unwrap();
            run(&mut av, i, i);
            let rb = ribbon(&av, 4);
            assert!(rb.level >= 0.5, "level {} at frame {i}", rb.level);
            assert!(rb.drawn > 0, "nothing drawn at frame {i}");
            // The point laid last frame, one frame on.
            let p = &rb.trail[rb.trail.len() - 2];
            let moved = ((p.x - last.0).powi(2) + (p.y - last.1).powi(2)).sqrt();
            assert!(moved > 0.5, "trail moved {moved} px at frame {i}");
        }
        // One trail point for every frame, none missing.
        let rb = ribbon(&av, 4);
        assert!(rb.trail.len() > 40, "a long trail: {}", rb.trail.len());
        for (a, b) in rb.trail.iter().zip(rb.trail.iter().skip(1)) {
            assert!(
                (a.age - b.age - DT).abs() < 1e-3,
                "trail gap: {} s",
                a.age - b.age
            );
        }
    }

    /// A melody's head height is its pitch: settled heights are ordered
    /// exactly as the pitches, a held note keeps the head still, and a
    /// glide between two notes never turns back.
    #[test]
    fn melody_head_height_follows_pitch_smoothly() {
        let tune = [60u8, 64, 67, 72, 67, 64, 60];
        let notes: Vec<_> = tune
            .iter()
            .enumerate()
            .map(|(i, &p)| (1.0 + i as f64 * 0.5, p, 100, 0.5))
            .collect();
        let sc = sidecar(serde_json::json!([lane("Flute", &notes)]));
        let mut av = new(&sc);
        let span = MELODY_BANDS[0].1 * 1080.0;
        let mut settled = Vec::new();
        let mut prev = ribbon(&av, tune.len()).head.1;
        let mut frame = 0;
        for k in 0..tune.len() {
            let onset = 1.0 + k as f32 * 0.5;
            let mut dir = 0.0f32;
            while (frame as f32 * DT) < onset + 0.5 - 1e-4 {
                run(&mut av, frame, frame);
                let t = frame as f32 * DT;
                let y = ribbon(&av, tune.len()).head.1;
                let dy = y - prev;
                if t >= onset - 1e-4 && t < onset + GLIDE + DT && dy.abs() > 1e-4 {
                    assert!(dir * dy >= 0.0, "glide turned back at {t} s");
                    dir = dy.signum();
                }
                if t >= onset + GLIDE + DT {
                    assert!(dy.abs() < 0.02 * span, "held note moved {dy} px at {t} s");
                }
                prev = y;
                frame += 1;
            }
            settled.push(prev);
        }
        for (i, (&pi, &yi)) in tune.iter().zip(&settled).enumerate() {
            for (&pj, &yj) in tune.iter().zip(&settled).skip(i + 1) {
                match pi.cmp(&pj) {
                    std::cmp::Ordering::Greater => assert!(yi < yj - 1.0, "{pi} not above {pj}"),
                    std::cmp::Ordering::Less => assert!(yi > yj + 1.0, "{pi} not below {pj}"),
                    std::cmp::Ordering::Equal => assert!((yi - yj).abs() < 0.5, "{pi} twice"),
                }
            }
        }
        // The lane's range covers its band.
        let top = settled.iter().copied().fold(f32::MAX, f32::min);
        let bottom = settled.iter().copied().fold(0.0, f32::max);
        assert!(
            bottom - top > 0.8 * span,
            "range {} px of {span}",
            bottom - top
        );
    }

    /// A narrow lane still moves over a visible share of its band, and a
    /// single pitch is a flat line.
    #[test]
    fn narrow_and_single_pitch_lanes() {
        let narrow: Vec<_> = (0..6)
            .map(|i| (1.0 + i as f64, [60u8, 61, 62][i % 3], 100, 1.0))
            .collect();
        let sc = sidecar(serde_json::json!([lane("Narrow", &narrow)]));
        let mut av = new(&sc);
        // Settled on 60 (1-2 s), then on 62 (3-4 s).
        run(&mut av, 0, 50);
        let low = ribbon(&av, 6).head.1;
        run(&mut av, 51, 110);
        let high = ribbon(&av, 6).head.1;
        let span = MELODY_BANDS[0].1 * 1080.0;
        let want = 2.0 / lanes::MELODY_MIN_RANGE * span;
        assert!(
            (low - high - want).abs() < 1.0,
            "{} of {want} px",
            low - high
        );

        let drone: Vec<_> = (0..4)
            .map(|i| (1.0 + i as f64 * 2.0, 57u8, 100, 2.0))
            .collect();
        let sc = sidecar(serde_json::json!([lane("Drone", &drone)]));
        let mut av = new(&sc);
        let centre = MELODY_BANDS[0].0 * 1080.0;
        for i in 0..240 {
            run(&mut av, i, i);
            let y = ribbon(&av, 4).head.1;
            assert!(
                (y - centre).abs() < 1e-3,
                "a drone moved to {y} at frame {i}"
            );
        }
    }

    /// A chord lane is classified chordal and drawn as dotted rings, where
    /// the melody is drawn as lines, and its band widens with the voicing.
    #[test]
    fn chords_are_rings_that_spread_with_the_voicing() {
        // One chord a bar: spans of 4, 7, 12 then 7 semitones.
        let voicings: [[u8; 3]; 4] = [[60, 62, 64], [60, 64, 67], [60, 67, 72], [62, 66, 69]];
        let chords: Vec<_> = voicings
            .iter()
            .enumerate()
            .flat_map(|(i, v)| v.map(|p| (1.0 + i as f64 * 2.0, p, 100, 2.0)))
            .collect();
        let sc = sidecar(serde_json::json!([
            lane("Pads", &chords),
            lane("Flute", &flute(1.0, 8)),
        ]));
        let mut av = new(&sc);
        let pads = av.lanes.iter().find(|l| l.name == "Pads").unwrap();
        assert_eq!(pads.character, Character::Chordal);
        assert!(matches!(pads.ribbon.as_ref().unwrap().form, Form::Chord(_)));
        let melody = av.lanes.iter().find(|l| l.name == "Flute").unwrap();
        assert!(matches!(
            melody.ribbon.as_ref().unwrap().form,
            Form::Melody(_)
        ));

        // Each spread settled, late in its bar.
        let mut spread = Vec::new();
        let mut from = 0;
        for bar in 0..4 {
            let to = ((2.8 + bar as f32 * 2.0) / DT) as usize;
            run(&mut av, from, to);
            from = to + 1;
            let rb = ribbon(&av, 12);
            spread.push(rb.r_hi.value() - rb.r_lo.value());
        }
        assert!(spread[0] < spread[1] && spread[1] < spread[2], "{spread:?}");
        assert!((spread[1] - spread[3]).abs() < 0.5, "same span: {spread:?}");

        // The two forms' primitives: chord dots, melody lines.
        let fresh = || Layers {
            acc: Vec::new(),
            crisp: Vec::new(),
            gain: [1.0, 1.0],
        };
        let kinds =
            |l: &Layers| -> Vec<f32> { l.acc.iter().chain(&l.crisp).map(|p| p.kind[0]).collect() };
        let mut chord = fresh();
        assert!(draw_chord(&mut chord, ribbon(&av, 12), (960.0, 450.0), [1.0; 3]) > 0);
        assert!(kinds(&chord).iter().all(|&k| k == 0.0), "chords are dots");
        let mut line = fresh();
        assert!(draw_melody(&mut line, ribbon(&av, 8), 0, [1.0; 3], 1.6, DT, 1.0) > 0);
        let k = kinds(&line);
        assert!(
            k.iter().filter(|&&k| k == 1.0).count() * 2 > k.len(),
            "melody is lines"
        );
    }

    /// Every sustained lane has a label at its own place in the frame;
    /// drums have none.
    #[test]
    fn sustained_lanes_have_labels() {
        let mut av = new(&sidecar(afterimage_like()));
        run(&mut av, 0, 60);
        let labels = av.labels();
        let mut names: Vec<&str> = labels.iter().map(|(n, _)| n.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["FX / ONE-SHOTS", "MAIN SYNTH", "SECONDARY SYNTH/CHRONOS"]
        );
        for (i, (_, a)) in labels.iter().enumerate() {
            assert!(
                (0.0..1920.0).contains(&a.0) && (0.0..1080.0).contains(&a.1),
                "{a:?}"
            );
            for (_, b) in labels.iter().skip(i + 1) {
                assert!((a.0 - b.0).abs() + (a.1 - b.1).abs() > 40.0, "{a:?} {b:?}");
            }
        }
    }

    /// A kick moves the dust and the ribbon's trail within one frame, and
    /// the push dies away.
    #[test]
    fn a_hit_pushes_what_is_there_and_decays() {
        let sc = sidecar(serde_json::json!([
            lane("KICK", &[(2.01, 36, 127, 0.25)]),
            lane("MAIN SYNTH", &flute(1.0, 4)),
        ]));
        let mut av = new(&sc);
        run(&mut av, 0, 60);
        let before = dust_speed(&av);
        let trail_before: f32 = ribbon(&av, 4)
            .trail
            .iter()
            .map(|p| p.vx.abs() + p.vy.abs())
            .sum();
        assert!(before < 0.01, "dust drifts without hits: {before}");
        assert_eq!(trail_before, 0.0);
        run(&mut av, 61, 61);
        let hit = dust_speed(&av);
        assert!(hit > 0.2, "the kick moved the dust: {hit}");
        let trail_hit: f32 = ribbon(&av, 4)
            .trail
            .iter()
            .map(|p| p.vx.abs() + p.vy.abs())
            .sum();
        assert!(trail_hit > 0.0, "the kick moved the trail");
        run(&mut av, 62, 91);
        let after = dust_speed(&av);
        assert!(after < hit * 0.1, "the push decays: {hit} -> {after}");
    }

    /// The share of a hit's push that points away from the core.
    fn radial_share(name: &str) -> f32 {
        let sc = sidecar(serde_json::json!([lane(name, &[(2.01, 40, 127, 0.05)])]));
        let mut av = new(&sc);
        run(&mut av, 0, 60);
        let (cx, cy) = (av.w * 0.5 + av.core_off.0, av.h * 0.42 + av.core_off.1);
        let v0: Vec<Pt> = av.parts.iter().map(|p| (p.vx, p.vy)).collect();
        let pos: Vec<Pt> = av.parts.iter().map(|p| (p.x, p.y)).collect();
        run(&mut av, 61, 61);
        let (mut rad, mut all) = (0.0, 0.0);
        for ((p, v), q) in av.parts.iter().zip(v0).zip(pos) {
            let (dx, dy) = (p.vx - v.0 * 0.95f32.powi(2), p.vy - v.1 * 0.95f32.powi(2));
            let (rx, ry) = (q.0 - cx, q.1 - cy);
            let r = (rx * rx + ry * ry).sqrt().max(1.0);
            rad += ((dx * rx + dy * ry) / r).abs();
            all += (dx * dx + dy * dy).sqrt();
        }
        rad / all.max(1e-9)
    }

    /// A kick pushes outward, a snare twists around the core.
    #[test]
    fn different_drums_push_differently() {
        let kick = radial_share("KICK");
        let snare = radial_share("Clap");
        assert!(kick > 0.9, "kick push is radial: {kick}");
        assert!(snare < 0.1, "snare push is tangential: {snare}");
    }

    /// A zero-length note still pushes, in the frame it falls in.
    #[test]
    fn zero_length_note_pushes_once() {
        // A kick lane far later, so the beats do not push meanwhile.
        let sc = sidecar(serde_json::json!([
            lane("Clap", &[(1.01, 42, 127, 0.0)]),
            lane("KICK", &[(15.0, 36, 127, 0.1)]),
        ]));
        let mut av = new(&sc);
        // 1.01 s falls in frame 31's window (1.0, 1.0333].
        run(&mut av, 0, 30);
        assert_eq!(dust_speed(&av), 0.0);
        run(&mut av, 31, 31);
        assert!(dust_speed(&av) > 0.0);
    }

    /// A kick note-on starts a shockwave within one frame; nothing else does.
    #[test]
    fn kick_starts_a_shockwave() {
        let sc = sidecar(serde_json::json!([
            lane("Clap", &[(1.0, 50, 127, 0.1)]),
            lane("KICK", &[(2.01, 36, 127, 0.25)]),
        ]));
        let mut av = new(&sc);
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
            assert!(Lane::new(name, Vec::new()).kick, "{name} is a kick lane");
        }
        for name in ["Clap", "Perc 1 · BD", "MAIN SYNTH"] {
            assert!(
                !Lane::new(name, Vec::new()).kick,
                "{name} is not a kick lane"
            );
        }
    }

    /// Without lanes the beats act as one kick lane.
    #[test]
    fn without_lanes_the_beats_drive_it() {
        let mut av = new(&sidecar(serde_json::Value::Null));
        assert_eq!(av.lanes.len(), 1);
        assert!(av.lanes[0].kick);
        run(&mut av, 1, 15);
        assert!(av.rings.iter().any(|r| r.max_r == av.h * 0.55));
        assert!(dust_speed(&av) > 0.0, "the beats push the dust");
    }

    /// A lane with no notes is left out; more sustained lanes than ribbons:
    /// the busiest get them.
    #[test]
    fn empty_lanes_draw_nothing_and_ribbons_go_to_the_busiest() {
        let mut lanes = vec![lane("Empty", &[])];
        for i in 0..MAX_RIBBONS + 2 {
            lanes.push(lane(&format!("Line {i}"), &flute(0.0, 4 + i)));
        }
        let av = new(&sidecar(serde_json::json!(lanes)));
        assert_eq!(av.lanes.len(), MAX_RIBBONS + 2, "the empty lane is gone");
        let with: Vec<usize> = av
            .lanes
            .iter()
            .filter(|l| l.ribbon.is_some())
            .map(|l| l.notes.len())
            .collect();
        assert_eq!(with, vec![6, 7, 8, 9], "the busiest four");
    }

    #[test]
    fn mood_names() {
        assert_eq!(Mood::from_name("dark"), Some(Mood::Dark));
        assert_eq!(Mood::from_name("bright"), Some(Mood::Bright));
        assert_eq!(Mood::from_name("grey"), None);
        assert_eq!(Mood::default(), Mood::Bright);
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

    /// The largest rise in mean frame brightness from one frame to the
    /// next, rendering 9.5 s to 10.5 s across the section change at 10 s.
    fn brightest_jump(ctx: &GpuContext, mood: Mood) -> f64 {
        let sc = sidecar(serde_json::json!([lane("MAIN SYNTH", &flute(9.0, 3))]));
        let mut r = AvProofScene::new(ctx, 128, 64, &sc, 48000.0, mood);
        let mean = |p: &[u8]| {
            p.chunks(4)
                .map(|c| 0.2126 * c[0] as f64 + 0.7152 * c[1] as f64 + 0.0722 * c[2] as f64)
                .sum::<f64>()
                / (p.len() / 4) as f64
        };
        let mut means = Vec::new();
        for i in 270..=315 {
            let t = i as f32 * DT;
            let f = r.render(ctx, &inputs(t), &scope(t)).unwrap();
            if i >= 285 {
                means.push(mean(&f));
            }
        }
        means
            .windows(2)
            .map(|w| w[1] - w[0])
            .fold(f64::MIN, f64::max)
    }

    /// Under the dark mood a section change does not light the whole frame;
    /// under the bright one it does, so the check can tell them apart.
    #[test]
    fn dark_mood_has_no_section_flash() {
        let ctx = match GpuContext::new_blocking() {
            Ok(c) => c,
            Err(_) => return,
        };
        let dark = brightest_jump(&ctx, Mood::Dark);
        let bright = brightest_jump(&ctx, Mood::Bright);
        assert!(dark < 3.0, "dark brightness jump {dark}");
        assert!(bright > 10.0, "bright brightness jump {bright}");
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
            lane("MAIN SYNTH", &flute(0.2, 3)),
        ]));
        for mood in [Mood::Bright, Mood::Dark] {
            let mut r = AvProofScene::new(&ctx, 128, 64, &sc, 48000.0, mood);
            let mean = |p: &[u8]| p.iter().map(|&b| b as f64).sum::<f64>() / p.len() as f64;
            let mut frames = Vec::new();
            for i in 0..20 {
                let t = i as f32 * DT;
                frames.push(r.render(&ctx, &inputs(t), &scope(t)).unwrap());
            }
            let m = mean(&frames[19]);
            assert!(
                m > 1.0 && m < 240.0,
                "{mood:?} frame should be non-degenerate, mean {m}"
            );
            assert_ne!(frames[18], frames[19], "{mood:?} frames should change");
        }
    }
}
