//! The one Bevy `Material` every shader node renders with.
//!
//! The bind group layout is static (a 256-byte parameter blob, a frame
//! uniform, `MAX_INPUTS` texture/sampler pairs and the feedback tap); the
//! fragment shader is chosen per node through the pipeline key. Node WGSL is
//! generated from a `NodeDef` (see `NodeDef::wgsl_source`), so a node never
//! declares bindings itself.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::Shader;
use zygote_core::UNIFORM_BYTES;

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

/// Textures bound to inputs that have nothing connected.
#[derive(Resource, Default)]
pub struct Fallbacks {
    pub black: Handle<Image>,
    /// Magenta/black checker for image assets that do not exist or failed to load.
    pub missing: Handle<Image>,
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

        let (w, h) = (64u32, 64u32);
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let on = ((x / 8) + (y / 8)).is_multiple_of(2);
                let (r, g, b) = if on { (255, 0, 255) } else { (0, 0, 0) };
                data.extend_from_slice(&[r, g, b, 255]);
            }
        }
        let mut missing = Image::new(
            Extent3d {
                width: w,
                height: h,
                ..Default::default()
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        missing.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Nearest,
            min_filter: ImageFilterMode::Nearest,
            ..Default::default()
        });
        fallbacks.missing = images.add(missing);
    }
}

/// Raw parameter bytes laid out by `NodeDef::write_uniform`.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct ParamsBlob {
    pub words: [Vec4; UNIFORM_BYTES / 16],
}

impl Default for ParamsBlob {
    fn default() -> Self {
        Self {
            words: [Vec4::ZERO; UNIFORM_BYTES / 16],
        }
    }
}

impl ParamsBlob {
    pub fn from_bytes(bytes: &[u8; UNIFORM_BYTES]) -> Self {
        let mut words = [Vec4::ZERO; UNIFORM_BYTES / 16];
        for (i, word) in words.iter_mut().enumerate() {
            let at = i * 16;
            let f = |o: usize| f32::from_le_bytes(bytes[at + o..at + o + 4].try_into().unwrap());
            *word = Vec4::new(f(0), f(4), f(8), f(12));
        }
        Self { words }
    }
}

/// Per-frame data every node shader can read as `frame`.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameUniform {
    pub time: f32,
    pub dt: f32,
    pub aspect: f32,
    pub index: f32,
    /// Bit `i` set when input slot `i` is connected.
    pub connected: u32,
    pub _r0: u32,
    pub _r1: u32,
    pub _r2: u32,
}

/// Pipeline key: which fragment shader this node runs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeKey {
    pub shader: Handle<Shader>,
}

impl From<&NodeMaterial> for NodeKey {
    fn from(material: &NodeMaterial) -> Self {
        Self {
            shader: material.shader.clone(),
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(NodeKey)]
pub struct NodeMaterial {
    #[uniform(0)]
    pub params: ParamsBlob,
    #[uniform(1)]
    pub frame: FrameUniform,
    #[texture(2)]
    #[sampler(3)]
    pub in0: Handle<Image>,
    #[texture(4)]
    #[sampler(5)]
    pub in1: Handle<Image>,
    #[texture(6)]
    #[sampler(7)]
    pub in2: Handle<Image>,
    #[texture(8)]
    #[sampler(9)]
    pub in3: Handle<Image>,
    /// Last frame's own output (feedback nodes), else the black fallback.
    #[texture(10)]
    #[sampler(11)]
    pub previous: Handle<Image>,
    /// Generated fragment shader for this node kind.
    pub shader: Handle<Shader>,
}

impl NodeMaterial {
    pub fn new(shader: Handle<Shader>, fallback: &Handle<Image>) -> Self {
        Self {
            params: ParamsBlob::default(),
            frame: FrameUniform::default(),
            in0: fallback.clone(),
            in1: fallback.clone(),
            in2: fallback.clone(),
            in3: fallback.clone(),
            previous: fallback.clone(),
            shader,
        }
    }

    pub fn input_mut(&mut self, slot: usize) -> Option<&mut Handle<Image>> {
        match slot {
            0 => Some(&mut self.in0),
            1 => Some(&mut self.in1),
            2 => Some(&mut self.in2),
            3 => Some(&mut self.in3),
            _ => None,
        }
    }

    pub fn input(&self, slot: usize) -> Option<&Handle<Image>> {
        match slot {
            0 => Some(&self.in0),
            1 => Some(&self.in1),
            2 => Some(&self.in2),
            3 => Some(&self.in3),
            _ => None,
        }
    }
}

impl Material for NodeMaterial {
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = key.bind_group_data.shader.clone();
        }
        Ok(())
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }
}
