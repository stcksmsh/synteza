//! Ported from `src/audio/worklet/brain/break-detector.ts`.
//!
//! A break is tension, not silence: sub collapsing plus onsets stopping.
//! Tension rises smoothly while both conditions persist and relaxes once
//! either returns, with hysteresis around the break start/end transition so
//! it does not chatter at the threshold.

use crate::dsp::envelope::EnvelopeFollower;

const LOW_ENERGY_MS: f32 = 2000.0;
const ONSET_ACTIVITY_MS: f32 = 1500.0;
const TENSION_ATTACK_MS: f32 = 500.0;
const TENSION_RELEASE_MS: f32 = 1200.0;
/// Novelty is normally small and sparse; amplify before treating it as activity.
const ONSET_ACTIVITY_SCALE: f32 = 3.0;
const BREAK_ENTER_THRESHOLD: f32 = 0.4;
/// Hysteresis band around ENTER, avoids chattering at the boundary.
const BREAK_EXIT_THRESHOLD: f32 = 0.25;

/// The TS `StructuralEvent` of type `breakStart`/`breakEnd`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BreakEvent {
    Start { strength: f32, t: f32 },
    End { strength: f32, t: f32 },
}

#[derive(Debug, Clone, Copy)]
pub struct BreakDetectorOutput {
    pub tension: f32,
    pub event: Option<BreakEvent>,
}

pub struct BreakDetector {
    low_energy_slow: EnvelopeFollower,
    onset_activity_slow: EnvelopeFollower,
    tension_env: EnvelopeFollower,
    in_break: bool,
}

impl BreakDetector {
    pub fn new(hop_ms: f32) -> Self {
        Self {
            low_energy_slow: EnvelopeFollower::new(LOW_ENERGY_MS, LOW_ENERGY_MS, hop_ms),
            onset_activity_slow: EnvelopeFollower::new(
                ONSET_ACTIVITY_MS,
                ONSET_ACTIVITY_MS,
                hop_ms,
            ),
            tension_env: EnvelopeFollower::new(TENSION_ATTACK_MS, TENSION_RELEASE_MS, hop_ms),
            in_break: false,
        }
    }

    pub fn update(&mut self, low_energy: f32, novelty: f32, t_now: f32) -> BreakDetectorOutput {
        let low = self.low_energy_slow.update(low_energy);
        let onset_activity = self.onset_activity_slow.update(novelty);
        let raw_tension = (1.0 - low) * (1.0 - (onset_activity * ONSET_ACTIVITY_SCALE).min(1.0));
        let tension = self.tension_env.update(raw_tension.max(0.0));

        let mut event = None;
        if !self.in_break && tension > BREAK_ENTER_THRESHOLD {
            self.in_break = true;
            event = Some(BreakEvent::Start {
                strength: tension,
                t: t_now,
            });
        } else if self.in_break && tension < BREAK_EXIT_THRESHOLD {
            self.in_break = false;
            event = Some(BreakEvent::End {
                strength: 1.0 - tension,
                t: t_now,
            });
        }

        BreakDetectorOutput { tension, event }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // break-detector.ts has no spec in tests/unit/; these cover the port's
    // basic contract on synthetic input.
    const HOP_SEC: f32 = 512.0 / 48_000.0;
    const HOP_MS: f32 = HOP_SEC * 1000.0;

    fn run(steps: impl Fn(f32) -> (f32, f32), duration_sec: f32) -> (Vec<BreakEvent>, f32) {
        let mut d = BreakDetector::new(HOP_MS);
        let mut events = Vec::new();
        let mut tension = 0.0;
        let hops = (duration_sec / HOP_SEC) as usize;
        for i in 0..hops {
            let t = i as f32 * HOP_SEC;
            let (low, novelty) = steps(t);
            let out = d.update(low, novelty, t);
            tension = out.tension;
            events.extend(out.event);
        }
        (events, tension)
    }

    #[test]
    fn a_full_busy_mix_never_reads_as_a_break() {
        let (events, tension) = run(|_| (0.9, 0.5), 20.0);
        assert!(events.is_empty());
        assert!(tension < 0.1);
    }

    #[test]
    fn sub_collapse_and_onsets_stopping_enter_then_exit_a_break_once() {
        let (events, _) = run(
            |t| {
                if (4.0..12.0).contains(&t) {
                    (0.02, 0.0)
                } else {
                    (0.9, 0.5)
                }
            },
            20.0,
        );
        assert_eq!(events.len(), 2, "{events:?}");
        match (events[0], events[1]) {
            (BreakEvent::Start { t: s, .. }, BreakEvent::End { t: e, .. }) => {
                assert!(s > 4.0 && s < 8.0, "start {s}");
                assert!(e > 12.0 && e < 16.0, "end {e}");
            }
            other => panic!("unexpected order {other:?}"),
        }
    }
}
