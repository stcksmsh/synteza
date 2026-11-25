//! Sketch scenes that are not fractals: a precise line grid, a
//! reaction-diffusion growth and a video-synth feedback loop, plus a
//! switcher that gives each song section a scene and dissolves between
//! them. Rough sketches for the owner to judge by eye, not finished scenes.
//!
//! Every scene renders into its own `Rgba16Float` target in field units
//! (about 0..2). [`SceneRenderer`] mixes the scenes by weight in HDR, then
//! runs the shared output chain (mip bloom, hue-keeping tonemap, dither).

pub mod avproof;
pub mod feedback;
pub mod grid;
pub mod growth;
pub mod music;
pub mod switcher;

use crate::gpu::{GpuContext, GpuError, OffscreenTarget};
use crate::passes::bloom::BloomChain;
use crate::passes::composite::render_composite_hdr;
use crate::passes::{pack_slots, CommonUniforms};

pub use music::{SceneInputs, SongTrack};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SceneKind {
    Grid = 0,
    Growth = 1,
    Feedback = 2,
}

impl SceneKind {
    pub const ALL: [SceneKind; 3] = [SceneKind::Grid, SceneKind::Growth, SceneKind::Feedback];

    pub fn name(self) -> &'static str {
        match self {
            SceneKind::Grid => "grid",
            SceneKind::Growth => "growth",
            SceneKind::Feedback => "feedback",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// The GLSL preamble every scene pass shares: the `Common` block, an
/// `Inputs` block of `slots` vec4s and `textures` texture/sampler pairs
/// named `uTex0`/`uSamp0`, ...
pub(crate) fn glsl_header(slots: usize, textures: usize) -> String {
    let mut s = format!(
        "#version 450 core\n\
         layout(location = 0) in vec2 vUv;\n\
         layout(location = 0) out vec4 fragColor;\n\
         layout(std140, set = 0, binding = 0) uniform Common {{ float TIME; float TIMEDELTA; vec2 RENDERSIZE; int PASSINDEX; int FRAMEINDEX; vec4 DATE; }};\n\
         layout(std140, set = 0, binding = 1) uniform Inputs {{ vec4 slot[{slots}]; }};\n"
    );
    for i in 0..textures {
        s += &format!(
            "layout(set = 0, binding = {}) uniform texture2D uTex{i};\n\
             layout(set = 0, binding = {}) uniform sampler uSamp{i};\n",
            2 + 2 * i,
            3 + 2 * i
        );
    }
    s
}

/// Runs one scene pass into `target` with `slots` as its `Inputs` block.
pub(crate) fn run_pass(
    ctx: &GpuContext,
    target: &OffscreenTarget,
    glsl: &str,
    slots: &[[f32; 4]],
    textures: &[(&wgpu::TextureView, &wgpu::Sampler)],
) -> Result<(), GpuError> {
    let common = CommonUniforms::new(0.0, [target.width() as f32, target.height() as f32]);
    target.render_glsl_fragment_shader(
        ctx,
        glsl,
        common.bytes(),
        Some(&pack_slots(slots)),
        textures,
    )
}

const MIX_BODY: &str = r#"
void main() {
  vec3 a = texture(sampler2D(uTex0, uSamp0), vUv).rgb;
  vec3 b = texture(sampler2D(uTex1, uSamp1), vUv).rgb;
  vec3 c = texture(sampler2D(uTex2, uSamp2), vUv).rgb;
  fragColor = vec4(a * slot[0].x + b * slot[0].y + c * slot[0].z, 1.0);
}
"#;

const BLOOM_THRESHOLD: f32 = 1.0;
const BLOOM_STRENGTH: f32 = 0.35;
const EXPOSURE: f32 = 1.0;

/// Owns the three scenes and the output chain.
pub struct SceneRenderer {
    width: u32,
    height: u32,
    clamp: wgpu::Sampler,
    pub grid: grid::GridScene,
    pub growth: growth::GrowthScene,
    pub feedback: feedback::FeedbackScene,
    mixed: OffscreenTarget,
    bloom: BloomChain,
    final_out: OffscreenTarget,
    frame_index: u32,
}

impl SceneRenderer {
    /// `width` must be a multiple of 64 (readback alignment).
    pub fn new(ctx: &GpuContext, width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            clamp: ctx.create_clamp_sampler(),
            grid: grid::GridScene::new(ctx, width, height),
            growth: growth::GrowthScene::new(ctx, width, height),
            feedback: feedback::FeedbackScene::new(ctx, width, height),
            mixed: OffscreenTarget::new_hdr(ctx, width, height),
            bloom: BloomChain::new(ctx, width, height),
            final_out: OffscreenTarget::new(ctx, width, height),
            frame_index: 0,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Renders every scene whose weight is above zero, mixes them by
    /// `weights` (indexed by [`SceneKind`]) in HDR, and returns the
    /// tonemapped RGBA8 frame.
    pub fn render(
        &mut self,
        ctx: &GpuContext,
        inputs: &SceneInputs,
        weights: [f32; 3],
    ) -> Result<Vec<u8>, GpuError> {
        self.frame_index = self.frame_index.wrapping_add(1);
        const ACTIVE: f32 = 1e-4;
        if weights[SceneKind::Grid as usize] > ACTIVE {
            self.grid.render(ctx, inputs)?;
        }
        if weights[SceneKind::Growth as usize] > ACTIVE {
            self.growth.render(ctx, inputs)?;
        }
        if weights[SceneKind::Feedback as usize] > ACTIVE {
            self.feedback.render(ctx, inputs)?;
        }
        let s = &self.clamp;
        run_pass(
            ctx,
            &self.mixed,
            &(glsl_header(1, 3) + MIX_BODY),
            &[[weights[0], weights[1], weights[2], 0.0]],
            &[
                (self.grid.output().view(), s),
                (self.growth.output().view(), s),
                (self.feedback.output().view(), s),
            ],
        )?;
        let bloom = self.bloom.render(ctx, &self.mixed, s, BLOOM_THRESHOLD)?;
        render_composite_hdr(
            ctx,
            &self.final_out,
            &self.mixed,
            bloom,
            s,
            [BLOOM_STRENGTH, EXPOSURE],
            self.frame_index,
        )?;
        self.final_out.read_pixels(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(i: usize) -> SceneInputs {
        let t = i as f32 / 30.0;
        SceneInputs {
            t,
            dt: 1.0 / 30.0,
            beat: t * 2.0,
            bar: t * 0.5,
            energy: 0.6,
            energy_slow: 0.5,
            sub: 0.5,
            air: 0.5,
            onsets: if i.is_multiple_of(10) {
                vec![music::Onset {
                    strength: 0.9,
                    tone: 0.5,
                    pan: 0.2,
                    t,
                }]
            } else {
                Vec::new()
            },
            kick: if i.is_multiple_of(10) { 0.9 } else { 0.2 },
        }
    }

    /// Each scene alone, and a dissolve, give a changing frame that is
    /// neither black nor blown out.
    #[test]
    fn every_scene_renders_changing_non_degenerate_frames() {
        let ctx = match GpuContext::new_blocking() {
            Ok(c) => c,
            Err(_) => return,
        };
        let mean = |p: &[u8]| p.iter().map(|&b| b as f64).sum::<f64>() / p.len() as f64;
        for weights in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.5, 0.5],
        ] {
            let mut r = SceneRenderer::new(&ctx, 128, 64);
            let mut frames = Vec::new();
            for i in 0..20 {
                frames.push(r.render(&ctx, &inputs(i), weights).unwrap());
            }
            let m = mean(&frames[19]);
            assert!(
                m > 1.0 && m < 240.0,
                "{weights:?}: frame should be non-degenerate, mean {m}"
            );
            assert_ne!(frames[18], frames[19], "{weights:?}: frames should change");
        }
    }
}
