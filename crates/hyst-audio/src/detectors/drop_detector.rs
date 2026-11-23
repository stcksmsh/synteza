//! Ported from `src/audio/worklet/brain/drop-detector.ts`.
//!
//! A drop is the mix going from sparse to full, not the mix getting louder.
//! It arms on fullness (sustained, non-spiky low energy) plus a novelty
//! contrast against the recent past, optionally with an onset-density jump
//! (the rhythmic path; the soft path lets a pad swelling in qualify too).
//! Confirmation over a short window, refractory, startup grace and a
//! beat-snap lookahead then decide when it fires. See the TS file's header
//! for the reasoning behind each threshold.

use super::novelty::NoveltyRingBuffer;
use crate::dsp::activity::{FullnessTracker, OnsetDensityTracker};

pub const DROP_VEC_LEN: usize = 7;

const FULLNESS_THRESHOLD: f32 = 0.5;
const ONSET_JUMP_MIN: f32 = 0.06;

const NOVELTY_WINDOW_SEC: f32 = 4.0;
/// 1 - max cosine similarity against the recent buffer.
const NOVELTY_THRESHOLD: f32 = 0.3;
const NOVELTY_HOLD_SEC: f32 = 3.0;

const CONFIRM_WINDOW_SEC: f32 = 0.3;
const CONFIRM_ELEVATED_FRACTION: f32 = 0.5;
const ELEVATED_MARGIN: f32 = 0.08;

const BEAT_SNAP_LOOKAHEAD_SEC: f32 = 0.15;
const REFRACTORY_SEC: f32 = 8.0;
const STARTUP_GRACE_SEC: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropEvent {
    pub strength: f32,
    pub t: f32,
}

/// Live values of the three qualifying signals, for diagnosing why a drop
/// does or does not fire on real audio.
#[derive(Debug, Clone, Copy, Default)]
pub struct DropDetectorDebug {
    pub fullness: f32,
    pub onset_jump: f32,
    pub novelty_peak: f32,
    pub armed: bool,
}

/// `vec` is the novelty-contrast feature vector (bands, centroid, flatness);
/// `onset_activity` is the continuous per-hop flux strength, not a
/// thresholded flag.
#[derive(Debug, Clone, Copy)]
pub struct DropDetectorFeatures {
    pub low_energy: f32,
    pub onset_activity: f32,
    pub vec: [f32; DROP_VEC_LEN],
}

pub struct DropDetector {
    fullness_tracker: FullnessTracker,
    onset_density_tracker: OnsetDensityTracker,
    novelty: NoveltyRingBuffer<DROP_VEC_LEN>,
    novelty_peak: f32,

    refractory_until: f32,
    armed_at: Option<f32>,
    armed_baseline: f32,
    armed_peak_fullness: f32,
    elevated_hops: u32,
    window_hops: u32,
    hop_sec: f32,
    started_at: Option<f32>,
    last_debug: DropDetectorDebug,
}

impl DropDetector {
    pub fn new(hop_ms: f32) -> Self {
        let capacity = ((NOVELTY_WINDOW_SEC * 1000.0) / hop_ms).round().max(1.0) as usize;
        Self {
            fullness_tracker: FullnessTracker::new(hop_ms),
            onset_density_tracker: OnsetDensityTracker::new(hop_ms),
            novelty: NoveltyRingBuffer::new(capacity),
            novelty_peak: 0.0,
            refractory_until: 0.0,
            armed_at: None,
            armed_baseline: 0.0,
            armed_peak_fullness: 0.0,
            elevated_hops: 0,
            window_hops: 0,
            hop_sec: hop_ms / 1000.0,
            started_at: None,
            last_debug: DropDetectorDebug::default(),
        }
    }

    pub fn update(
        &mut self,
        features: &DropDetectorFeatures,
        beat_phase: f32,
        tempo_bpm: f32,
        t_now: f32,
    ) -> Option<DropEvent> {
        let started_at = *self.started_at.get_or_insert(t_now);

        let fullness_result = self.fullness_tracker.update(features.low_energy);
        let fullness = fullness_result.fullness;
        let slow = fullness_result.slow;
        let onset_jump = self
            .onset_density_tracker
            .update(features.onset_activity)
            .jump;

        let novelty_value = 1.0 - self.novelty.max_similarity(&features.vec);
        self.novelty.push(features.vec);
        self.novelty_peak =
            novelty_value.max(self.novelty_peak * (-self.hop_sec / NOVELTY_HOLD_SEC).exp());

        self.last_debug = DropDetectorDebug {
            fullness,
            onset_jump,
            novelty_peak: self.novelty_peak,
            armed: self.armed_at.is_some(),
        };

        if t_now - started_at < STARTUP_GRACE_SEC || t_now < self.refractory_until {
            self.disarm();
            return None;
        }

        let Some(armed_at) = self.armed_at else {
            let rhythmic_drop = fullness > FULLNESS_THRESHOLD
                && onset_jump > ONSET_JUMP_MIN
                && self.novelty_peak > NOVELTY_THRESHOLD;
            let soft_drop = fullness > FULLNESS_THRESHOLD && self.novelty_peak > NOVELTY_THRESHOLD;
            if rhythmic_drop || soft_drop {
                self.armed_at = Some(t_now);
                self.armed_baseline = slow;
                self.armed_peak_fullness = fullness;
                self.elevated_hops = 0;
                self.window_hops = 0;
            }
            return None;
        };

        self.armed_peak_fullness = self.armed_peak_fullness.max(fullness);
        self.window_hops += 1;
        if features.low_energy > self.armed_baseline + ELEVATED_MARGIN {
            self.elevated_hops += 1;
        }

        if t_now - armed_at < CONFIRM_WINDOW_SEC {
            return None;
        }

        // Occupancy check: a drop keeps energy up across the window; a lone
        // percussive hit only spikes briefly.
        if (self.elevated_hops as f32 / self.window_hops.max(1) as f32) < CONFIRM_ELEVATED_FRACTION
        {
            self.disarm();
            return None;
        }

        // Confirmed. Hold briefly only if a beat boundary is imminent.
        if tempo_bpm > 0.0 {
            let beat_period_sec = 60.0 / tempo_bpm;
            let sec_to_next_beat = (1.0 - beat_phase) * beat_period_sec;
            if sec_to_next_beat > self.hop_sec && sec_to_next_beat <= BEAT_SNAP_LOOKAHEAD_SEC {
                return None;
            }
        }

        let strength = self.armed_peak_fullness.clamp(0.0, 1.0);
        self.disarm();
        self.refractory_until = t_now + REFRACTORY_SEC;
        Some(DropEvent { strength, t: t_now })
    }

