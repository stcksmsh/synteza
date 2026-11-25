//! Minimal line work: a fine grid with major lines that steps one cell on
//! every bar, a moire window of two ring gratings, a sweep line crossing the
//! frame once per bar, cells lit on the beat and crosshairs on onsets.
//! Mostly monochrome with one accent colour; every line is analytic and
//! anti-aliased over about one pixel. Nothing ever lights the whole field.

use crate::gpu::{GpuContext, GpuError, OffscreenTarget};

use super::music::{Onset, SceneInputs};
use super::{glsl_header, run_pass};

/// Vermilion, the one accent.
const ACCENT: [f32; 3] = [1.0, 0.2, 0.07];
/// Seconds an onset crosshair stays visible.
const ONSET_LIFE: f32 = 0.8;

const BODY: &str = r#"
float PX;
float hash11(float n) { return fract(sin(n * 127.1 + 311.7) * 43758.5453); }
// Coverage of a line of half-width w at distance d, anti-aliased over a pixel.
float band(float d, float w) { return 1.0 - smoothstep(w - PX, w + PX, d); }

void main() {
  float aspect = slot[0].x;
  float beat = slot[0].y;
  float bar = slot[0].z;
  float energy = slot[0].w;
  float eslow = slot[1].x;
  float drift = slot[1].z;
  vec3 accent = slot[6].rgb;
  PX = 1.0 / RENDERSIZE.y;
  vec2 p = (vUv - 0.5) * vec2(aspect, 1.0);
  vec3 ink = vec3(0.82, 0.85, 0.88);
  vec3 col = vec3(0.004, 0.0045, 0.005);

  // The grid steps one cell sideways per bar, eased over the bar's first fifth.
  float cell = 1.0 / 8.0;
  float barIdx = floor(bar);
  float barF = fract(bar);
  float stepPos = barIdx + smoothstep(0.0, 0.2, barF);
  vec2 off = vec2(stepPos * cell, 0.0);
  vec2 q = p + off;
  vec2 dMinor = abs(fract(q / cell + 0.5) - 0.5) * cell;
  float minor = max(band(dMinor.x, 0.5 * PX), band(dMinor.y, 0.5 * PX));
  vec2 dMajor = abs(fract(q / (4.0 * cell) + 0.5) - 0.5) * 4.0 * cell;
  float major = max(band(dMajor.x, 0.8 * PX), band(dMajor.y, 0.8 * PX));
  col += ink * (0.05 * minor + 0.2 * major);

  // Cells lit on the beat, attached to the grid; more of them when louder.
  vec2 ci = floor(q / cell);
  float bIdx = floor(beat);
  float bF = fract(beat);
  float lit = 0.0;
  int n = 1 + int(energy * 2.99);
  for (int j = 0; j < 3; j++) {
    if (j >= n) break;
    vec2 sp = vec2((hash11(bIdx * 3.0 + float(j)) - 0.5) * aspect * 0.9,
                   (hash11(bIdx * 7.0 + float(j) + 0.5) - 0.5) * 0.9);
    vec2 pick = floor((sp + vec2((barIdx + 1.0) * cell, 0.0)) / cell);
    if (all(equal(ci, pick))) lit += exp(-bF * 4.0);
  }
  vec2 inCell = fract(q / cell) * cell;
  float edge = min(min(inCell.x, cell - inCell.x), min(inCell.y, cell - inCell.y));
  float outline = band(abs(edge - 3.0 * PX), 0.6 * PX);
  col += accent * lit * (0.15 + 1.4 * outline);

  // Moire window: two ring gratings whose centres drift apart and together.
  float R = 0.2 + 0.08 * eslow;
  vec2 c2 = vec2(0.04 * sin(drift), 0.025 * cos(drift * 0.7));
  float s = 6.0 * PX;
  float r = length(p);
  float g1 = 0.5 + 0.5 * cos(6.2831853 * r / s);
  float g2 = 0.5 + 0.5 * cos(6.2831853 * length(p - c2) / s);
  float win = 1.0 - smoothstep(R - PX, R + PX, r);
  col = mix(col, vec3(0.004), win * 0.9);
  col += ink * win * g1 * g2 * (0.08 + 0.22 * energy);
  col += ink * 0.35 * band(abs(r - R), 0.6 * PX);

  // The sweep crosses the frame once per bar, turning round at each bar line.
  float dir = mod(barIdx, 2.0) < 0.5 ? 1.0 : -1.0;
  float f = dir > 0.0 ? barF : 1.0 - barF;
  float dx = p.x - (f - 0.5) * aspect;
  float behind = -dx * dir;
  float trail = behind > 0.0 ? exp(-behind * 14.0) : 0.0;
  col += accent * (1.6 * band(abs(dx), 0.7 * PX) + 0.1 * trail * (0.3 + energy));

  // Onset crosshairs, snapped to the nearest grid intersection.
  for (int i = 0; i < 4; i++) {
    vec4 o = slot[2 + i];
    if (o.w <= 0.0) continue;
    vec2 at = floor((o.xy + off) / cell + 0.5) * cell - off;
    vec2 d = abs(p - at);
    float arm = 0.012 + 0.03 * o.w;
    float armX = 1.0 - smoothstep(arm - PX, arm + PX, d.x);
    float armY = 1.0 - smoothstep(arm - PX, arm + PX, d.y);
    float cross = max(band(d.x, 0.6 * PX) * armY, band(d.y, 0.6 * PX) * armX);
    col += ink * cross * exp(-o.z * 5.0) * (0.6 + 0.9 * o.w);
  }
  fragColor = vec4(col, 1.0);
}
"#;

pub struct GridScene {
    aspect: f32,
    out: OffscreenTarget,
    glsl: String,
    /// Integrated moire drift, radians.
    drift: f32,
    recent: Vec<Onset>,
}

impl GridScene {
    pub fn new(ctx: &GpuContext, width: u32, height: u32) -> Self {
        Self {
            aspect: width as f32 / height as f32,
            out: OffscreenTarget::new_hdr(ctx, width, height),
            glsl: glsl_header(7, 0) + BODY,
            drift: 0.0,
            recent: Vec::new(),
        }
    }

    pub fn output(&self) -> &OffscreenTarget {
        &self.out
    }

    pub fn render(&mut self, ctx: &GpuContext, inp: &SceneInputs) -> Result<(), GpuError> {
        self.drift += inp.dt * (0.08 + 0.3 * inp.energy_slow);
        self.recent
            .extend(inp.onsets.iter().filter(|o| o.strength > 0.3).copied());
        self.recent.retain(|o| inp.t - o.t < ONSET_LIFE);
        let mut slots = [[0.0f32; 4]; 7];
        slots[0] = [self.aspect, inp.beat, inp.bar, inp.energy];
        slots[1] = [inp.energy_slow, inp.kick, self.drift, inp.air];
        for (slot, o) in slots[2..6].iter_mut().zip(self.recent.iter().rev()) {
            *slot = [
                o.pan * 0.42 * self.aspect,
                (0.5 - o.tone) * 0.8,
                inp.t - o.t,
                o.strength,
            ];
        }
        slots[6] = [ACCENT[0], ACCENT[1], ACCENT[2], 0.0];
        run_pass(ctx, &self.out, &self.glsl, &slots, &[])
    }
}
