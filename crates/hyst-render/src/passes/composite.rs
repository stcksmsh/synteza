//! Final composite — ported from `composite.frag.glsl`: additive bloom,
//! Reinhard tonemap, gamma-correct output.

use crate::gpu::{GpuContext, GpuError, OffscreenTarget};
use crate::passes::{pack_slots, CommonUniforms};

#[allow(clippy::too_many_arguments)]
pub fn render_composite(
    ctx: &GpuContext,
    output: &OffscreenTarget,
    scene: &OffscreenTarget,
    bloom: Option<&OffscreenTarget>,
    sampler: &wgpu::Sampler,
    bloom_strength: f32,
    exposure: f32,
) -> Result<(), GpuError> {
    let common = CommonUniforms::new(0.0, [output.width() as f32, output.height() as f32]);
    let has_bloom = if bloom.is_some() { 1.0 } else { 0.0 };
    let slots = [[bloom_strength, has_bloom, exposure, 0.0]];
    let bloom_tex = bloom.unwrap_or(scene); // dummy binding when absent; uHasBloom gates its use in-shader
    output.render_glsl_fragment_shader(
        ctx,
        COMPOSITE_GLSL,
        common.bytes(),
        Some(&pack_slots(&slots)),
        &[(scene.view(), sampler), (bloom_tex.view(), sampler)],
    )
}

const COMPOSITE_GLSL: &str = r#"
#version 450 core
layout(location = 0) in vec2 vUv;
layout(location = 0) out vec4 fragColor;
layout(std140, set = 0, binding = 0) uniform Common { float TIME; float TIMEDELTA; vec2 RENDERSIZE; int PASSINDEX; int FRAMEINDEX; vec4 DATE; };
layout(std140, set = 0, binding = 1) uniform Inputs { vec4 slot[1]; };
layout(set = 0, binding = 2) uniform texture2D uSceneTex;
layout(set = 0, binding = 3) uniform sampler uSceneSamp;
layout(set = 0, binding = 4) uniform texture2D uBloomTex;
layout(set = 0, binding = 5) uniform sampler uBloomSamp;

vec3 tonemapReinhard(vec3 c) { return c / (1.0 + c); }

void main() {
  float bloomStrength = slot[0].x;
  bool hasBloom = slot[0].y > 0.5;
  float exposure = slot[0].z;
  vec3 color = texture(sampler2D(uSceneTex, uSceneSamp), vUv).rgb;
  if (hasBloom) {
    color += texture(sampler2D(uBloomTex, uBloomSamp), vUv).rgb * bloomStrength;
  }
  color *= exposure;
  color = tonemapReinhard(color);
  color = pow(color, vec3(1.0 / 2.2));
  fragColor = vec4(color, 1.0);
}
"#;

/// HDR sibling of [`render_composite`]: adds bloom, applies `exposure`,
/// then tonemaps by the brightest channel rather than per channel, so a
/// bright saturated colour keeps its hue instead of drifting toward its
/// strongest primary and washing to white; only far past the knee does it
/// fade toward white, as film does. Gamma-encodes and adds triangular
/// dither of about one 8-bit step, which breaks up banding in the dark,
/// slow gradients the memory field leaves behind.
/// `dither_seed` changes per frame so the dither pattern does not sit still.
pub fn render_composite_hdr(
    ctx: &GpuContext,
    output: &OffscreenTarget,
    scene: &OffscreenTarget,
    bloom: &OffscreenTarget,
    sampler: &wgpu::Sampler,
    bloom_strength_exposure: [f32; 2],
    dither_seed: u32,
) -> Result<(), GpuError> {
    let common = CommonUniforms::new(0.0, [output.width() as f32, output.height() as f32]);
    let [bloom_strength, exposure] = bloom_strength_exposure;
    let slots = [[bloom_strength, exposure, (dither_seed % 1024) as f32, 0.0]];
    output.render_glsl_fragment_shader(
        ctx,
        COMPOSITE_HDR_GLSL,
        common.bytes(),
        Some(&pack_slots(&slots)),
        &[(scene.view(), sampler), (bloom.view(), sampler)],
    )
}

