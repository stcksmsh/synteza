//! Gray-Scott reaction-diffusion at half resolution on a ping-pong pair,
//! several steps per frame, shaded as relief with a restrained palette.
//!
//! The state stores `1 - U` rather than `U`: the background sits at U = 1,
//! where a half float only resolves steps of about 5e-4, too coarse for the
//! slow feed term; near 0 it resolves far finer. The third channel is heat,
//! set where an onset injects and decaying, which tints fresh growth.
//!
//! Onsets inject V locally (pan picks x, tone picks y). The song's slow
//! energy moves feed and kill along [`FEED_KILL_PATH`] over about ten
//! seconds, from dividing spots when quiet to worms when loud. The feed also
//! drifts across the frame, bar-locked, so fronts where the pattern dies
//! back and regrows keep moving.

use crate::gpu::{GpuContext, GpuError, OffscreenTarget};

use super::music::SceneInputs;
use super::{glsl_header, run_pass};

/// Simulation steps per video frame.
pub const STEPS_PER_FRAME: usize = 24;
/// (feed, kill) from quiet to loud: dividing spots (mitosis), spots
/// stretching into worms, worms.
pub const FEED_KILL_PATH: [[f32; 2]; 3] = [[0.0367, 0.0649], [0.042, 0.0638], [0.046, 0.063]];
/// Amber, the one accent, on fresh growth.
const ACCENT: [f32; 3] = [1.0, 0.5, 0.16];