    fn disarm(&mut self) {
        self.armed_at = None;
        self.armed_peak_fullness = 0.0;
    }

    pub fn debug(&self) -> DropDetectorDebug {
        self.last_debug
    }
}

#[cfg(test)]
mod tests {
    //! Mirrors `tests/unit/drop-detector.spec.ts` case for case.
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;
    const HOP_SIZE: f32 = 512.0;
    const HOP_MS: f32 = (HOP_SIZE / SAMPLE_RATE) * 1000.0;
    const HOP_SEC: f32 = HOP_MS / 1000.0;
    const TEMPO: f32 = 120.0;

    const QUIET_VEC: [f32; DROP_VEC_LEN] = [0.05, 0.05, 0.05, 0.05, 0.05, 0.3, 0.5];
    const LOUD_VEC: [f32; DROP_VEC_LEN] = [0.9, 0.9, 0.5, 0.4, 0.3, 0.5, 0.3];

    fn f(low_energy: f32, onset_activity: f32, vec: [f32; DROP_VEC_LEN]) -> DropDetectorFeatures {
        DropDetectorFeatures {
            low_energy,
            onset_activity,
            vec,
        }
    }

    // t is computed from the hop index rather than accumulated, so f32 does
    // not drift where the TS original accumulates in f64.
    fn run(steps: impl Fn(f32) -> DropDetectorFeatures, duration_sec: f32) -> Vec<f32> {
        let mut detector = DropDetector::new(HOP_MS);
        let beat_period_sec = 60.0 / TEMPO;
        let mut drops = Vec::new();
        let mut i = 0u32;
        loop {
            let t = i as f32 * HOP_SEC;
            if t >= duration_sec {
                break;
            }
            let beat_phase = (t % beat_period_sec) / beat_period_sec;
            if let Some(event) = detector.update(&steps(t), beat_phase, TEMPO, t) {
                drops.push(event.t);
            }
            i += 1;
        }
        drops
    }

    fn quiet() -> DropDetectorFeatures {
        f(0.05, 0.02, QUIET_VEC)
    }

    fn groove(since: f32, t: f32) -> DropDetectorFeatures {
        let on_beat = (t - since) % 0.25 < HOP_SEC;
        f(0.9, if on_beat { 0.8 } else { 0.1 }, LOUD_VEC)
    }

    #[test]
    fn emits_nothing_for_a_steady_groove_with_no_thinned_section() {
        let drops = run(
            |t| {
                let kick = t % 0.5 < 0.1;
                if kick {
                    f(0.9, 0.8, LOUD_VEC)
                } else {
                    f(0.45, 0.05, QUIET_VEC)
                }
            },
            30.0,
        );
        assert!(drops.is_empty(), "{drops:?}");
    }

    #[test]
    fn emits_exactly_one_drop_for_quiet_section_then_sustained_jump() {
        let drops = run(|t| if t < 4.0 { quiet() } else { groove(4.0, t) }, 20.0);
        assert_eq!(drops.len(), 1, "{drops:?}");
        assert!(drops[0] > 4.0);
        assert!(drops[0] < 9.0);
    }

    #[test]
    fn ignores_a_lone_transient_spike_that_decays_immediately() {
        let drops = run(
            |t| {
                if (4.0..4.1).contains(&t) {
                    f(0.9, 0.8, LOUD_VEC)
                } else {
                    quiet()
                }
            },
            20.0,
        );
        assert!(drops.is_empty(), "{drops:?}");
    }

    #[test]
    fn does_not_emit_a_second_drop_without_another_thinned_section() {
        let drops = run(|t| if t < 4.0 { quiet() } else { groove(4.0, t) }, 40.0);
        assert_eq!(drops.len(), 1, "{drops:?}");
    }

    #[test]
    fn emits_a_soft_ambient_drop_with_no_onset_density_jump_at_all() {
        let drops = run(
            |t| {
                if t < 4.0 {
                    quiet()
                } else {
                    f(0.9, 0.02, LOUD_VEC)
                }
            },
            20.0,
        );
        assert_eq!(drops.len(), 1, "{drops:?}");
        assert!(drops[0] > 4.0);
    }

    #[test]
    fn emits_again_after_a_second_genuine_thinned_section() {
        let drops = run(
            |t| {
                if t < 4.0 {
                    quiet()
                } else if t < 20.0 {
                    groove(4.0, t)
                } else if t < 26.0 {
                    quiet()
                } else {
                    groove(26.0, t)
                }
            },
            40.0,
        );
        assert_eq!(drops.len(), 2, "{drops:?}");
        assert!(drops[1] > 26.0);
    }
}
