//! One real frame loop: Julia substrate -> memory field ping-pong ->
//! persistence/decay -> bloom -> composite -> beam -> final
//! `OffscreenTarget`. R2's own done-when bar (plan §4): "Julia substrate +
//! memory field + beam render". **Not claimed**: a pixel-perfect diff
//! against the frozen browser build — that needs a browser, out of reach
//! here; flagged, not faked.
//!
//! Two looks share the loop ([`Look`]). `Before` is the original: 8-bit
//! targets, perturbation Julia at every depth, single-level bloom,
//! per-channel Reinhard, and the beam alpha-blended onto the tonemapped
//! output. `After` keeps the field, persistence and bloom in `Rgba16Float`,
//! draws the beam additively into the memory field before tonemapping (as
//! the TS original's `renderForeground` does, so it trails and blooms),
//! uses a small mip-chain bloom, a hue-preserving tonemap with dither, a
//! tiling fbm flow noise under a repeat sampler, and direct Julia
//! iteration above `DIRECT_ZOOM_MIN`.

use crate::gpu::{GpuContext, GpuError, OffscreenTarget};
use crate::passes::beam::{render_beam, render_beam_additive, Segment};
use crate::passes::bloom::{render_blur, render_bright_pass, BloomChain};
use crate::passes::composite::{render_composite, render_composite_hdr};
use crate::passes::julia::{render_julia, render_julia_perturbed, JuliaNavState, DIRECT_ZOOM_MIN};
use crate::passes::memory_field::{cur_gain, render_memory_field, MemoryFieldParams};
use crate::passes::noise_texture::NoiseTexture;

/// Which version of the pipeline a [`Renderer`] runs; see the module doc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Before,
    After,
}

/// Per-frame knobs a caller (a driven test, or eventually `hyst-audio`
/// signals) sets; everything else about the pipeline's shape is fixed.
pub struct FrameParams {
    pub julia: JuliaNavState,
    pub memory_field: MemoryFieldParams,
    pub persistence_decay: f32,
    pub bloom_threshold: f32,
    pub bloom_strength: f32,
    /// Pre-tonemap scale (composite.rs). The memory field's `cur_gain`
    /// formula normalizes its own steady-state gain to a fixed ~7x
    /// (memory_field.rs's `DECAY_REFERENCE`) regardless of decay — by
    /// design, not a bug — so a long-running feedback loop needs real
    /// exposure headroom here or it saturates Reinhard's knee into a
    /// washed-out near-white smudge. 1.0 = no adjustment.
    pub exposure: f32,
    pub beam_segments: Vec<Segment>,
    pub beam_color: [f32; 3],
    /// In `After`, scaled by the field's `cur_gain` like the Julia input,
    /// so a beam held still settles at the same ~7x whatever the decay.
    pub beam_intensity: f32,
}

/// Owns every buffer the frame loop needs across calls (the memory field's
/// ping-pong pair persists between `render_frame` calls — that's the whole
/// point of a feedback field).
pub struct Renderer {
    look: Look,
    width: u32,
    height: u32,
    aspect: f32,
    sampler: wgpu::Sampler,
    noise_sampler: wgpu::Sampler,
    noise: NoiseTexture,
    julia_out: OffscreenTarget,
    field_a: OffscreenTarget,
    field_b: OffscreenTarget,
    field_is_a: bool,
    persistence_prev: OffscreenTarget,
    bright: OffscreenTarget,
    blur_h: OffscreenTarget,
    blur_v: OffscreenTarget,
    bloom_chain: Option<BloomChain>,
    composited: OffscreenTarget,
    final_out: OffscreenTarget,
    frame_index: u32,
}

impl Renderer {
    /// The `Before` look. `width`/`height` must keep `width * 4` a multiple
    /// of 256 (wgpu readback alignment — see `OffscreenTarget::read_pixels`);
    /// any multiple of 64 works.
    pub fn new(ctx: &GpuContext, width: u32, height: u32) -> Self {
        Self::with_look(ctx, width, height, Look::Before)
    }

