//! Song-aware layer over the "after" look: how hard the picture reacts is
//! set by the song's own character, and the look changes with the song's
//! sections.
//!
//! Two parts. [`SongCharacter`] is measured once over the whole track: a
//! calm song gets small, slow reactions, an energetic one keeps the full
//! punch, and loudness is read relative to the song's own loud passages.
//! [`SectionTrack`] reads sections from the offline sidecar, gives each one a
//! [`SubLook`] chosen from its character alone (so a repeated section looks
//! like itself again) and eases every change over [`SECTION_EASE_SECS`].
//!
//! Nothing here scales a phase by a signal: smoothing is EMAs of bounded
//! values, and blends run on their own clock from a section boundary.

use hyst_core::sidecar::{Sidecar, SidecarSectionKind};

use super::CURATED_C;

/// Whole-track statistics [`SongCharacter`] is computed from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackFeatures {
    pub tempo_bpm: f32,
    /// Onsets per second.
    pub onset_density: f32,
    /// Median per-hop RMS level of the raw signal, dBFS. The extractor's
    /// `energy` is adaptively normalised, so every track averages about the
    /// same there; the raw level tells a quiet mix from a loud one.
    pub mean_db: f32,
    /// 90th minus 10th percentile of the per-hop RMS level, dB.
    pub dynamic_range_db: f32,
    /// 90th percentile of the extractor's `energy`: the song's own loud
    /// level, which [`SongCharacter::relative`] divides by.
    pub loud: f32,
}

fn percentile(values: &[f32], p: f32) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    if sorted.is_empty() {
        0.0
    } else {
        sorted[((sorted.len() - 1) as f32 * p).round() as usize]
    }
}

/// Onsets per second in a per-hop spectral-flux curve: local maxima of the
/// flux above its own ~0.5 s moving mean that clear the mean plus one
/// standard deviation of that detrended curve.
pub fn onset_rate(flux: &[f32], hops_per_sec: f32) -> f32 {
    if flux.len() < 3 || hops_per_sec <= 0.0 {
        return 0.0;
    }
    let half = ((hops_per_sec * 0.25) as usize).max(1);
    let mut prefix = vec![0.0f64; flux.len() + 1];
    for (i, &f) in flux.iter().enumerate() {
        prefix[i + 1] = prefix[i] + f as f64;
    }
    let detrended: Vec<f32> = (0..flux.len())
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(flux.len());
            let mean = (prefix[hi] - prefix[lo]) / (hi - lo) as f64;
            (flux[i] - mean as f32).max(0.0)
        })
        .collect();
    let n = detrended.len() as f32;
    let mean = detrended.iter().sum::<f32>() / n;
    let std = (detrended.iter().map(|d| (d - mean).powi(2)).sum::<f32>() / n).sqrt();
    let threshold = mean + std;
    let count = detrended
        .windows(3)
        .filter(|w| w[1] > threshold && w[1] >= w[0] && w[1] > w[2])
        .count();
    count as f32 / (flux.len() as f32 / hops_per_sec)
}

impl TrackFeatures {
    /// `rms_db` (raw per-hop level) and `energy` (the extractor's per-hop
    /// `energy`) cover the whole track.
    pub fn measure(rms_db: &[f32], energy: &[f32], onset_density: f32, tempo_bpm: f32) -> Self {
        Self {
            tempo_bpm,
            onset_density,
            mean_db: percentile(rms_db, 0.5),
            dynamic_range_db: percentile(rms_db, 0.9) - percentile(rms_db, 0.1),
            loud: percentile(energy, 0.9).clamp(0.0, 1.0),
        }
    }
}

/// Ranges each feature is read over, from calm to energetic. Set from the
/// owner's three tracks (afterimage, event_horizon, panicspiral; numbers
/// in the PR), so they are a starting point, not a general calibration.
const TEMPO_RANGE: [f32; 2] = [90.0, 150.0];
const ONSET_RANGE: [f32; 2] = [4.0, 6.5];
const MEAN_DB_RANGE: [f32; 2] = [-24.0, -17.0];
/// A wide dynamic range reads as calmer (quiet passages between loud ones),
/// a squashed one as relentless.
const DYNAMIC_RANGE_DB_RANGE: [f32; 2] = [19.0, 15.0];
/// Energetic scores at or above the top of this map to calm 0 (today's
/// punch), at or below the bottom to calm 1.
const CALM_SCORE_RANGE: [f32; 2] = [0.3, 0.85];

