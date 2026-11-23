//! The "after" look's Julia driver: a curated list of `c` values, a
//! bounded dive into each, and a slow or energy-triggered move between them.

use super::{
    find_interesting_pan, steer_toward_boundary, ColourEnv, JuliaNavState, C_ORBIT_CAP,
    C_ORBIT_REL, PAN_EASE, PAN_STEP_FRAC, RESTEER_FRAMES,
};

/// Hand-picked `c` values on the Mandelbrot boundary, whose Julia sets are
/// connected and keep detail at every depth: the dendrite (`i`), Douady's
/// rabbit, a seahorse-valley point and a Siegel-disk parameter.
pub const CURATED_C: [[f32; 2]; 4] = [[0.0, 1.0], [-0.123, 0.745], [-0.75, 0.1], [-0.391, -0.587]];

/// Zoom at the top of each dive: the whole set in frame, a little margin.
const CURATED_ZOOM_TOP: f32 = 1.4;
/// Deepest the dive goes. Well above `DIRECT_ZOOM_MIN`, so the renderer
/// stays on direct iteration, and deep enough that the frame is all
/// boundary filament rather than the set's outline.
const CURATED_ZOOM_DEEPEST: f32 = 2e-3;
/// Dive rate in log-zoom per second at zero and full smoothed energy.
const CURATED_DIVE_RATE: [f32; 2] = [0.08, 0.2];
/// Log-zoom per second the view pulls back while `c` moves to the next
/// entry, and the depth (log-zoom below the top) it pulls back to.
const CURATED_PULLBACK_RATE: f32 = 0.6;
const CURATED_PULLBACK_DEPTH: f32 = 0.4;
/// Seconds an entry is held before the cadence moves on, the shortest hold
/// an energy jump may cut.
const CURATED_HOLD_SECS: f32 = 24.0;
const CURATED_MIN_HOLD_SECS: f32 = 8.0;
/// A move between entries takes at least this long, and longer for far
/// entries, so `c` never travels faster than `CURATED_MOVE_SPEED` per
/// second at the middle of its eased path: the set reshapes, it does not
/// snap.
const CURATED_MOVE_MIN_SECS: f32 = 6.0;
const CURATED_MOVE_SPEED: f32 = 0.15;
/// Fast-minus-slow energy envelope that counts as a jump.
const CURATED_ENERGY_JUMP: f32 = 0.2;

/// Below this zoom the renderer switches from direct float32 iteration to
/// the perturbation path; above it direct iteration is exact enough and
/// avoids perturbation's speckle where `Z_n + dz_n` loses precision.
pub const DIRECT_ZOOM_MIN: f32 = 1e-4;

/// Driver for the "after" look: `c` sits on one [`CURATED_C`] entry at a
/// time and dives into it; on a slow cadence, or when smoothed energy jumps,
/// `c` eases to the next entry while the view pulls back out, then the dive
/// starts again. The dive is bounded between `CURATED_ZOOM_TOP` and
/// `CURATED_ZOOM_DEEPEST` so the clip stays where the detail is.
///
/// Like [`super::JuliaDriver`], every phase-like value advances by `dt * rate`;
/// audio only ever sets a rate, an amplitude or a trigger.
#[derive(Clone, Copy, Debug)]
pub struct CuratedJuliaDriver {
    entry: usize,
    /// `c` the current move started from.
    from_c: [f32; 2],
    /// Seconds into the current move, or `None` while holding an entry.
    moving: Option<f32>,
    move_secs: f32,
    hold_secs: f32,
    /// Log-zoom below `CURATED_ZOOM_TOP`.
    depth: f32,
    wobble: f32,
    pan: [f32; 2],
    pan_target: [f32; 2],
    frame: u32,
    energy_fast: f32,
    energy_env: f32,
    energy_slow: f32,
    colour: ColourEnv,
}

impl Default for CuratedJuliaDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl CuratedJuliaDriver {
    pub fn new() -> Self {
        let c = CURATED_C[0];
        let pan = find_interesting_pan(c);
        Self {
            entry: 0,
            from_c: c,
            moving: None,
            move_secs: CURATED_MOVE_MIN_SECS,
            hold_secs: 0.0,
            depth: 0.0,
            wobble: 0.0,
            pan,
            pan_target: pan,
            frame: 0,
            energy_fast: 0.0,
            energy_env: 0.0,
            energy_slow: 0.0,
            colour: ColourEnv::new(),
        }
    }