    pub fn with_look(ctx: &GpuContext, width: u32, height: u32, look: Look) -> Self {
        let (field, noise, noise_sampler) = match look {
            Look::Before => (
                OffscreenTarget::new as fn(&GpuContext, u32, u32) -> OffscreenTarget,
                NoiseTexture::bake(ctx, 64, 64),
                ctx.create_clamp_sampler(),
            ),
            Look::After => (
                OffscreenTarget::new_hdr as fn(&GpuContext, u32, u32) -> OffscreenTarget,
                NoiseTexture::bake_fbm(ctx, 128),
                ctx.create_repeat_sampler(),
            ),
        };
        Self {
            look,
            width,
            height,
            aspect: width as f32 / height as f32,
            sampler: ctx.create_clamp_sampler(),
            noise_sampler,
            noise,
            julia_out: OffscreenTarget::new(ctx, width, height),
            field_a: field(ctx, width, height),
            field_b: field(ctx, width, height),
            field_is_a: true,
            persistence_prev: field(ctx, width, height),
            bright: field(ctx, width, height),
            blur_h: field(ctx, width, height),
            blur_v: field(ctx, width, height),
            bloom_chain: (look == Look::After).then(|| BloomChain::new(ctx, width, height)),
            composited: field(ctx, width, height),
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

    /// Renders one frame end to end, returns the final RGBA8 pixels
    /// (`read_pixels` on the internal final target).
    pub fn render_frame(
        &mut self,
        ctx: &GpuContext,
        params: &FrameParams,
    ) -> Result<Vec<u8>, GpuError> {
        self.frame_index = self.frame_index.wrapping_add(1);
        // Before: always the perturbation path. It is an exact algebraic
        // reformulation of the same recurrence, but in float32 its escape
        // test `Z_n + dz_n` speckles at shallow zoom, so After iterates
        // directly wherever float32 still resolves the view.
        if self.look == Look::After && params.julia.zoom >= DIRECT_ZOOM_MIN {
            render_julia(ctx, &self.julia_out, &params.julia, self.aspect)?;
        } else {
            render_julia_perturbed(ctx, &self.julia_out, &params.julia, self.aspect)?;
        }

        let (field_prev, field_dst) = if self.field_is_a {
            (&self.field_a, &self.field_b)
        } else {
            (&self.field_b, &self.field_a)
        };
        render_memory_field(
            ctx,
            field_dst,
            &self.julia_out,
            field_prev,
            &self.noise,
            &self.sampler,
            &self.noise_sampler,
            &params.memory_field,
        )?;
        const HALF_WIDTH: f32 = 0.012;
        if self.look == Look::After && !params.beam_segments.is_empty() {
            render_beam_additive(
                ctx,
                field_dst,
                &params.beam_segments,
                params.beam_color,
                params.beam_intensity * cur_gain(params.memory_field.decay),
                self.aspect,
                HALF_WIDTH,
            )?;
        }
        self.field_is_a = !self.field_is_a;
        let field_out = if self.field_is_a {
            &self.field_a
        } else {
            &self.field_b
        };

        crate::passes::persistence::render_persistence(
            ctx,
            &self.composited,
            field_out,
            &self.persistence_prev,
            &self.sampler,
            params.persistence_decay,
        )?;
        copy_texture(ctx, &self.composited, &self.persistence_prev);

        if let Some(chain) = &self.bloom_chain {
            let bloom =
                chain.render(ctx, &self.composited, &self.sampler, params.bloom_threshold)?;
            render_composite_hdr(
                ctx,
                &self.final_out,
                &self.composited,
                bloom,
                &self.sampler,
                [params.bloom_strength, params.exposure],
                self.frame_index,
            )?;
            return self.final_out.read_pixels(ctx);
        }

        render_bright_pass(
            ctx,
            &self.bright,
            &self.composited,
            &self.sampler,
            params.bloom_threshold,
        )?;
        render_blur(ctx, &self.blur_h, &self.bright, &self.sampler, [1.0, 0.0])?;
        render_blur(ctx, &self.blur_v, &self.blur_h, &self.sampler, [0.0, 1.0])?;

        render_composite(
            ctx,
            &self.final_out,
            &self.composited,
            Some(&self.blur_v),
            &self.sampler,
            params.bloom_strength,
            params.exposure,
        )?;

        if !params.beam_segments.is_empty() {
            // Tried round joint discs at every vertex (fixes flat-cap
            // notches at sharp corners) — for a dense, near-straight
            // polyline this read as a visible bead/pearl chain no matter
            // how the falloff/blending was tuned (reported: "balls
            // connected together"), because a disc's silhouette always
            // bulges past a short rectangle's sides regardless of shading.
            // Reverted: segments only, matching how this looked before
            // that change. At a small enough bend angle between
            // consecutive segments (i.e. enough points) and a thin enough
            // line, a flat-cap notch is genuinely imperceptible — that's
            // the actual fix for "invisible vertices/bends", not joint
            // geometry. See `render_beam_joints` in beam.rs — kept, tested,
            // just not called here; a real primitive for sharp corners
            // elsewhere, wrong tool for this smooth curve.
            render_beam(
                ctx,
                &self.final_out,
                &params.beam_segments,
                params.beam_color,
                params.beam_intensity,
                self.aspect,
                HALF_WIDTH,
            )?;
        }

        self.final_out.read_pixels(ctx)
    }
}

/// `persistence.frag.glsl`'s own feedback needs last frame's *composited*
/// output as `uPrev` next frame — a plain GPU-side texture-to-texture copy,
/// no shader needed.
fn copy_texture(ctx: &GpuContext, src: &OffscreenTarget, dst: &OffscreenTarget) {
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_texture(
        wgpu::ImageCopyTexture {
            texture: src.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyTexture {
            texture: dst.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: src.width(),
            height: src.height(),
            depth_or_array_layers: 1,
        },
    );
    ctx.queue.submit(std::iter::once(encoder.finish()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passes::beam::Segment;

    fn default_params(t: f32) -> FrameParams {
        FrameParams {
            julia: JuliaNavState::driven_by_time(t),
            memory_field: MemoryFieldParams {
                decay: 0.9,
                aspect: 1.0,
                flow_uv: [t * 0.05, 0.0],
                flow_scale: 2.0,
                flow_strength: 0.01,
                fold_count: 4.0,
                mirror_strength: 0.5,
            },
            persistence_decay: 0.8,
            bloom_threshold: 0.3,
            bloom_strength: 0.6,
            exposure: 1.0,
            beam_segments: vec![Segment {
                p0: [-0.5, 0.0],
                p1: [0.5, 0.0],
            }],
            beam_color: [1.0, 1.0, 1.0],
            beam_intensity: 1.5,
        }
    }

    /// Bites: a full multi-frame run through every pass produces real,
    /// changing, non-degenerate (not all-black, not all-saturated) output —
    /// the actual "driveable render loop" bar, not just "no panic."
    #[test]
    fn full_frame_loop_produces_changing_non_degenerate_output() {
        let ctx = match GpuContext::new_blocking() {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut renderer = Renderer::new(&ctx, 128, 128);
        let frame0 = renderer.render_frame(&ctx, &default_params(0.0)).unwrap();
        let frame10 = renderer.render_frame(&ctx, &default_params(1.0)).unwrap();

        let mean = |p: &[u8]| p.iter().map(|&b| b as f64).sum::<f64>() / p.len() as f64;
        let m0 = mean(&frame0);
        let m10 = mean(&frame10);
        assert!(
            m0 > 1.0 && m0 < 250.0,
            "frame should be non-degenerate, mean {m0}"
        );
        assert!(
            (m0 - m10).abs() > 0.1 || frame0 != frame10,
            "consecutive frames of a live feedback loop should differ"
        );
    }

    /// Same bar for the `After` look: HDR targets, mip bloom, the beam drawn
    /// into the field and the hue-preserving tonemap still give a real,
    /// changing frame at field-unit exposure.
    #[test]
    fn after_look_frame_loop_produces_changing_non_degenerate_output() {
        let ctx = match GpuContext::new_blocking() {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut renderer = Renderer::with_look(&ctx, 128, 128, Look::After);
        let after_params = |t: f32| FrameParams {
            exposure: 0.25,
            bloom_threshold: 4.0,
            ..default_params(t)
        };
        let mut frames = Vec::new();
        for i in 0..12 {
            frames.push(
                renderer
                    .render_frame(&ctx, &after_params(i as f32 / 10.0))
                    .unwrap(),
            );
        }
        let mean = |p: &[u8]| p.iter().map(|&b| b as f64).sum::<f64>() / p.len() as f64;
        let m_last = mean(&frames[11]);
        assert!(
            m_last > 1.0 && m_last < 250.0,
            "frame should be non-degenerate, mean {m_last}"
        );
        assert_ne!(frames[10], frames[11], "consecutive frames should differ");
    }
}