const COMPOSITE_HDR_GLSL: &str = r#"
#version 450 core
layout(location = 0) in vec2 vUv;
layout(location = 0) out vec4 fragColor;
layout(std140, set = 0, binding = 0) uniform Common { float TIME; float TIMEDELTA; vec2 RENDERSIZE; int PASSINDEX; int FRAMEINDEX; vec4 DATE; };
layout(std140, set = 0, binding = 1) uniform Inputs { vec4 slot[1]; };
layout(set = 0, binding = 2) uniform texture2D uSceneTex;
layout(set = 0, binding = 3) uniform sampler uSceneSamp;
layout(set = 0, binding = 4) uniform texture2D uBloomTex;
layout(set = 0, binding = 5) uniform sampler uBloomSamp;

vec3 tonemapHuePreserving(vec3 c) {
  float peak = max(c.r, max(c.g, c.b));
  if (peak <= 1e-6) return vec3(0.0);
  float mapped = peak / (1.0 + peak);
  vec3 toned = c * (mapped / peak);
  // How far past the knee this pixel was: 0 at peak 1, toward 1 far above.
  float over = 1.0 - 1.0 / (1.0 + 0.15 * max(peak - 1.0, 0.0));
  return mix(toned, vec3(mapped), over * 0.7);
}

float hash12(vec2 p) {
  vec3 p3 = fract(vec3(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

void main() {
  float bloomStrength = slot[0].x;
  float exposure = slot[0].y;
  float seed = slot[0].z;
  vec3 color = texture(sampler2D(uSceneTex, uSceneSamp), vUv).rgb;
  color += texture(sampler2D(uBloomTex, uBloomSamp), vUv).rgb * bloomStrength;
  color = tonemapHuePreserving(max(color, vec3(0.0)) * exposure);
  color = pow(color, vec3(1.0 / 2.2));
  vec2 p = gl_FragCoord.xy + seed * vec2(17.0, 59.0);
  float dither = (hash12(p) + hash12(p + 0.5 * vec2(113.0, 71.0)) - 1.0) / 255.0;
  fragColor = vec4(clamp(color + dither, 0.0, 1.0), 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_bloom_and_tonemaps() {
        let ctx = match GpuContext::new_blocking() {
            Ok(c) => c,
            Err(_) => return,
        };
        let sampler = ctx.create_clamp_sampler();
        let scene = OffscreenTarget::new(&ctx, 64, 64);
        scene.render_glsl_fragment_shader(&ctx, "#version 450 core\nlayout(location=0) out vec4 fragColor;\nvoid main(){fragColor=vec4(0.1,0.1,0.1,1.0);}", &[0u8; 32], None, &[]).unwrap();
        let bloom = OffscreenTarget::new(&ctx, 64, 64);
        bloom.render_glsl_fragment_shader(&ctx, "#version 450 core\nlayout(location=0) out vec4 fragColor;\nvoid main(){fragColor=vec4(1.0,1.0,1.0,1.0);}", &[0u8; 32], None, &[]).unwrap();

        let no_bloom = OffscreenTarget::new(&ctx, 64, 64);
        render_composite(&ctx, &no_bloom, &scene, None, &sampler, 1.0, 1.0).unwrap();
        let with_bloom = OffscreenTarget::new(&ctx, 64, 64);
        render_composite(&ctx, &with_bloom, &scene, Some(&bloom), &sampler, 1.0, 1.0).unwrap();

        let a = no_bloom.read_pixels(&ctx).unwrap();
        let b = with_bloom.read_pixels(&ctx).unwrap();
        assert!(
            b[0] > a[0],
            "bloom-composited output should be brighter than without, {} vs {}",
            b[0],
            a[0]
        );
    }
}
