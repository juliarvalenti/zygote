//! Bevy `Material`s for each processing node. Every material renders a
//! fullscreen quad into the node's output texture; parameters travel as
//! `vec4` lanes so the uniform layout is trivially std140-compatible.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::Material;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;

const GENERATOR_SHADER: &str = "embedded://zygote_render/shaders/generator.wgsl";
const WARP_SHADER: &str = "embedded://zygote_render/shaders/warp.wgsl";
const BLEND_SHADER: &str = "embedded://zygote_render/shaders/blend.wgsl";
const FEEDBACK_SHADER: &str = "embedded://zygote_render/shaders/feedback.wgsl";
const COLOR_GRADE_SHADER: &str = "embedded://zygote_render/shaders/color_grade.wgsl";

/// Sampler used for every node output: linear, mirrored so warps and
/// feedback zooms never show hard edges.
pub fn node_sampler() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::MirrorRepeat,
        address_mode_v: ImageAddressMode::MirrorRepeat,
        address_mode_w: ImageAddressMode::MirrorRepeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..Default::default()
    })
}

/// Textures bound to inputs that have nothing connected yet.
#[derive(Resource, Default)]
pub struct Fallbacks {
    pub black: Handle<Image>,
    /// 256x1 identity LUT (grey ramp).
    pub identity_lut: Handle<Image>,
    /// Placeholder shown by `Camera` nodes until a capture backend writes frames.
    pub camera_placeholder: Handle<Image>,
}

impl Fallbacks {
    pub fn init(mut fallbacks: ResMut<Fallbacks>, mut images: ResMut<Assets<Image>>) {
        let mut black = Image::new_fill(
            Extent3d {
                width: 1,
                height: 1,
                ..Default::default()
            },
            TextureDimension::D2,
            &[0, 0, 0, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        black.sampler = node_sampler();
        fallbacks.black = images.add(black);

        let mut ramp = Vec::with_capacity(256 * 4);
        for i in 0..256u32 {
            ramp.extend_from_slice(&[i as u8, i as u8, i as u8, 255]);
        }
        let mut lut = Image::new(
            Extent3d {
                width: 256,
                height: 1,
                ..Default::default()
            },
            TextureDimension::D2,
            ramp,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        lut.sampler = ImageSampler::linear();
        fallbacks.identity_lut = images.add(lut);

        // Diagonal stripes: obviously synthetic, so a missing camera is never
        // mistaken for a black feed.
        let (w, h) = (256u32, 144u32);
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let on = ((x + y) / 16) % 2 == 0;
                let (r, g, b) = if on { (40, 40, 48) } else { (120, 30, 140) };
                data.extend_from_slice(&[r, g, b, 255]);
            }
        }
        let mut placeholder = Image::new(
            Extent3d {
                width: w,
                height: h,
                ..Default::default()
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        placeholder.sampler = node_sampler();
        fallbacks.camera_placeholder = images.add(placeholder);
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct GeneratorParams {
    /// kind (0 solid, 1 test pattern, 2 noise), time, scale, speed
    pub p0: Vec4,
    /// r, g, b, octaves
    pub p1: Vec4,
    /// contrast, aspect, -, -
    pub p2: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct GeneratorMaterial {
    #[uniform(0)]
    pub params: GeneratorParams,
}

impl Material for GeneratorMaterial {
    fn fragment_shader() -> ShaderRef {
        GENERATOR_SHADER.into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct WarpParams {
    /// amount, scale, speed, twist
    pub p0: Vec4,
    /// time, use_displacement, aspect, -
    pub p1: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct WarpMaterial {
    #[uniform(0)]
    pub params: WarpParams,
    #[texture(1)]
    #[sampler(2)]
    pub source: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub displacement: Handle<Image>,
}

impl Material for WarpMaterial {
    fn fragment_shader() -> ShaderRef {
        WARP_SHADER.into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct BlendParams {
    /// mode, mix, -, -
    pub p0: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct BlendMaterial {
    #[uniform(0)]
    pub params: BlendParams,
    #[texture(1)]
    #[sampler(2)]
    pub a: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub b: Handle<Image>,
}

impl Material for BlendMaterial {
    fn fragment_shader() -> ShaderRef {
        BLEND_SHADER.into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct FeedbackParams {
    /// decay, zoom, rotate, hue_shift
    pub p0: Vec4,
    /// mix, time, aspect, -
    pub p1: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct FeedbackMaterial {
    #[uniform(0)]
    pub params: FeedbackParams,
    #[texture(1)]
    #[sampler(2)]
    pub source: Handle<Image>,
    /// Last frame's output of this node (the other half of the ping-pong pair).
    #[texture(3)]
    #[sampler(4)]
    pub previous: Handle<Image>,
}

impl Material for FeedbackMaterial {
    fn fragment_shader() -> ShaderRef {
        FEEDBACK_SHADER.into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct ColorGradeParams {
    /// hue, saturation, posterize, palette
    pub p0: Vec4,
    /// palette_mix, lut_mix, use_lut, -
    pub p1: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct ColorGradeMaterial {
    #[uniform(0)]
    pub params: ColorGradeParams,
    #[texture(1)]
    #[sampler(2)]
    pub source: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub lut: Handle<Image>,
}

impl Material for ColorGradeMaterial {
    fn fragment_shader() -> ShaderRef {
        COLOR_GRADE_SHADER.into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}
