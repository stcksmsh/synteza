//! Bloom — ported from `bloom-pass.ts` + `blur.frag.glsl`/
//! `bright-pass.frag.glsl`. Two shapes: the single full-resolution level
//! (bright-pass -> blur H -> blur V) the "before" look uses, and
//! [`BloomChain`], a small mip chain closer to the original's
//! downsample/upsample cascade.

use crate::gpu::{GpuContext, GpuError, OffscreenTarget};
use crate::passes::{pack_slots, CommonUniforms};

pub fn render_bright_pass(
    ctx: &GpuContext,
    output: &OffscreenTarget,
    scene: &OffscreenTarget,
    sampler: &wgpu::Sampler,
    threshold: f32,
) -> Result<(), GpuError> {
    let common = CommonUniforms::new(0.0, [output.width() as f32, output.height() as f32]);
    let slots = [[threshold, 0.0, 0.0, 0.0]];
    output.render_glsl_fragment_shader(
        ctx,
        BRIGHT_PASS_GLSL,
        common.bytes(),
        Some(&pack_slots(&slots)),
        &[(scene.view(), sampler)],
    )
}

const BRIGHT_PASS_GLSL: &str = r#"
#version 450 core
layout(location = 0) in vec2 vUv;
layout(location = 0) out vec4 fragColor;
layout(std140, set = 0, binding = 0) uniform Common { float TIME; float TIMEDELTA; vec2 RENDERSIZE; int PASSINDEX; int FRAMEINDEX; vec4 DATE; };
layout(std140, set = 0, binding = 1) uniform Inputs { vec4 slot[1]; };
layout(set = 0, binding = 2) uniform texture2D uSceneTex;
layout(set = 0, binding = 3) uniform sampler uSceneSamp;
void main() {
  float threshold = slot[0].x;
  vec3 color = texture(sampler2D(uSceneTex, uSceneSamp), vUv).rgb;
  float brightness = max(color.r, max(color.g, color.b));
  float contribution = smoothstep(threshold, threshold + 0.2, brightness);
  fragColor = vec4(color * contribution, 1.0);
}
"#;

pub fn render_blur(
    ctx: &GpuContext,
    output: &OffscreenTarget,
    tex: &OffscreenTarget,
    sampler: &wgpu::Sampler,
    direction: [f32; 2],
) -> Result<(), GpuError> {
    let common = CommonUniforms::new(0.0, [output.width() as f32, output.height() as f32]);
    let texel = [1.0 / tex.width() as f32, 1.0 / tex.height() as f32];
    let slots = [
        [direction[0], direction[1], 0.0, 0.0],
        [texel[0], texel[1], 0.0, 0.0],
    ];
    output.render_glsl_fragment_shader(
        ctx,
        BLUR_GLSL,
        common.bytes(),
        Some(&pack_slots(&slots)),
        &[(tex.view(), sampler)],
    )
}

const BLUR_GLSL: &str = r#"
#version 450 core
layout(location = 0) in vec2 vUv;
layout(location = 0) out vec4 fragColor;
layout(std140, set = 0, binding = 0) uniform Common { float TIME; float TIMEDELTA; vec2 RENDERSIZE; int PASSINDEX; int FRAMEINDEX; vec4 DATE; };
layout(std140, set = 0, binding = 1) uniform Inputs { vec4 slot[2]; };
layout(set = 0, binding = 2) uniform texture2D uTexTex;
layout(set = 0, binding = 3) uniform sampler uTexSamp;
void main() {
  vec2 direction = slot[0].xy;
  vec2 texel = slot[1].xy;
  float w0 = 0.227027;
  float w1 = 0.1945946;
  float w2 = 0.1216216;
  float w3 = 0.054054;
  float w4 = 0.016216;
  vec3 result = texture(sampler2D(uTexTex, uTexSamp), vUv).rgb * w0;
  vec2 o1 = direction * texel * 1.0;
  vec2 o2 = direction * texel * 2.0;
  vec2 o3 = direction * texel * 3.0;
  vec2 o4 = direction * texel * 4.0;
  result += texture(sampler2D(uTexTex, uTexSamp), vUv + o1).rgb * w1 + texture(sampler2D(uTexTex, uTexSamp), vUv - o1).rgb * w1;
  result += texture(sampler2D(uTexTex, uTexSamp), vUv + o2).rgb * w2 + texture(sampler2D(uTexTex, uTexSamp), vUv - o2).rgb * w2;
  result += texture(sampler2D(uTexTex, uTexSamp), vUv + o3).rgb * w3 + texture(sampler2D(uTexTex, uTexSamp), vUv - o3).rgb * w3;
  result += texture(sampler2D(uTexTex, uTexSamp), vUv + o4).rgb * w4 + texture(sampler2D(uTexTex, uTexSamp), vUv - o4).rgb * w4;
  fragColor = vec4(result, 1.0);
}
"#;

/// Half-resolution bright pass followed by `LEVELS` successively halved
/// blur levels, summed back up from the smallest. Wider and smoother than
/// the single full-resolution 9-tap blur (whose radius is ~4 pixels at any
/// output size), at a fraction of its fill cost.
pub struct BloomChain {
    bright: OffscreenTarget,
    /// (horizontal scratch, blurred) per level, largest first.
    levels: Vec<(OffscreenTarget, OffscreenTarget)>,
    /// `up[k]` = `levels[k]` blurred + everything smaller, largest first.
    up: Vec<OffscreenTarget>,
}

impl BloomChain {
    const LEVELS: u32 = 4;

