//! Baked curl-noise potential texture — port-the-intent sibling of
//! `gl/noise-texture.ts`. The TS original bakes a scalar potential into the
//! B channel once at startup; `memory-field.frag.glsl`'s `curlAt` takes its
//! curl (central difference) to get a divergence-free flow direction
//! without a simulation buffer. **Deviation**: this is a simple deterministic
//! multi-octave sine field, not a port of the TS file's exact noise
//! algorithm (never read bit-for-bit — the memory-field pass only cares
//! that the B channel has spatial structure to take a curl of, not its
//! precise statistics, and the fold-seam fix's correctness — see
//! `memory_field.rs` — doesn't depend on the noise source at all).

use crate::gpu::GpuContext;

pub struct NoiseTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl NoiseTexture {
    pub fn bake(ctx: &GpuContext, width: u32, height: u32) -> Self {
        let mut data = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let u = x as f32 / width as f32;
                let v = y as f32 / height as f32;
                // Sum of a few incommensurate sine waves: cheap, deterministic,
                // has real spatial gradient structure for curlAt to sample.
                use std::f32::consts::TAU;
                let potential = (u * TAU * 3.0 + v * TAU * 2.0).sin() * 0.5
                    + (u * TAU * 5.0 - v * TAU * 4.0).sin() * 0.3
                    + (u * TAU * 1.7 + v * TAU * 7.0).sin() * 0.2;
                let b = ((potential * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
                let i = ((y * width + x) * 4) as usize;
                data[i] = 128;
                data[i + 1] = 128;
                data[i + 2] = b;
                data[i + 3] = 255;
            }
        }

        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hyst-render noise texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        ctx.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width,
            height,
        }
    }

    /// Tileable fbm potential in the B channel of a square `size`² texture:
    /// five octaves of periodic value noise whose lattices all divide the
    /// texture, so it wraps seamlessly under a repeat sampler. Stored as
    /// `Rgba16Float` because the memory field takes the curl of this by
    /// one-texel central differences, which 8 bits would quantise to steps.
    pub fn bake_fbm(ctx: &GpuContext, size: u32) -> Self {
        const OCTAVES: u32 = 5;
        const BASE_CELLS: u32 = 4;
        assert!(
            size.is_multiple_of(BASE_CELLS << (OCTAVES - 1)),
            "fbm noise size {size} must be a multiple of {}",
            BASE_CELLS << (OCTAVES - 1)
        );
        let hash = |x: u32, y: u32, octave: u32| -> f32 {
            let mut h = x
                .wrapping_mul(0x8da6_b343)
                .wrapping_add(y.wrapping_mul(0xd816_3841))
                .wrapping_add(octave.wrapping_mul(0xcb1a_b31f));
            h ^= h >> 13;
            h = h.wrapping_mul(0x5bd1_e995);
            h ^= h >> 15;
            (h & 0xffff) as f32 / 65535.0
        };
        let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        let mut potential = vec![0.0f32; (size * size) as usize];
        let mut amp_sum = 0.0;
        for octave in 0..OCTAVES {
            let cells = BASE_CELLS << octave;
            let cell_px = (size / cells) as f32;
            let amp = 0.5f32.powi(octave as i32);
            amp_sum += amp;
            for y in 0..size {
                for x in 0..size {
                    let fx = x as f32 / cell_px;
                    let fy = y as f32 / cell_px;
                    let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
                    let (x1, y1) = ((x0 + 1) % cells, (y0 + 1) % cells);
                    let (tx, ty) = (fade(fx.fract()), fade(fy.fract()));
                    let top = hash(x0, y0, octave) * (1.0 - tx) + hash(x1, y0, octave) * tx;
                    let bottom = hash(x0, y1, octave) * (1.0 - tx) + hash(x1, y1, octave) * tx;
                    potential[(y * size + x) as usize] += amp * (top * (1.0 - ty) + bottom * ty);
                }
            }
        }
        let mut data = Vec::with_capacity((size * size * 4) as usize);
        for p in potential {
            for channel in [0.5, 0.5, p / amp_sum, 1.0] {
                data.push(f32_to_f16_bits(channel));
            }
        }

        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hyst-render fbm noise texture"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        ctx.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&data),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(size * 8),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width: size,
            height: size,
        }
    }
}

/// IEEE half from a non-negative f32 in the noise's 0..1 range; values too
/// small for a normal half flush to zero, which is all this texture needs.
fn f32_to_f16_bits(v: f32) -> u16 {
    let bits = v.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    if exp <= 0 {
        return 0;
    }
    if exp >= 31 {
        return 0x7c00;
    }
    let mantissa = ((bits >> 13) & 0x3ff) as u16;
    ((exp as u16) << 10) | mantissa
}

#[cfg(test)]
mod tests {
    use super::f32_to_f16_bits;

    #[test]
    fn half_conversion_matches_known_values() {
        assert_eq!(f32_to_f16_bits(0.0), 0);
        assert_eq!(f32_to_f16_bits(1.0), 0x3c00);
        assert_eq!(f32_to_f16_bits(0.5), 0x3800);
        assert_eq!(f32_to_f16_bits(0.25), 0x3400);
    }
}