const STEP_BODY: &str = r#"
float hash12(vec2 p) {
  vec3 p3 = fract(vec3(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}
vec3 st(vec2 o) { return texture(sampler2D(uTex0, uSamp0), vUv + o * slot[1].xy).rgb; }

void main() {
  float aspect = slot[1].z;
  // Feed drifts across the frame so fronts between regimes keep moving.
  float drift = slot[0].z;
  float F = slot[0].x + 0.006 * sin(vUv.x * aspect * 3.0 + drift) * cos(vUv.y * 2.0 - 0.7 * drift);
  float K = slot[0].y;
  if (slot[0].w > 0.5) {
    // Initial state: sparse seeds on a coarse jittered lattice.
    vec2 g = vUv * vec2(18.0 * aspect, 18.0);
    vec2 cell = floor(g);
    vec2 f = fract(g) - 0.5;
    float on = (hash12(cell + slot[1].w) > 0.8 && length(f) < 0.35) ? 1.0 : 0.0;
    fragColor = vec4(0.5 * on, 0.5 * on, 0.0, 1.0);
    return;
  }
  vec3 c = st(vec2(0.0));
  vec3 lap = 0.2 * (st(vec2(1.0, 0.0)) + st(vec2(-1.0, 0.0)) + st(vec2(0.0, 1.0)) + st(vec2(0.0, -1.0)))
           + 0.05 * (st(vec2(1.0, 1.0)) + st(vec2(-1.0, 1.0)) + st(vec2(1.0, -1.0)) + st(vec2(-1.0, -1.0)))
           - c;
  float U = 1.0 - c.x;
  float V = c.y;
  float uvv = U * V * V;
  // Du = 1, Dv = 0.5, dt = 1; lap(U) = -lap(1 - U).
  float U2 = U + (-lap.x - uvv + F * (1.0 - U));
  float V2 = V + (0.5 * lap.y + uvv - (F + K) * V);
  float heat = c.z * 0.995 + 0.2 * lap.z;
  for (int i = 0; i < 4; i++) {
    vec4 inj = slot[2 + i];
    if (inj.w <= 0.0) continue;
    float d = length((vUv - inj.xy) * vec2(aspect, 1.0));
    float k = 1.0 - smoothstep(0.6 * inj.z, inj.z, d);
    V2 = max(V2, 0.5 * inj.w * k);
    U2 = min(U2, mix(1.0, 0.5, k));
    heat = max(heat, inj.w * k);
  }
  fragColor = vec4(1.0 - clamp(U2, 0.0, 1.0), clamp(V2, 0.0, 1.0), clamp(heat, 0.0, 1.0), 1.0);
}
"#;

const SHADE_BODY: &str = r#"
vec3 st(vec2 o) { return texture(sampler2D(uTex0, uSamp0), vUv + o * slot[1].xy).rgb; }

void main() {
  float energy = slot[0].x;
  float light = slot[0].y;
  vec3 accent = slot[2].rgb;
  vec3 c = st(vec2(0.0));
  float V = c.y;
  float vx = st(vec2(1.0, 0.0)).y - st(vec2(-1.0, 0.0)).y;
  float vy = st(vec2(0.0, 1.0)).y - st(vec2(0.0, -1.0)).y;
  vec3 n = normalize(vec3(-vx * 6.0, -vy * 6.0, 1.0));
  vec3 L = normalize(vec3(cos(light), sin(light), 1.1));
  float diff = max(dot(n, L), 0.0);
  float spec = pow(max(dot(reflect(-L, n), vec3(0.0, 0.0, 1.0)), 0.0), 24.0);
  float body = smoothstep(0.08, 0.4, V);
  vec3 base = mix(vec3(0.006, 0.007, 0.009), vec3(0.4, 0.39, 0.35), body);
  vec3 col = base * (0.3 + 0.9 * diff) + vec3(0.9, 0.88, 0.82) * spec * 0.5 * body;
  col += accent * c.z * body * 1.4;
  fragColor = vec4(col * (0.75 + 0.5 * energy), 1.0);
}
"#;

pub struct GrowthScene {
    aspect: f32,
    state: [OffscreenTarget; 2],
    /// Which of `state` holds the latest step.
    cur: usize,
    out: OffscreenTarget,
    repeat: wgpu::Sampler,
    clamp: wgpu::Sampler,
    step_glsl: String,
    shade_glsl: String,
    initialised: bool,
    /// Position along [`FEED_KILL_PATH`], 0..1, smoothed.
    path: Option<f32>,
    /// Phase of the feed's spatial drift, radians.
    drift: f32,
}

/// Feed and kill at position `s` (0..1) along [`FEED_KILL_PATH`].
pub fn feed_kill(s: f32) -> [f32; 2] {
    let x = s.clamp(0.0, 1.0) * (FEED_KILL_PATH.len() - 1) as f32;
    let i = (x as usize).min(FEED_KILL_PATH.len() - 2);
    let f = x - i as f32;
    let (a, b) = (FEED_KILL_PATH[i], FEED_KILL_PATH[i + 1]);
    [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
}

impl GrowthScene {
    pub fn new(ctx: &GpuContext, width: u32, height: u32) -> Self {
        let (sw, sh) = ((width / 2).max(1), (height / 2).max(1));
        Self {
            aspect: width as f32 / height as f32,
            state: [
                OffscreenTarget::new_hdr(ctx, sw, sh),
                OffscreenTarget::new_hdr(ctx, sw, sh),
            ],
            cur: 0,
            out: OffscreenTarget::new_hdr(ctx, width, height),
            repeat: ctx.create_repeat_sampler(),
            clamp: ctx.create_clamp_sampler(),
            step_glsl: glsl_header(6, 1) + STEP_BODY,
            shade_glsl: glsl_header(3, 1) + SHADE_BODY,
            initialised: false,
            path: None,
            drift: 0.0,
        }
    }

    pub fn output(&self) -> &OffscreenTarget {
        &self.out
    }

    /// The latest simulation state: r = 1 - U, g = V, b = heat.
    pub fn state(&self) -> &OffscreenTarget {
        &self.state[self.cur]
    }

    /// Advances the simulation `steps` steps at the given feed and kill;
    /// `injections` (x, y in uv, radius in uv height, strength) apply on the
    /// first step.
    pub fn step(
        &mut self,
        ctx: &GpuContext,
        feed_kill: [f32; 2],
        injections: &[[f32; 4]],
        steps: usize,
    ) -> Result<(), GpuError> {
        let src = self.state[self.cur].width() as f32;
        let texel = [
            1.0 / src,
            1.0 / self.state[self.cur].height() as f32,
            self.aspect,
            7.0,
        ];
        if !self.initialised {
            let slots = [
                [0.0, 0.0, 0.0, 1.0],
                texel,
                [0.0; 4],
                [0.0; 4],
                [0.0; 4],
                [0.0; 4],
            ];
            // The shader declares its input texture even when initialising.
            let (dst, unused) = (&self.state[self.cur], &self.state[1 - self.cur]);
            run_pass(
                ctx,
                dst,
                &self.step_glsl,
                &slots,
                &[(unused.view(), &self.repeat)],
            )?;
            self.initialised = true;
        }
        for k in 0..steps {
            let mut slots = [[0.0f32; 4]; 6];
            slots[0] = [feed_kill[0], feed_kill[1], self.drift, 0.0];
            slots[1] = texel;
            if k == 0 {
                for (slot, inj) in slots[2..].iter_mut().zip(injections) {
                    *slot = *inj;
                }
            }
            let (src, dst) = (&self.state[self.cur], &self.state[1 - self.cur]);
            run_pass(
                ctx,
                dst,
                &self.step_glsl,
                &slots,
                &[(src.view(), &self.repeat)],
            )?;
            self.cur = 1 - self.cur;
        }
        Ok(())
    }

    pub fn render(&mut self, ctx: &GpuContext, inp: &SceneInputs) -> Result<(), GpuError> {
        let path = self.path.get_or_insert(inp.energy_slow);
        *path += (inp.energy_slow - *path) * (inp.dt / 10.0).min(1.0);
        let fk = feed_kill(*path);
        // One radian every eight bars: the spatial feed pattern is bar-locked.
        self.drift = inp.bar / 8.0;
        let injections: Vec<[f32; 4]> = inp
            .onsets
            .iter()
            .filter(|o| o.strength > 0.3)
            .map(|o| {
                // A small per-onset jitter so repeated onsets do not stack.
                let j = (o.t * 91.7).sin() * 0.08;
                [
                    0.5 + 0.42 * o.pan + j,
                    0.5 + 0.38 * (0.5 - o.tone) * 2.0 - j,
                    0.012 + 0.025 * o.strength,
                    o.strength,
                ]
            })
            .collect();
        self.step(ctx, fk, &injections, STEPS_PER_FRAME)?;
        let st = &self.state[self.cur];
        let slots = [
            [
                inp.energy,
                std::f32::consts::TAU * inp.bar / 32.0 + 2.2,
                0.0,
                0.0,
            ],
            [1.0 / st.width() as f32, 1.0 / st.height() as f32, 0.0, 0.0],
            [ACCENT[0], ACCENT[1], ACCENT[2], 0.0],
        ];
        run_pass(
            ctx,
            &self.out,
            &self.shade_glsl,
            &slots,
            &[(st.view(), &self.clamp)],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encodes the state as RGBA8: r = V, g = 1 - U, b = 1 wherever a
    /// channel is NaN or outside 0..1.
    const CHECK_BODY: &str = r#"
void main() {
  vec3 c = texture(sampler2D(uTex0, uSamp0), vUv).rgb;
  bool bad = any(isnan(c)) || any(lessThan(c, vec3(0.0))) || any(greaterThan(c, vec3(1.0)));
  fragColor = vec4(c.y, c.x, bad ? 1.0 : 0.0, 1.0);
}
"#;

    /// After 600 steps with onsets injecting, the field stays in range and
    /// holds a pattern: some cells grown, some empty, no NaN.
    #[test]
    fn reaction_diffusion_stays_bounded_and_patterned() {
        let ctx = match GpuContext::new_blocking() {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut g = GrowthScene::new(&ctx, 256, 256);
        for i in 0..50 {
            let inj = [[0.3 + 0.01 * i as f32, 0.6, 0.03, 0.9]];
            let injections: &[[f32; 4]] = if i % 10 == 0 { &inj } else { &[] };
            g.step(&ctx, feed_kill(0.5), injections, STEPS_PER_FRAME)
                .unwrap();
        }
        let check = OffscreenTarget::new(&ctx, 128, 128);
        let sampler = ctx.create_clamp_sampler();
        run_pass(
            &ctx,
            &check,
            &(glsl_header(1, 1) + CHECK_BODY),
            &[[0.0; 4]],
            &[(g.state().view(), &sampler)],
        )
        .unwrap();
        let px = check.read_pixels(&ctx).unwrap();
        let n = px.len() / 4;
        let bad = px.chunks(4).filter(|p| p[2] > 0).count();
        assert_eq!(bad, 0, "{bad} pixels out of range or NaN");
        let grown = px.chunks(4).filter(|p| p[0] > 40).count();
        let empty = px.chunks(4).filter(|p| p[0] < 8).count();
        assert!(
            grown > n / 50 && empty > n / 50,
            "pattern degenerate: {grown} grown, {empty} empty of {n}"
        );
    }
}