/// The song's character, measured once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SongCharacter {
    /// 0 keeps today's reactions; 1 is the calmest.
    pub calm: f32,
    /// The song's own loud level; [`Self::relative`] divides by it.
    pub loud: f32,
}

fn unit(range: [f32; 2], x: f32) -> f32 {
    ((x - range[0]) / (range[1] - range[0])).clamp(0.0, 1.0)
}

pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = unit([edge0, edge1], x);
    t * t * (3.0 - 2.0 * t)
}

impl SongCharacter {
    pub fn from_features(f: &TrackFeatures) -> Self {
        let energetic = (unit(TEMPO_RANGE, f.tempo_bpm)
            + unit(ONSET_RANGE, f.onset_density)
            + unit(MEAN_DB_RANGE, f.mean_db)
            + unit(DYNAMIC_RANGE_DB_RANGE, f.dynamic_range_db))
            / 4.0;
        Self {
            calm: 1.0 - smoothstep(CALM_SCORE_RANGE[0], CALM_SCORE_RANGE[1], energetic),
            loud: f.loud.max(1e-3),
        }
    }

    /// Energy as a fraction of the song's own loud level, 0..1.
    pub fn relative(&self, energy: f32) -> f32 {
        (energy / self.loud).clamp(0.0, 1.0)
    }

    /// Multiplier on the size of fast reactions (beat flash, jitter, bloom,
    /// fold): 1 at calm 0.
    pub fn react(&self) -> f32 {
        1.0 - 0.85 * self.calm
    }

    /// Multiplier on reaction time constants: 1 at calm 0.
    pub fn slow(&self) -> f32 {
        1.0 + 4.0 * self.calm
    }
}

/// Per-frame signals after the song's character has been applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reactions {
    /// Relative loudness for the Julia driver (dive rate, orbit, shimmer,
    /// energy jumps), scaled by `react`.
    pub drive_energy: f32,
    /// Smoothed relative loudness for the beam's size.
    pub beam_energy: f32,
    /// Smoothed relative loudness scaled by `react`: bloom and fold.
    pub punch_energy: f32,
    /// Smoothed `sub`, scaled by `react`: the memory field's flow warp.
    pub sub: f32,
    /// Smoothed colour features for the palette.
    pub chroma_hue: f32,
    pub centroid: f32,
    pub flatness: f32,
}

/// Smooths the bus by the song's character: time constants grow with calm,
/// and at calm 0 the beam, bloom and fold read the raw relative loudness.
#[derive(Clone, Copy, Debug)]
pub struct Reactor {
    pub character: SongCharacter,
    level: f32,
    sub: f32,
    hue: f32,
    centroid: f32,
    flatness: f32,
    primed: bool,
}

impl Reactor {
    pub fn new(character: SongCharacter) -> Self {
        Self {
            character,
            level: 0.0,
            sub: 0.0,
            hue: 0.0,
            centroid: 0.5,
            flatness: 0.3,
            primed: false,
        }
    }

    pub fn update(&mut self, dt: f32, bus: &hyst_core::SignalBus) -> Reactions {
        let ch = self.character;
        let calm = ch.calm;
        let ema = |cur: f32, target: f32, tc: f32| cur + (target - cur) * (dt / tc).min(1.0);
        let rel = ch.relative(bus.energy);
        let hue_target = bus.chroma_root_hue.rem_euclid(1.0);
        if !self.primed {
            self.primed = true;
            self.level = rel;
            self.sub = bus.sub.clamp(0.0, 1.0);
            self.hue = hue_target;
            self.centroid = bus.centroid.clamp(0.0, 1.0);
            self.flatness = bus.flatness.clamp(0.0, 1.0);
        }
        // Zero time constant at calm 0: the raw value, as today.
        self.level = ema(self.level, rel, 0.8 * calm);
        self.sub = ema(self.sub, bus.sub.clamp(0.0, 1.0), 0.8 * calm);
        self.centroid = ema(self.centroid, bus.centroid.clamp(0.0, 1.0), 3.0 * calm);
        self.flatness = ema(self.flatness, bus.flatness.clamp(0.0, 1.0), 3.0 * calm);
        let mut diff = hue_target - self.hue;
        if diff > 0.5 {
            diff -= 1.0;
        } else if diff < -0.5 {
            diff += 1.0;
        }
        self.hue = (self.hue + diff * (dt / (4.0 * calm)).min(1.0)).rem_euclid(1.0);
        let react = ch.react();
        Reactions {
            drive_energy: rel * react,
            beam_energy: self.level,
            punch_energy: self.level * react,
            sub: self.sub * react,
            chroma_hue: self.hue,
            centroid: self.centroid,
            flatness: self.flatness,
        }
    }
}

