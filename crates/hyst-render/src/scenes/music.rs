//! The music as the scenes read it: beat and bar clocks, envelopes and
//! onsets from the offline sidecar, each read on the song's own scale.
//!
//! The live beat tracker reads about 120 BPM on every track, so everything
//! here comes from the sidecar. The beat and bar counts are continuous: the
//! phase interpolates between consecutive grid times, so it never snaps and
//! its rate only changes at a grid point. Loudness is normalised between the
//! song's own 10th and 90th percentiles, so a quiet song still spans 0..1.

use hyst_core::sidecar::Sidecar;

/// One onset that fell inside the last frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Onset {
    /// Strength relative to the song's own 90th-percentile onset, 0..1.
    pub strength: f32,
    /// 0 = sub, 1 = air.
    pub tone: f32,
    /// -1..1.
    pub pan: f32,
    /// Seconds into the track.
    pub t: f32,
}

/// What every scene reads each frame.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneInputs {
    /// Seconds into the track.
    pub t: f32,
    pub dt: f32,
    /// Continuous beat count: `floor` is the beat index, `fract` the phase.
    pub beat: f32,
    /// Continuous bar count, from the sidecar's downbeats.
    pub bar: f32,
    /// Relative loudness, 0..1 on the song's own scale.
    pub energy: f32,
    /// `energy` smoothed over about four seconds.
    pub energy_slow: f32,
    pub sub: f32,
    pub air: f32,
    /// Onsets in `(t - dt, t]`.
    pub onsets: Vec<Onset>,
    /// Decaying onset pulse, 0..1.
    pub kick: f32,
}

pub struct SongTrack {
    beats: Vec<f32>,
    downbeats: Vec<f32>,
    rate: f32,
    energy: Vec<f32>,
    sub: Vec<f32>,
    air: Vec<f32>,
    onsets: Vec<Onset>,
    energy_slow: Option<f32>,
    kick: f32,
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

/// `values` rescaled so the song's own 10th percentile is 0 and its 90th is 1.
fn normalised(values: &[f32]) -> Vec<f32> {
    let lo = percentile(values, 0.1);
    let hi = percentile(values, 0.9).max(lo + 1e-4);
    values
        .iter()
        .map(|v| ((v - lo) / (hi - lo)).clamp(0.0, 1.0))
        .collect()
}

/// Continuous count along a sorted time grid: `i + frac` between grid
/// points `i` and `i + 1`, extrapolated at the median spacing before the
/// first and after the last point. Falls back to `fallback_period` when the
/// grid has fewer than two points.
pub fn grid_count(grid: &[f32], t: f32, fallback_period: f32) -> f32 {
    if grid.len() < 2 {
        let origin = grid.first().copied().unwrap_or(0.0);
        return (t - origin) / fallback_period;
    }
    let mut gaps: Vec<f32> = grid.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_by(f32::total_cmp);
    let period = gaps[gaps.len() / 2].max(1e-3);
    let i = grid.partition_point(|&g| g <= t);
    if i == 0 {
        (t - grid[0]) / period
    } else if i == grid.len() {
        (grid.len() - 1) as f32 + (t - grid[grid.len() - 1]) / period
    } else {
        let (a, b) = (grid[i - 1], grid[i]);
        (i - 1) as f32 + (t - a) / (b - a).max(1e-4)
    }
}

fn env_at(values: &[f32], rate: f32, t: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let x = (t * rate).max(0.0);
    let i = (x as usize).min(values.len() - 1);
    let j = (i + 1).min(values.len() - 1);
    let f = (x - i as f32).clamp(0.0, 1.0);
    values[i] + (values[j] - values[i]) * f
}

impl SongTrack {
    pub fn new(sidecar: &Sidecar) -> Self {
        let downbeats: Vec<f32> = sidecar
            .events
            .iter()
            .filter(|e| e.kind == hyst_core::sidecar::SidecarEventType::Downbeat)
            .map(|e| e.t as f32)
            .collect();
        let strengths: Vec<f32> = sidecar.onsets.iter().map(|o| o.strength).collect();
        let loud_onset = percentile(&strengths, 0.9).max(1e-4);
        Self {
            beats: sidecar.beats.iter().map(|&b| b as f32).collect(),
            downbeats,
            rate: sidecar.envelope_rate.max(1e-3),
            energy: normalised(&sidecar.energy_envelope),
            sub: normalised(&sidecar.band_envelope.sub),
            air: normalised(&sidecar.band_envelope.air),
            onsets: sidecar
                .onsets
                .iter()
                .map(|o| Onset {
                    strength: (o.strength / loud_onset).clamp(0.0, 1.0),
                    tone: o.tone.clamp(0.0, 1.0),
                    pan: o.pan.clamp(-1.0, 1.0),
                    t: o.t as f32,
                })
                .collect(),
            energy_slow: None,
            kick: 0.0,
        }
    }

    /// The inputs for the frame at `t`, advancing the smoothed state by `dt`.
    pub fn advance(&mut self, t: f32, dt: f32) -> SceneInputs {
        let energy = env_at(&self.energy, self.rate, t);
        let slow = self.energy_slow.get_or_insert(energy);
        *slow += (energy - *slow) * (dt / 4.0).min(1.0);
        let energy_slow = *slow;
        let lo = self.onsets.partition_point(|o| o.t <= t - dt);
        let hi = self.onsets.partition_point(|o| o.t <= t);
        let onsets = self.onsets[lo..hi].to_vec();
        self.kick *= (-dt / 0.25).exp();
        for o in &onsets {
            self.kick = self.kick.max(o.strength);
        }
        let beat = grid_count(&self.beats, t, 0.5);
        let bar = if self.downbeats.len() >= 2 {
            grid_count(&self.downbeats, t, 2.0)
        } else {
            beat / 4.0
        };
        SceneInputs {
            t,
            dt,
            beat,
            bar,
            energy,
            energy_slow,
            sub: env_at(&self.sub, self.rate, t),
            air: env_at(&self.air, self.rate, t),
            onsets,
            kick: self.kick,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The beat count is continuous and increasing across grid points, an
    /// irregular gap, and both extrapolated ends.
    #[test]
    fn grid_count_is_continuous_and_monotonic() {
        let grid = [1.0, 1.5, 2.0, 6.0, 6.5, 7.0];
        let mut prev = grid_count(&grid, -1.0, 0.5);
        let mut t = -1.0 + 1.0 / 30.0;
        while t < 10.0 {
            let c = grid_count(&grid, t, 0.5);
            assert!(c > prev, "count must increase at t={t}: {prev} -> {c}");
            // Fastest rate is one count per 0.5 s.
            assert!(c - prev < 0.07, "count jumped at t={t}: {prev} -> {c}");
            prev = c;
            t += 1.0 / 30.0;
        }
        assert!((grid_count(&grid, 2.0, 0.5) - 2.0).abs() < 1e-5);
    }
}
