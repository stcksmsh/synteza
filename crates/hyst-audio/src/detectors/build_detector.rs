//! Ported from `src/audio/worklet/brain/build-detector.ts`.
//!
//! A build is a trend, not a level: sustained rising centroid ("filter
//! opening") plus rising sub, tracked as a fast-versus-slow moving-average
//! spread. `build_progress` integrates that rise over time rather than
//! reading the instantaneous rise rate, because direct feature reads twitch.

use crate::dsp::envelope::EnvelopeFollower;

const FAST_MS: f32 = 600.0;
const SLOW_MS: f32 = 6000.0;
/// How fast a sustained rise winds progress up.
const INTEGRATION_GAIN: f32 = 0.06;
/// Constant relaxation so progress does not stick at 1 forever.
const DECAY_PER_HOP: f32 = 0.0015;

pub struct BuildDetector {
    centroid_fast: EnvelopeFollower,
    centroid_slow: EnvelopeFollower,
    sub_fast: EnvelopeFollower,
    sub_slow: EnvelopeFollower,
    progress: f32,
}

impl BuildDetector {
    pub fn new(hop_ms: f32) -> Self {
        Self {
            centroid_fast: EnvelopeFollower::new(FAST_MS, FAST_MS, hop_ms),
            centroid_slow: EnvelopeFollower::new(SLOW_MS, SLOW_MS, hop_ms),
            sub_fast: EnvelopeFollower::new(FAST_MS, FAST_MS, hop_ms),
            sub_slow: EnvelopeFollower::new(SLOW_MS, SLOW_MS, hop_ms),
            progress: 0.0,
        }
    }

    pub fn update(&mut self, centroid: f32, sub: f32) -> f32 {
        let centroid_rise =
            (self.centroid_fast.update(centroid) - self.centroid_slow.update(centroid)).max(0.0);
        let sub_rise = (self.sub_fast.update(sub) - self.sub_slow.update(sub)).max(0.0);
        let rise = (centroid_rise + sub_rise) / 2.0;

        self.progress += rise * INTEGRATION_GAIN - DECAY_PER_HOP;
        self.progress = self.progress.clamp(0.0, 1.0);
        self.progress
    }

    /// Called when a drop fires, so the release is immediate rather than
    /// only a slow decay.
    pub fn reset(&mut self) {
        self.progress = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // build-detector.ts has no spec in tests/unit/; these cover the port's
    // basic contract on synthetic input.
    const HOP_MS: f32 = 512.0 / 48_000.0 * 1000.0;

    #[test]
    fn a_steady_signal_never_builds() {
        let mut d = BuildDetector::new(HOP_MS);
        let mut last = 1.0;
        for _ in 0..2000 {
            last = d.update(0.4, 0.5);
        }
        assert_eq!(last, 0.0);
    }

    #[test]
    fn a_sustained_rise_in_centroid_and_sub_winds_progress_up_and_reset_releases_it() {
        let mut d = BuildDetector::new(HOP_MS);
        let hops = (8000.0 / HOP_MS) as usize;
        let mut last = 0.0;
        for i in 0..hops {
            let x = i as f32 / hops as f32;
            last = d.update(0.1 + 0.8 * x, 0.1 + 0.8 * x);
        }
        assert!(last > 0.2, "progress {last}");
        d.reset();
        assert!(d.update(0.9, 0.9) < last);
    }
}