/// Seconds a section change takes to ease in.
pub const SECTION_EASE_SECS: f32 = 4.0;
/// Shortest section kept; boundaries closer than this are merged.
pub const MIN_SECTION_SECS: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeamStyle {
    Off,
    /// The Lissajous trace with no waveform jitter.
    Lissajous,
    /// The Lissajous trace riding the waveform, as today.
    Oscilloscope,
}

/// A palette preset: hue offset added to the palette phase and a tint
/// multiplied into the accent colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub hue: f32,
    pub tint: [f32; 3],
}

pub const PALETTES: [Palette; 4] = [
    // Neutral: today's colours.
    Palette {
        hue: 0.0,
        tint: [1.0, 1.0, 1.0],
    },
    // Cool blue.
    Palette {
        hue: 0.58,
        tint: [0.55, 0.75, 1.0],
    },
    // Warm amber.
    Palette {
        hue: 0.08,
        tint: [1.0, 0.75, 0.5],
    },
    // Violet.
    Palette {
        hue: 0.82,
        tint: [0.85, 0.6, 1.0],
    },
];

/// What a section looks like.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubLook {
    /// Index into [`CURATED_C`].
    pub entry: usize,
    /// Index into [`PALETTES`].
    pub palette: usize,
    /// Memory-field decay floor: higher holds longer, softer trails.
    pub decay: f32,
    pub beam: BeamStyle,
    /// Whether the kaleidoscope fold is earned in this section.
    pub fold: bool,
}

/// How loud a section is relative to the song: 0 quiet, 1 mid, 2 loud.
pub fn tier(kind: SidecarSectionKind, level: f32) -> u8 {
    match kind {
        SidecarSectionKind::Break => u8::from(level >= 0.75),
        SidecarSectionKind::Drop => 1 + u8::from(level >= 0.75),
        SidecarSectionKind::Build | SidecarSectionKind::Other => {
            if level < 0.5 {
                0
            } else if level < 0.75 {
                1
            } else {
                2
            }
        }
    }
}