    pub fn update(&mut self, dt: f32, bus: &hyst_core::SignalBus) -> JuliaNavState {
        self.frame = self.frame.wrapping_add(1);
        let ema = |cur: f32, target: f32, tc: f32| cur + (target - cur) * (dt / tc).min(1.0);
        let energy = bus.energy.clamp(0.0, 1.0);
        self.energy_fast = ema(self.energy_fast, energy, 0.3);
        self.energy_env = ema(self.energy_env, energy, 1.5);
        self.energy_slow = ema(self.energy_slow, energy, 4.0);
        let e = self.energy_env;

        match self.moving {
            None => {
                self.hold_secs += dt;
                let jumped = self.energy_fast - self.energy_slow > CURATED_ENERGY_JUMP
                    && self.hold_secs >= CURATED_MIN_HOLD_SECS;
                if self.hold_secs >= CURATED_HOLD_SECS || jumped {
                    self.from_c = CURATED_C[self.entry];
                    self.entry = (self.entry + 1) % CURATED_C.len();
                    self.moving = Some(0.0);
                    let to = CURATED_C[self.entry];
                    let dist = ((to[0] - self.from_c[0]).powi(2)
                        + (to[1] - self.from_c[1]).powi(2))
                    .sqrt();
                    // Smoothstep's peak slope is 1.5x its mean.
                    self.move_secs = (1.5 * dist / CURATED_MOVE_SPEED).max(CURATED_MOVE_MIN_SECS);
                    self.pan_target = find_interesting_pan(CURATED_C[self.entry]);
                }
            }
            Some(s) => {
                let s = s + dt;
                if s >= self.move_secs {
                    self.moving = None;
                    self.hold_secs = 0.0;
                } else {
                    self.moving = Some(s);
                }
            }
        }

        let max_depth = (CURATED_ZOOM_TOP / CURATED_ZOOM_DEEPEST).ln();
        if self.moving.is_some() {
            self.depth = (self.depth - dt * CURATED_PULLBACK_RATE).max(CURATED_PULLBACK_DEPTH);
        } else {
            let rate = CURATED_DIVE_RATE[0] + (CURATED_DIVE_RATE[1] - CURATED_DIVE_RATE[0]) * e;
            self.depth = (self.depth + dt * rate).min(max_depth);
        }
        let zoom = CURATED_ZOOM_TOP * (-self.depth).exp();

        let target_c = CURATED_C[self.entry];
        let base_c = match self.moving {
            Some(s) => {
                let x = (s / self.move_secs).clamp(0.0, 1.0);
                let k = x * x * (3.0 - 2.0 * x);
                [
                    self.from_c[0] + (target_c[0] - self.from_c[0]) * k,
                    self.from_c[1] + (target_c[1] - self.from_c[1]) * k,
                ]
            }
            None => target_c,
        };
        // Small orbit around the entry so the shape keeps breathing while
        // held; sized to the view, as in `JuliaDriver`.
        self.wobble += dt * (0.1 + 0.3 * e);
        let r = (C_ORBIT_REL * zoom).min(C_ORBIT_CAP) * 0.3 * (0.5 + 0.5 * e);
        let c = [
            base_c[0] + r * self.wobble.cos(),
            base_c[1] + r * (0.75 * self.wobble).sin(),
        ];

        if self.moving.is_none() && self.frame.is_multiple_of(RESTEER_FRAMES) {
            self.pan_target = steer_toward_boundary(self.pan, c, zoom);
        }
        let mut sx = (self.pan_target[0] - self.pan[0]) * PAN_EASE;
        let mut sy = (self.pan_target[1] - self.pan[1]) * PAN_EASE;
        let smag = (sx * sx + sy * sy).sqrt();
        let cap = PAN_STEP_FRAC * zoom;
        if smag > cap {
            let s = cap / smag;
            sx *= s;
            sy *= s;
        }
        self.pan[0] += sx;
        self.pan[1] += sy;

        self.colour.update(dt, bus, e).nav_state(c, self.pan, zoom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus(energy: f32) -> hyst_core::SignalBus {
        hyst_core::SignalBus {
            energy,
            ..Default::default()
        }
    }

    /// Bites: over a long run with jittery audio, zoom never leaves the
    /// bounded dive (so the renderer stays on direct iteration), `c` stays
    /// close to the curated list, and `c` never jumps between frames.
    #[test]
    fn stays_in_bounds_and_never_snaps() {
        let mut d = CuratedJuliaDriver::new();
        let dt = 1.0 / 30.0;
        let mut prev_c: Option<[f32; 2]> = None;
        let mut visited = [false; CURATED_C.len()];
        for i in 0..(30 * 120) {
            let e = if (i / 7) % 2 == 0 { 0.9 } else { 0.1 };
            let s = d.update(dt, &bus(e));
            assert!(s.zoom >= CURATED_ZOOM_DEEPEST * 0.999, "zoom {}", s.zoom);
            assert!(s.zoom <= CURATED_ZOOM_TOP * 1.001, "zoom {}", s.zoom);
            assert!(s.zoom > DIRECT_ZOOM_MIN);
            let nearest = CURATED_C
                .iter()
                .enumerate()
                .map(|(k, c)| {
                    (
                        k,
                        ((s.c[0] - c[0]).powi(2) + (s.c[1] - c[1]).powi(2)).sqrt(),
                    )
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            if nearest.1 < 0.02 {
                visited[nearest.0] = true;
            }
            if let Some(p) = prev_c {
                let step = ((s.c[0] - p[0]).powi(2) + (s.c[1] - p[1]).powi(2)).sqrt();
                assert!(step < 0.01, "c jumped {step} at frame {i}");
            }
            prev_c = Some(s.c);
        }
        assert!(visited.iter().all(|&v| v), "visited {visited:?}");
    }

    /// Bites: a sharp rise in energy moves `c` on before the hold cadence.
    #[test]
    fn energy_jump_moves_on_early() {
        let mut d = CuratedJuliaDriver::new();
        let dt = 1.0 / 30.0;
        for _ in 0..(30 * 10) {
            d.update(dt, &bus(0.05));
        }
        assert_eq!(d.entry, 0);
        for _ in 0..30 {
            d.update(dt, &bus(0.95));
        }
        assert_eq!(
            d.entry, 1,
            "an energy jump after the minimum hold should move on"
        );
    }
}
