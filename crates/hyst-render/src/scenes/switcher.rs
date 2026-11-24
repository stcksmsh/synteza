//! Which scene plays when: the song's sections, a scene per section chosen
//! from the section's character alone, and a cross-dissolve centred on each
//! boundary.
//!
//! Section derivation is copied from the parked song-aware Julia branch
//! (`passes/julia/song.rs` on `claude/song-aware-julia`): boundaries are the
//! sidecar's section edges, its repeat spans and its section-novelty peaks,
//! merged so no section is shorter than [`MIN_SECTION_SECS`]. A section the
//! repeat map says repeats an earlier one gets that one's scene back.
//!
//! The weights are a pure function of time, so a render is deterministic
//! and the dissolve can start before the boundary it is centred on.

use hyst_core::sidecar::{Sidecar, SidecarSectionKind};

use super::SceneKind;

/// Seconds a dissolve between two scenes takes, centred on the boundary.
pub const BLEND_SECS: f32 = 4.0;
/// Shortest section kept; boundaries closer than this are merged. Longer
/// than [`BLEND_SECS`], so two dissolves never overlap.
pub const MIN_SECTION_SECS: f32 = 8.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub start: f32,
    pub end: f32,
    pub kind: SidecarSectionKind,
    /// Where the section's mean energy ranks in the song's own energy,
    /// stretched so the quietest section is 0 and the loudest 1.
    pub level: f32,
    pub scene: SceneKind,
    /// The earlier section this one repeats, whose scene it reuses.
    pub repeats: Option<usize>,
}

/// Quiet sections grow, middling ones get the grid, loud ones the
/// feedback synth; a break never gets the feedback synth.
pub fn choose_scene(kind: SidecarSectionKind, level: f32) -> SceneKind {
    match kind {
        SidecarSectionKind::Break => {
            if level < 0.5 {
                SceneKind::Growth
            } else {
                SceneKind::Grid
            }
        }
        SidecarSectionKind::Drop => {
            if level < 0.5 {
                SceneKind::Grid
            } else {
                SceneKind::Feedback
            }
        }
        SidecarSectionKind::Build | SidecarSectionKind::Other => {
            if level < 0.35 {
                SceneKind::Growth
            } else if level < 0.7 {
                SceneKind::Grid
            } else {
                SceneKind::Feedback
            }
        }
    }
}

/// Peaks of a novelty envelope above median + 2 MAD that are the largest
/// value within `spacing` samples either side.
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

/// Sections for the whole track, each with its scene.
pub fn sections_from_sidecar(sidecar: &Sidecar) -> Vec<Section> {
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
            scene: choose_scene(kind, level),
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
            s.scene = choose_scene(s.kind, s.level);
        }
    }

    // A section mostly covered by a repeat's later span reuses the scene of
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
            sections[i].scene = sections[source].scene;
            sections[i].repeats = Some(source);
        }
    }
    sections
}

fn one_hot(scene: SceneKind) -> [f32; 3] {
    let mut w = [0.0; 3];
    w[scene as usize] = 1.0;
    w
}

fn smoothstep01(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Scene weights at time `t`, indexed by [`SceneKind`]. They sum to 1; at a
/// boundary the outgoing and incoming scenes trade weight over
/// [`BLEND_SECS`] centred on it.
pub fn weights_at(sections: &[Section], t: f32) -> [f32; 3] {
    let Some(i) = sections.iter().rposition(|s| s.start <= t).or(
        // Before the first section's start: the first section.
        (!sections.is_empty()).then_some(0),
    ) else {
        return one_hot(SceneKind::Grid);
    };
    let cur = &sections[i];
    let half = 0.5 * BLEND_SECS;
    let (from, to, k) = if i > 0 && t < cur.start + half {
        (
            sections[i - 1].scene,
            cur.scene,
            smoothstep01((t - (cur.start - half)) / BLEND_SECS),
        )
    } else if i + 1 < sections.len() && t > cur.end - half {
        (
            cur.scene,
            sections[i + 1].scene,
            smoothstep01((t - (cur.end - half)) / BLEND_SECS),
        )
    } else {
        (cur.scene, cur.scene, 0.0)
    };
    let (a, b) = (one_hot(from), one_hot(to));
    std::array::from_fn(|j| a[j] * (1.0 - k) + b[j] * k)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 120 s song: quiet intro, a loud middle, a break, then the intro's
    /// material again (the repeat map says 90..120 repeats 0..30).
    fn sidecar() -> Sidecar {
        let rate = 20.0;
        let energy: Vec<f32> = (0..2400)
            .map(|i| {
                let t = i as f32 / rate;
                if t < 30.0 {
                    0.2
                } else if t < 60.0 {
                    0.9
                } else if t < 90.0 {
                    0.5
                } else {
                    0.2
                }
            })
            .collect();
        let json = serde_json::json!({
            "schema": 4,
            "duration": 120.0,
            "tempo": 120.0,
            "beats": [],
            "sections": [
                {"start": 0.0, "end": 30.0, "kind": "other"},
                {"start": 30.0, "end": 60.0, "kind": "drop"},
                {"start": 60.0, "end": 90.0, "kind": "break"},
                {"start": 90.0, "end": 120.0, "kind": "other"}
            ],
            "events": [],
            "onsets": [],
            "energyEnvelope": energy,
            "bandEnvelope": {"sub": [0.1], "low": [0.1], "mid": [0.1], "presence": [0.1], "air": [0.1]},
            "centroidEnvelope": [0.5],
            "flatnessEnvelope": [0.3],
            "envelopeRate": rate,
            "repeats": [{"aStart": 0.0, "aEnd": 30.0, "bStart": 90.0, "bEnd": 120.0, "similarity": 0.8}]
        });
        Sidecar::from_json_str(&json.to_string()).unwrap()
    }

    #[test]
    fn scenes_follow_section_character_and_repeats() {
        let a = sections_from_sidecar(&sidecar());
        let b = sections_from_sidecar(&sidecar());
        assert_eq!(a, b, "the same sidecar must give the same sections");
        let scenes: Vec<SceneKind> = a.iter().map(|s| s.scene).collect();
        assert_eq!(
            scenes,
            [
                SceneKind::Growth,
                SceneKind::Feedback,
                SceneKind::Grid,
                SceneKind::Growth
            ]
        );
        assert_eq!(a[3].repeats, Some(0));
    }

    /// Weights always sum to 1, and at 30 fps no frame moves any weight by
    /// more than the dissolve's steepest slope allows.
    #[test]
    fn weights_sum_to_one_without_snaps() {
        let sections = sections_from_sidecar(&sidecar());
        let dt = 1.0 / 30.0;
        let max_step = 1.5 / BLEND_SECS * dt + 1e-4;
        let mut prev = weights_at(&sections, -1.0);
        let mut t = -1.0 + dt;
        let mut saw_blend = false;
        while t < 125.0 {
            let w = weights_at(&sections, t);
            let sum: f32 = w.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5, "weights sum to {sum} at t={t}");
            for j in 0..3 {
                assert!(
                    (w[j] - prev[j]).abs() <= max_step,
                    "weight {j} snapped at t={t}: {} -> {}",
                    prev[j],
                    w[j]
                );
            }
            saw_blend |= w.iter().filter(|&&x| x > 0.1).count() == 2;
            prev = w;
            t += dt;
        }
        assert!(
            saw_blend,
            "a section change must dissolve through both scenes"
        );
    }
}