/// The sub-look for a section of `kind` whose mean energy ranks at `level`
/// in the song's own energy, in a song of the given calm. A pure function, so two
/// sections of the same character always look the same.
pub fn choose_sublook(kind: SidecarSectionKind, level: f32, calm: f32) -> SubLook {
    let is_break = kind == SidecarSectionKind::Break;
    match tier(kind, level) {
        0 => SubLook {
            entry: if is_break { 3 } else { 0 },
            palette: 1,
            decay: 0.93,
            beam: if is_break {
                BeamStyle::Off
            } else {
                BeamStyle::Lissajous
            },
            fold: false,
        },
        1 => SubLook {
            entry: if is_break { 0 } else { 1 },
            palette: if is_break { 3 } else { 0 },
            decay: 0.9,
            beam: BeamStyle::Lissajous,
            fold: false,
        },
        _ => SubLook {
            entry: 2,
            palette: 2,
            decay: 0.86,
            beam: if calm > 0.5 {
                BeamStyle::Lissajous
            } else {
                BeamStyle::Oscilloscope
            },
            fold: !is_break && calm < 0.5,
        },
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub start: f32,
    pub end: f32,
    pub kind: SidecarSectionKind,
    /// Where the section's mean energy ranks in the whole song's energy
    /// envelope, stretched so the song's quietest section is 0 and its
    /// loudest 1.
    pub level: f32,
    pub look: SubLook,
    /// The earlier section this one repeats, whose look it reuses.
    pub repeats: Option<usize>,
}

/// Peaks of a novelty envelope above median + 2 MAD that are the largest
/// value within `spacing` samples either side, as `scripts/ssm.ts`'s
/// `pickPeaks` does on the beat grid.
fn novelty_peaks(novelty: &[f32], spacing: usize) -> Vec<usize> {
    if novelty.is_empty() {
        return Vec::new();
    }
    let median = |v: &mut Vec<f32>| -> f32 {
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let med = median(&mut novelty.to_vec());
    let mad = median(&mut novelty.iter().map(|x| (x - med).abs()).collect());
    let threshold = med + 2.0 * mad;
    let mut peaks: Vec<usize> = Vec::new();
    for (i, &x) in novelty.iter().enumerate() {
        // Strictly above, so a flat stretch (MAD 0) is not a run of peaks.
        if x <= threshold {
            continue;
        }
        let lo = i.saturating_sub(spacing);
        let hi = (i + spacing).min(novelty.len() - 1);
        if novelty[lo..=hi].iter().any(|&y| y > x) {
            continue;
        }
        if peaks.last().is_none_or(|&p| i - p >= spacing) {
            peaks.push(i);
        }
    }
    peaks
}

fn overlap(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.1.min(b.1) - a.0.max(b.0)).max(0.0)
}

/// Sections for the whole track: boundaries are the sidecar's own section
/// edges, its repeat spans and its section-novelty peaks, merged so no
/// section is shorter than [`MIN_SECTION_SECS`]. Each takes the kind of the
/// sidecar section covering its middle (or `Other`), and a section that the
/// sidecar's repeat map says repeats an earlier one reuses that one's look.
pub fn sections_from_sidecar(sidecar: &Sidecar, calm: f32) -> Vec<Section> {
    let duration = sidecar.duration as f32;
    let rate = sidecar.envelope_rate.max(1e-3);
    let mut points: Vec<f32> = Vec::new();
    for s in &sidecar.sections {
        points.push(s.start as f32);
        points.push(s.end as f32);
    }
    for r in sidecar.repeats.iter().flatten() {
        points.extend([r.a_start, r.a_end, r.b_start, r.b_end].map(|x| x as f32));
    }
    if let Some(novelty) = &sidecar.novelty_section_envelope {
        let spacing = (MIN_SECTION_SECS * rate) as usize;
        points.extend(
            novelty_peaks(novelty, spacing)
                .into_iter()
                .map(|i| i as f32 / rate),
        );
    }
    points.sort_by(f32::total_cmp);
    let mut edges = vec![0.0];
    for p in points {
        if p - edges[edges.len() - 1] >= MIN_SECTION_SECS && duration - p >= MIN_SECTION_SECS {
            edges.push(p);
        }
    }
    edges.push(duration);

    let envelope = &sidecar.energy_envelope;
    let mut sorted = envelope.clone();
    sorted.sort_by(f32::total_cmp);

    let mut sections: Vec<Section> = Vec::new();
    for w in edges.windows(2) {
        let (start, end) = (w[0], w[1]);
        let mid = 0.5 * (start + end) as f64;
        let kind = sidecar
            .sections
            .iter()
            .find(|s| s.start <= mid && mid < s.end)
            .map_or(SidecarSectionKind::Other, |s| s.kind);
        let lo = ((start * rate) as usize).min(envelope.len());
        let hi = ((end * rate) as usize).clamp(lo, envelope.len());
        let span = &envelope[lo..hi];
        // Rank within the song's own envelope, not a fraction of its peak:
        // section means bunch together, ranks spread over 0..1 in any song.
        let level = if span.is_empty() || sorted.is_empty() {
            0.0
        } else {
            let mean = span.iter().sum::<f32>() / span.len() as f32;
            sorted.partition_point(|&v| v <= mean) as f32 / sorted.len() as f32
        };
        sections.push(Section {
            start,
            end,
            kind,
            level,
            look: choose_sublook(kind, level, calm),
            repeats: None,
        });
    }

    // Long sections' means sit near the song's median, so stretch the ranks
    // over the song's own quietest-to-loudest section.
    let lo = sections
        .iter()
        .map(|s| s.level)
        .fold(f32::INFINITY, f32::min);
    let hi = sections.iter().map(|s| s.level).fold(0.0, f32::max);
    if hi - lo > 1e-3 {
        for s in &mut sections {
            s.level = (s.level - lo) / (hi - lo);
            s.look = choose_sublook(s.kind, s.level, calm);
        }
    }

    // A section mostly covered by a repeat's later span reuses the look of
    // the earlier section mostly covered by its first span.
    for i in 0..sections.len() {
        let me = (sections[i].start, sections[i].end);
        let len = me.1 - me.0;
        let best = sidecar
            .repeats
            .iter()
            .flatten()
            .filter(|r| overlap(me, (r.b_start as f32, r.b_end as f32)) >= 0.5 * len)
            .max_by(|a, b| a.similarity.total_cmp(&b.similarity));
        let Some(r) = best else { continue };
        let a = (r.a_start as f32, r.a_end as f32);
        let earlier = (0..i)
            .map(|j| (j, overlap((sections[j].start, sections[j].end), a)))
            .filter(|&(j, o)| o >= 0.5 * (sections[j].end - sections[j].start))
            .max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((j, _)) = earlier {
            let source = sections[j].repeats.unwrap_or(j);
            sections[i].look = sections[source].look;
            sections[i].repeats = Some(source);
        }
    }
    sections
}

/// The continuous half of a sub-look, which eases between sections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend {
    pub hue: f32,
    pub tint: [f32; 3],
    pub decay: f32,
    /// Beam brightness weight: 0 off, 1 on.
    pub beam: f32,
    /// Waveform jitter weight: 0 clean Lissajous, 1 oscilloscope.
    pub jitter: f32,
    /// Kaleidoscope fold weight.
    pub fold: f32,
}