    pub fn new(ctx: &GpuContext, width: u32, height: u32) -> Self {
        let size = |k: u32| ((width >> k).max(1), (height >> k).max(1));
        let (bw, bh) = size(1);
        let levels = (1..=Self::LEVELS)
            .map(|k| {
                let (w, h) = size(k);
                (
                    OffscreenTarget::new_hdr(ctx, w, h),
                    OffscreenTarget::new_hdr(ctx, w, h),
                )
            })
            .collect();
        let up = (1..Self::LEVELS)
            .map(|k| {
                let (w, h) = size(k);
                OffscreenTarget::new_hdr(ctx, w, h)
            })
            .collect();
        Self {
            bright: OffscreenTarget::new_hdr(ctx, bw, bh),
            levels,
            up,
        }
    }

    /// Returns the summed bloom (half resolution; `LEVELS` levels added at
    /// equal weight, so it carries up to `LEVELS`x the bright-pass energy).
    pub fn render(
        &self,
        ctx: &GpuContext,
        scene: &OffscreenTarget,
        sampler: &wgpu::Sampler,
        threshold: f32,
    ) -> Result<&OffscreenTarget, GpuError> {
        render_bright_pass(ctx, &self.bright, scene, sampler, threshold)?;
        let mut src = &self.bright;
        for (h, v) in &self.levels {
            render_blur(ctx, h, src, sampler, [1.0, 0.0])?;
            render_blur(ctx, v, h, sampler, [0.0, 1.0])?;
            src = v;
        }
        let mut acc = &self.levels[self.levels.len() - 1].1;
        for (k, out) in self.up.iter().enumerate().rev() {
            render_add(ctx, out, &self.levels[k].1, acc, sampler)?;
            acc = out;
        }
        Ok(acc)
    }
}

fn render_add(
    ctx: &GpuContext,
    output: &OffscreenTarget,
    a: &OffscreenTarget,
    b: &OffscreenTarget,
    sampler: &wgpu::Sampler,
) -> Result<(), GpuError> {
    let common = CommonUniforms::new(0.0, [output.width() as f32, output.height() as f32]);
    output.render_glsl_fragment_shader(
        ctx,
        ADD_GLSL,
        common.bytes(),
        None,
        &[(a.view(), sampler), (b.view(), sampler)],
    )
}

const ADD_GLSL: &str = r#"
#version 450 core
layout(location = 0) in vec2 vUv;
layout(location = 0) out vec4 fragColor;
layout(std140, set = 0, binding = 0) uniform Common { float TIME; float TIMEDELTA; vec2 RENDERSIZE; int PASSINDEX; int FRAMEINDEX; vec4 DATE; };
layout(set = 0, binding = 2) uniform texture2D uATex;
layout(set = 0, binding = 3) uniform sampler uASamp;
layout(set = 0, binding = 4) uniform texture2D uBTex;
layout(set = 0, binding = 5) uniform sampler uBSamp;
void main() {
  vec3 a = texture(sampler2D(uATex, uASamp), vUv).rgb;
  vec3 b = texture(sampler2D(uBTex, uBSamp), vUv).rgb;
  fragColor = vec4(a + b, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bright_pass_keeps_only_pixels_above_threshold() {
        let ctx = match GpuContext::new_blocking() {
            Ok(c) => c,
            Err(_) => return,
        };
        let sampler = ctx.create_clamp_sampler();
        let scene = OffscreenTarget::new(&ctx, 64, 64);
        // Left half dim (below threshold), right half bright (above).
        scene
            .render_glsl_fragment_shader(
                &ctx,
                r#"#version 450 core
layout(location=0) in vec2 vUv;
layout(location=0) out vec4 fragColor;
void main() { float v = step(0.5, vUv.x); fragColor = vec4(vec3(v), 1.0); }
"#,
                &[0u8; 32],
                None,
                &[],
            )
            .unwrap();
        let out = OffscreenTarget::new(&ctx, 64, 64);
        render_bright_pass(&ctx, &out, &scene, &sampler, 0.5).unwrap();
        let pixels = out.read_pixels(&ctx).unwrap();
        let px = |x: u32| pixels[((32 * 64 + x) * 4) as usize];
        assert!(
            px(5) < 10,
            "dim half should be crushed to ~0, got {}",
            px(5)
        );
        assert!(
            px(60) > 200,
            "bright half should survive threshold, got {}",
            px(60)
        );
    }

    #[test]
    fn blur_spreads_a_single_bright_pixel() {
        let ctx = match GpuContext::new_blocking() {
            Ok(c) => c,
            Err(_) => return,
        };
        let sampler = ctx.create_clamp_sampler();
        let tex = OffscreenTarget::new(&ctx, 64, 64);
        tex.render_glsl_fragment_shader(
            &ctx,
            r#"#version 450 core
layout(location=0) in vec2 vUv;
layout(location=0) out vec4 fragColor;
void main() {
  float d = distance(vUv, vec2(0.5));
  fragColor = vec4(vec3(d < 0.02 ? 1.0 : 0.0), 1.0);
}
"#,
            &[0u8; 32],
            None,
            &[],
        )
        .unwrap();
        let h = OffscreenTarget::new(&ctx, 64, 64);
        render_blur(&ctx, &h, &tex, &sampler, [1.0, 0.0]).unwrap();
        let pixels = h.read_pixels(&ctx).unwrap();
        // A few pixels beside the original spike should now be nonzero (blur spread it).
        let px = |x: u32, y: u32| pixels[((y * 64 + x) * 4) as usize];
        assert!(
            px(35, 32) > 0,
            "blur should spread energy sideways from the spike, got 0 at (35,32)"
        );
    }
}
