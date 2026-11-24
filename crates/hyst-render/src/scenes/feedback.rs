//! Video-synth feedback: three cross-modulated oscillators, a soft luma key,
//! a colouriser, and a feedback loop that zooms, rotates and shifts hue each
//! frame at close to unity gain, held in range by a soft limiter.
//!
//! Oscillator phases are read off the continuous beat count and the
//! rotation off the bar count, so motion locks to the song's grid without
//! ever scaling a phase by a signal. Zoom and hue shift are per-frame rates.
//! Onsets kick the key threshold down, letting more of the oscillators in.

use crate::gpu::{GpuContext, GpuError, OffscreenTarget};

use super::music::SceneInputs;
use super::{glsl_header, run_pass};

const BODY: &str = r#"
vec3 hueRot(vec3 c, float a) {
  const vec3 k = vec3(0.57735);
  float ca = cos(a);
  return c * ca + cross(k, c) * sin(a) + k * dot(k, c) * (1.0 - ca);
}

void main() {
  float aspect = slot[0].x;
  float rot = slot[0].y;
  float zoom = slot[0].z;
  float hue = slot[0].w;
  float gain = slot[1].x;
  vec3 ph = slot[1].yzw;
  vec3 fr = slot[2].xyz;
  float xmod = slot[2].w;
  float thr = slot[3].x;
  float soft = slot[3].y;
  float inject = slot[3].z;
  float pal = slot[3].w;

  vec2 p = (vUv - 0.5) * vec2(aspect, 1.0);
  float cr = cos(rot), sr = sin(rot);
  vec2 q = mat2(cr, sr, -sr, cr) * p / zoom;
  vec3 fb = texture(sampler2D(uTex0, uSamp0), q / vec2(aspect, 1.0) + 0.5).rgb;
  fb = max(hueRot(fb, hue), vec3(0.0)) * gain;
  float pk = max(fb.r, max(fb.g, fb.b));
  const float knee = 0.8;
  const float ceiling = 1.6;
  if (pk > knee) {
    float m = knee + (ceiling - knee) * tanh((pk - knee) / (ceiling - knee));
    fb *= m / pk;
  }

  float o1 = sin(p.x * fr.x + ph.x);
  float o2 = sin(p.y * fr.y + ph.y + xmod * o1);
  float o3 = sin(length(p) * fr.z - ph.z + xmod * o2);
  float v = 0.5 + 0.5 * (0.3 * o1 + 0.3 * o2 + 0.4 * o3);
  float key = smoothstep(thr - soft, thr + soft, v);
  vec3 colr = 0.55 + 0.45 * cos(6.2831853 * (vec3(0.0, 0.15, 0.35) + v * 0.5 + pal));
  fragColor = vec4(mix(fb, colr * 0.7, key * inject), 1.0);
}
"#;

pub struct FeedbackScene {
    aspect: f32,
    state: [OffscreenTarget; 2],
    cur: usize,
    clamp: wgpu::Sampler,
    glsl: String,
    prev_angle: Option<f32>,
}

/// Feedback rotation as a function of the bar count: a slow swing back and
/// forth over 16 bars on a small steady drift.
fn angle(bar: f32) -> f32 {
    0.35 * (std::f32::consts::TAU * bar / 16.0).sin() + 0.03 * bar
}

impl FeedbackScene {
    pub fn new(ctx: &GpuContext, width: u32, height: u32) -> Self {
        Self {
            aspect: width as f32 / height as f32,
            state: [
                OffscreenTarget::new_hdr(ctx, width, height),
                OffscreenTarget::new_hdr(ctx, width, height),
            ],
            cur: 0,
            clamp: ctx.create_clamp_sampler(),
            glsl: glsl_header(4, 1) + BODY,
            prev_angle: None,
        }
    }

    pub fn output(&self) -> &OffscreenTarget {
        &self.state[self.cur]
    }

    pub fn render(&mut self, ctx: &GpuContext, inp: &SceneInputs) -> Result<(), GpuError> {
        use std::f32::consts::TAU;
        let a = angle(inp.bar);
        let rot = a - self.prev_angle.unwrap_or(a);
        self.prev_angle = Some(a);
        let e = inp.energy_slow;
        let slots = [
            [
                self.aspect,
                rot,
                1.0 + 0.002 + 0.005 * e,
                inp.dt * (0.15 + 0.5 * inp.energy),
            ],
            [
                0.93 + 0.03 * e,
                inp.beat * TAU / 8.0,
                inp.beat * TAU / 12.0,
                inp.beat * TAU / 2.0,
            ],
            [7.0, 11.0, 14.0 + 10.0 * e, 1.2 + 1.5 * e],
            [
                0.82 - 0.2 * inp.kick,
                0.03,
                0.35 + 0.25 * inp.energy,
                inp.bar / 32.0,
            ],
        ];
        let (src, dst) = (&self.state[self.cur], &self.state[1 - self.cur]);
        run_pass(ctx, dst, &self.glsl, &slots, &[(src.view(), &self.clamp)])?;
        self.cur = 1 - self.cur;
        Ok(())
    }
}