impl Blend {
    fn of(look: &SubLook) -> Self {
        let p = PALETTES[look.palette];
        Self {
            hue: p.hue,
            tint: p.tint,
            decay: look.decay,
            beam: if look.beam == BeamStyle::Off {
                0.0
            } else {
                1.0
            },
            jitter: if look.beam == BeamStyle::Oscilloscope {
                1.0
            } else {
                0.0
            },
            fold: if look.fold { 1.0 } else { 0.0 },
        }
    }

    fn lerp(a: &Self, b: &Self, k: f32) -> Self {
        let l = |x: f32, y: f32| x + (y - x) * k;
        // Hue goes the short way round.
        let mut dh = b.hue - a.hue;
        if dh > 0.5 {
            dh -= 1.0;
        } else if dh < -0.5 {
            dh += 1.0;
        }
        Self {
            hue: (a.hue + dh * k).rem_euclid(1.0),
            tint: [
                l(a.tint[0], b.tint[0]),
                l(a.tint[1], b.tint[1]),
                l(a.tint[2], b.tint[2]),
            ],
            decay: l(a.decay, b.decay),
            beam: l(a.beam, b.beam),
            jitter: l(a.jitter, b.jitter),
            fold: l(a.fold, b.fold),
        }
    }
}

/// Follows the song's sections in time and eases the look between them.
#[derive(Clone, Debug)]
pub struct SectionTrack {
    pub sections: Vec<Section>,
    current: Option<usize>,
    from: Blend,
    to: Blend,
    since: f32,
}

impl SectionTrack {
    pub fn new(sections: Vec<Section>) -> Self {
        let first = sections.first().map_or(
            Blend::of(&choose_sublook(SidecarSectionKind::Other, 0.7, 0.0)),
            |s| Blend::of(&s.look),
        );
        Self {
            sections,
            current: None,
            from: first,
            to: first,
            since: SECTION_EASE_SECS,
        }
    }

    fn blend(&self) -> Blend {
        let x = (self.since / SECTION_EASE_SECS).clamp(0.0, 1.0);
        Blend::lerp(&self.from, &self.to, x * x * (3.0 - 2.0 * x))
    }

    /// Advance to time `t` (seconds into the track); returns this frame's
    /// blended look and the section's [`CURATED_C`] entry.
    pub fn update(&mut self, t: f32, dt: f32) -> (Blend, usize) {
        let idx = self
            .sections
            .iter()
            .rposition(|s| s.start <= t)
            .unwrap_or(0);
        self.since += dt;
        if self.current != Some(idx) {
            if let Some(section) = self.sections.get(idx) {
                let target = Blend::of(&section.look);
                if self.current.is_none() {
                    self.from = target;
                } else {
                    self.from = self.blend();
                }
                self.to = target;
                self.since = 0.0;
            }
            self.current = Some(idx);
        }
        let entry = self.sections.get(idx).map_or(0, |s| s.look.entry);
        (self.blend(), entry % CURATED_C.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyst_core::sidecar::{SidecarRepeat, SidecarSection};

    fn sidecar(
        duration: f64,
        sections: &[(f64, f64, SidecarSectionKind)],
        energy: &[(f64, f32)],
    ) -> Sidecar {
        let rate = 20.0f32;
        let n = (duration * rate as f64) as usize;
        let envelope = (0..n)
            .map(|i| {
                let t = i as f64 / rate as f64;
                energy
                    .iter()
                    .rev()
                    .find(|(s, _)| *s <= t)
                    .map_or(0.5, |&(_, e)| e)
            })
            .collect();
        let mut json: Sidecar = Sidecar::from_json_str(
            r#"{"schema":3,"duration":1,"tempo":120,"beats":[],"sections":[],"events":[],
            "onsets":[],"energyEnvelope":[],"bandEnvelope":{"sub":[],"low":[],"mid":[],
            "presence":[],"air":[]},"centroidEnvelope":[],"flatnessEnvelope":[],"envelopeRate":20}"#,
        )
        .unwrap();
        json.duration = duration;
        json.energy_envelope = envelope;
        json.sections = sections
            .iter()
            .map(|&(start, end, kind)| SidecarSection {
                start,
                end,
                kind,
                label: None,
                boundary_confidence: None,
            })
            .collect();
        json
    }

    fn character(tempo: f32, onsets: f32, mean_db: f32, range_db: f32) -> SongCharacter {
        SongCharacter::from_features(&TrackFeatures {
            tempo_bpm: tempo,
            onset_density: onsets,
            mean_db,
            dynamic_range_db: range_db,
            loud: 0.8,
        })
    }

    /// Bites: a click every half second in a flat flux curve counts as two
    /// onsets per second; a flat curve has none.
    #[test]
    fn onset_rate_counts_peaks() {
        let hps = 86.0;
        let mut flux = vec![1.0f32; 86 * 20];
        for i in (20..flux.len()).step_by(43) {
            flux[i] = 10.0;
        }
        let rate = onset_rate(&flux, hps);
        assert!((rate - 2.0).abs() < 0.1, "{rate}");
        assert_eq!(onset_rate(&[1.0; 500], hps), 0.0);
    }

    /// Bites: the levels come from the raw-level percentiles and the loud
    /// level from the extractor energy's 90th percentile.
    #[test]
    fn features_measure_levels() {
        let db: Vec<f32> = (0..=100).map(|i| -40.0 + 0.3 * i as f32).collect();
        let energy: Vec<f32> = (0..=100).map(|i| i as f32 / 100.0).collect();
        let f = TrackFeatures::measure(&db, &energy, 3.0, 120.0);
        assert!((f.mean_db + 25.0).abs() < 1e-4, "{}", f.mean_db);
        assert!((f.dynamic_range_db - 24.0).abs() < 1e-4);
        assert_eq!(f.loud, 0.9);
    }

    /// Bites: the owner's three tracks, with the features measured on them,
    /// order as he heard them: panicspiral keeps today's punch, the two
    /// slower-feeling ones are calmer.
    #[test]
    fn owner_tracks_calm_ordering() {
        let afterimage = character(119.7, 5.15, -20.0, 18.3);
        let event_horizon = character(119.7, 3.76, -23.7, 16.0);
        let panicspiral = character(119.7, 6.58, -17.1, 15.3);
        assert!(panicspiral.calm < 0.05, "{}", panicspiral.calm);
        assert!(afterimage.calm > 0.4, "{}", afterimage.calm);
        assert!(event_horizon.calm > 0.4, "{}", event_horizon.calm);
    }

    /// Bites: a slow, sparse, quiet track is calmer than a fast, dense,
    /// loud, squashed one, and the energetic one keeps calm 0 exactly, so
    /// its reactions are today's.
    #[test]
    fn calmness_orders_tracks_and_keeps_energetic_punch() {
        let chill = character(85.0, 2.0, -26.0, 20.0);
        let busy = character(150.0, 7.0, -12.0, 12.0);
        assert!(chill.calm > 0.9, "chill calm {}", chill.calm);
        assert_eq!(busy.calm, 0.0);
        assert_eq!(busy.react(), 1.0);
        assert_eq!(busy.slow(), 1.0);
        assert!(chill.react() < 0.3);
        assert!(chill.slow() > 4.0);
        // Relative to the song's own loud level.
        assert_eq!(busy.relative(0.4), 0.5);
        assert_eq!(busy.relative(2.0), 1.0);
    }

    /// Bites: with energy flipping every frame, a calm song's reactions move
    /// far less per frame than an energetic song's, and calm 0 passes the
    /// raw relative loudness through.
    #[test]
    fn calm_song_reacts_smaller_and_slower() {
        let dt = 1.0 / 30.0;
        let run = |calm: f32| -> (f32, f32) {
            let mut r = Reactor::new(SongCharacter { calm, loud: 0.8 });
            let mut prev: Option<Reactions> = None;
            let (mut beam_step, mut punch_step) = (0.0f32, 0.0f32);
            for i in 0..300 {
                let bus = hyst_core::SignalBus {
                    energy: if i % 2 == 0 { 0.8 } else { 0.1 },
                    ..Default::default()
                };
                let out = r.update(dt, &bus);
                if calm == 0.0 {
                    assert_eq!(out.beam_energy, bus.energy / 0.8);
                }
                if let Some(p) = prev {
                    beam_step = beam_step.max((out.beam_energy - p.beam_energy).abs());
                    punch_step = punch_step.max((out.punch_energy - p.punch_energy).abs());
                }
                prev = Some(out);
            }
            (beam_step, punch_step)
        };
        let (beam_busy, punch_busy) = run(0.0);
        let (beam_calm, punch_calm) = run(1.0);
        assert!(beam_busy > 0.8, "{beam_busy}");
        assert!(beam_calm < 0.1 * beam_busy, "{beam_calm} vs {beam_busy}");
        assert!(
            punch_calm < 0.02 * punch_busy,
            "{punch_calm} vs {punch_busy}"
        );
    }

    /// Bites: the sub-look is a pure function of character; loud
    /// non-break sections in an energetic song earn the fold and the
    /// oscilloscope, quiet ones and calm songs do not.
    #[test]
    fn sublook_follows_character() {
        use SidecarSectionKind::*;
        assert_eq!(
            choose_sublook(Other, 0.95, 0.0),
            choose_sublook(Other, 0.95, 0.0)
        );
        let loud = choose_sublook(Drop, 0.9, 0.0);
        assert!(loud.fold);
        assert_eq!(loud.beam, BeamStyle::Oscilloscope);
        let loud_calm_song = choose_sublook(Drop, 0.9, 0.8);
        assert!(!loud_calm_song.fold);
        assert_eq!(loud_calm_song.beam, BeamStyle::Lissajous);
        let quiet_break = choose_sublook(Break, 0.3, 0.0);
        assert!(!quiet_break.fold);
        assert_eq!(quiet_break.beam, BeamStyle::Off);
        assert!(quiet_break.decay > loud.decay);
        assert_ne!(quiet_break.entry, loud.entry);
    }

    /// Bites: boundaries come from sidecar sections and repeats, short
    /// pieces merge, gaps become `Other`, and a repeated span reuses the
    /// earlier section's look even when its own level would choose another.
    #[test]
    fn sections_split_merge_and_reuse_repeats() {
        use SidecarSectionKind::*;
        let mut sc = sidecar(
            120.0,
            &[(0.0, 3.0, Other), (3.0, 40.0, Break)],
            &[(0.0, 0.3), (40.0, 0.9), (80.0, 0.5)],
        );
        sc.repeats = Some(vec![SidecarRepeat {
            a_start: 40.0,
            a_end: 80.0,
            b_start: 80.0,
            b_end: 120.0,
            similarity: 0.5,
        }]);
        let s = sections_from_sidecar(&sc, 0.0);
        let edges: Vec<(f32, f32)> = s.iter().map(|x| (x.start, x.end)).collect();
        assert_eq!(edges, vec![(0.0, 40.0), (40.0, 80.0), (80.0, 120.0)]);
        assert_eq!(s[0].kind, Break);
        assert_eq!(s[1].kind, Other);
        assert_eq!(s[2].repeats, Some(1));
        assert_eq!(s[2].look, s[1].look);
        assert_ne!(
            choose_sublook(s[2].kind, s[2].level, 0.0),
            s[1].look,
            "the test needs the repeat to override a different look"
        );
        // Deterministic.
        assert_eq!(sections_from_sidecar(&sc, 0.0), s);
    }

    /// Bites: novelty peaks become boundaries.
    #[test]
    fn novelty_peaks_split_sections() {
        let mut sc = sidecar(100.0, &[], &[]);
        let novelty: Vec<f32> = (0..2000)
            .map(|i| 0.05 + 0.6 * (-((i as f32 - 1000.0) / 40.0).powi(2)).exp())
            .collect();
        sc.novelty_section_envelope = Some(novelty);
        let s = sections_from_sidecar(&sc, 0.0);
        let edges: Vec<(f32, f32)> = s.iter().map(|x| (x.start, x.end)).collect();
        assert_eq!(edges, vec![(0.0, 50.0), (50.0, 100.0)]);
    }

    /// Bites: across section changes every blended value moves continuously
    /// (no frame steps more than an eased change allows), each change takes
    /// the full ease, and the end state is the new section's look.
    #[test]
    fn section_changes_ease_without_snaps() {
        use SidecarSectionKind::*;
        let sc = sidecar(
            90.0,
            &[(0.0, 30.0, Break), (60.0, 90.0, Drop)],
            &[(0.0, 0.2), (30.0, 0.6), (60.0, 0.95)],
        );
        let sections = sections_from_sidecar(&sc, 0.0);
        assert_eq!(sections.len(), 3);
        let looks: Vec<SubLook> = sections.iter().map(|s| s.look).collect();
        assert_ne!(looks[0], looks[2]);
        let mut track = SectionTrack::new(sections);
        let dt = 1.0 / 30.0;
        // Smoothstep's peak slope is 1.5 per unit of progress.
        let max_step = 1.5 * dt / SECTION_EASE_SECS * 1.01;
        let mut prev: Option<Blend> = None;
        let mut entries = Vec::new();
        for i in 0..(90 * 30) {
            let t = i as f32 * dt;
            let (b, entry) = track.update(t, dt);
            entries.push(entry);
            if let Some(p) = prev {
                let mut dh = (b.hue - p.hue).abs();
                dh = dh.min(1.0 - dh);
                assert!(dh <= max_step * 0.5 + 1e-6, "hue step {dh} at {t}");
                for (x, y, range) in [
                    (b.decay, p.decay, 0.1),
                    (b.beam, p.beam, 1.0),
                    (b.jitter, p.jitter, 1.0),
                    (b.fold, p.fold, 1.0),
                    (b.tint[0], p.tint[0], 1.0),
                    (b.tint[2], p.tint[2], 1.0),
                ] {
                    assert!(
                        (x - y).abs() <= max_step * range + 1e-6,
                        "step {} at {t}",
                        (x - y).abs()
                    );
                }
            }
            prev = Some(b);
            if (t - 29.0).abs() < dt / 2.0 {
                assert_eq!(b, Blend::of(&looks[0]));
            }
            if (t - 32.0).abs() < dt / 2.0 {
                assert_ne!(b, Blend::of(&looks[1]), "still easing 2 s in");
            }
        }
        assert_eq!(prev.unwrap(), Blend::of(&looks[2]));
        assert_eq!(entries[0], looks[0].entry);
        assert_eq!(*entries.last().unwrap(), looks[2].entry);
    }
}
