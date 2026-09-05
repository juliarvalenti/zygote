//! Runtime instantiation of the node graph.
//!
//! Every processing node becomes: an offscreen `Camera3d` on its own render
//! layer, a fullscreen quad carrying the node's material, and one (or, for
//! feedback, two ping-ponged) render-target `Image`s. Cameras are ordered by
//! the graph's topological order so each frame flows source → output.

use std::collections::HashMap;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Hdr, OrthographicProjection, Projection, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::system::SystemParam;
use bevy::image::{ImageLoaderSettings, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::Msaa;
use zygote_core::{NodeId, NodeKind, ParamPath};

use crate::materials::{
    BlendMaterial, BlendParams, ColorGradeMaterial, ColorGradeParams, Fallbacks, FeedbackMaterial,
    FeedbackParams, GeneratorMaterial, GeneratorParams, WarpMaterial, WarpParams, node_sampler,
};
use crate::params::ParamState;
use crate::plugin::{GraphRes, NodeResolution};

/// First render layer used by node passes. Layer 0 belongs to the window.
const FIRST_NODE_LAYER: usize = 8;
/// Camera order of the first node pass; the window camera is at 0.
const FIRST_NODE_ORDER: isize = -1000;

pub enum MaterialHandle {
    Generator(Handle<GeneratorMaterial>),
    Warp(Handle<WarpMaterial>),
    Blend(Handle<BlendMaterial>),
    Feedback(Handle<FeedbackMaterial>),
    ColorGrade(Handle<ColorGradeMaterial>),
}

pub struct Pass {
    pub camera: Entity,
    pub quad: Entity,
    pub material: MaterialHandle,
}

pub enum Output {
    /// Fixed texture (loaded asset or single render target).
    Single(Handle<Image>),
    /// Two render targets, alternating every frame. `write` is the one being
    /// rendered this frame and is what downstream nodes read.
    PingPong {
        images: [Handle<Image>; 2],
        write: usize,
    },
}

pub struct NodeRuntime {
    pub id: NodeId,
    pub kind: NodeKind,
    pub enabled: bool,
    pub inputs: Vec<NodeId>,
    pub output: Output,
    pub pass: Option<Pass>,
}

impl NodeRuntime {
    /// Texture downstream consumers should sample this frame.
    pub fn current_output(&self) -> &Handle<Image> {
        match &self.output {
            Output::Single(h) => h,
            Output::PingPong { images, write } => &images[*write],
        }
    }

    /// Last frame's texture (feedback only).
    pub fn previous_output(&self) -> Option<&Handle<Image>> {
        match &self.output {
            Output::PingPong { images, write } => Some(&images[1 - *write]),
            Output::Single(_) => None,
        }
    }
}

/// All instantiated nodes plus lookup tables. Addressable at runtime: the
/// timeline (or anything else) can query nodes by id and inspect what they
/// render to.
#[derive(Resource, Default)]
pub struct Runtime {
    /// Nodes in topological order.
    pub nodes: Vec<NodeRuntime>,
    pub index: HashMap<NodeId, usize>,
    pub output: Option<NodeId>,
    pub frame: u64,
}

impl Runtime {
    pub fn node(&self, id: &NodeId) -> Option<&NodeRuntime> {
        self.index.get(id).map(|&i| &self.nodes[i])
    }

    /// One line per node, for logs and inspectors.
    pub fn describe(&self) -> String {
        self.nodes
            .iter()
            .map(|n| {
                let pass = match &n.pass {
                    Some(p) => format!("camera {:?} quad {:?}", p.camera, p.quad),
                    None => "no pass".to_owned(),
                };
                let inputs = n
                    .inputs
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{} [{}]{} inputs: [{inputs}] {pass}",
                    n.id,
                    n.kind.label(),
                    if n.enabled { "" } else { " (bypassed)" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Effective output of a node, following bypasses of disabled nodes.
    pub fn output_of(&self, id: &NodeId) -> Option<Handle<Image>> {
        let mut current = id;
        for _ in 0..self.nodes.len().max(1) {
            let node = self.node(current)?;
            if node.enabled || node.inputs.is_empty() {
                return Some(node.current_output().clone());
            }
            current = &node.inputs[0];
        }
        None
    }

    /// Texture shown on the display quad.
    pub fn output_handle(&self) -> Handle<Image> {
        self.output
            .as_ref()
            .and_then(|id| self.output_of(id))
            .unwrap_or_default()
    }
}

#[derive(SystemParam)]
pub struct NodeMaterials<'w> {
    pub generator: ResMut<'w, Assets<GeneratorMaterial>>,
    pub warp: ResMut<'w, Assets<WarpMaterial>>,
    pub blend: ResMut<'w, Assets<BlendMaterial>>,
    pub feedback: ResMut<'w, Assets<FeedbackMaterial>>,
    pub color_grade: ResMut<'w, Assets<ColorGradeMaterial>>,
}

fn target_image(resolution: UVec2) -> Image {
    let mut image =
        Image::new_target_texture(resolution.x, resolution.y, TextureFormat::Rgba16Float, None);
    image.sampler = node_sampler();
    image
}

/// Instantiate the graph from the `GraphRes` resource.
// Bevy systems declare their world access through parameters; splitting this
// one up would only hide that.
#[allow(clippy::too_many_arguments)]
pub fn build_runtime(
    mut commands: Commands,
    graph: Res<GraphRes>,
    resolution: Res<NodeResolution>,
    fallbacks: Res<Fallbacks>,
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: NodeMaterials,
) {
    let order = match graph.topo_order() {
        Ok(order) => order,
        Err(e) => {
            error!("graph is invalid: {e}");
            commands.insert_resource(Runtime::default());
            return;
        }
    };

    let quad_mesh = meshes.add(Rectangle::new(2.0, 2.0));
    let aspect = resolution.aspect();
    let mut runtime = Runtime {
        output: Some(graph.output.clone()),
        ..Default::default()
    };

    for (i, id) in order.iter().enumerate() {
        let spec = graph.node(id).expect("topo order only yields known nodes");
        let layer = RenderLayers::layer(FIRST_NODE_LAYER + i);
        let camera_order = FIRST_NODE_ORDER + i as isize;

        let (output, material) = match &spec.kind {
            NodeKind::Image { path } => {
                let handle: Handle<Image> = asset_server
                    .load_builder()
                    .with_settings(|settings: &mut ImageLoaderSettings| {
                        settings.sampler = node_sampler()
                    })
                    .load(path.clone());
                (Output::Single(handle), None)
            }
            NodeKind::Camera { device } => {
                warn!(
                    "node `{id}`: live camera input (device {device}) has no capture backend in this build; showing placeholder"
                );
                (Output::Single(fallbacks.camera_placeholder.clone()), None)
            }
            NodeKind::Solid | NodeKind::TestPattern | NodeKind::Noise => {
                let kind = match spec.kind {
                    NodeKind::Solid => 0.0,
                    NodeKind::TestPattern => 1.0,
                    _ => 2.0,
                };
                let handle = materials.generator.add(GeneratorMaterial {
                    params: GeneratorParams {
                        p0: Vec4::new(kind, 0.0, 1.0, 0.0),
                        p1: Vec4::new(1.0, 1.0, 1.0, 4.0),
                        p2: Vec4::new(1.0, aspect, 0.0, 0.0),
                    },
                });
                (
                    Output::Single(images.add(target_image(resolution.0))),
                    Some(MaterialHandle::Generator(handle)),
                )
            }
            NodeKind::Warp => {
                let handle = materials.warp.add(WarpMaterial {
                    params: WarpParams {
                        p0: Vec4::ZERO,
                        p1: Vec4::new(0.0, 0.0, aspect, 0.0),
                    },
                    source: fallbacks.black.clone(),
                    displacement: fallbacks.black.clone(),
                });
                (
                    Output::Single(images.add(target_image(resolution.0))),
                    Some(MaterialHandle::Warp(handle)),
                )
            }
            NodeKind::Blend { mode } => {
                let handle = materials.blend.add(BlendMaterial {
                    params: BlendParams {
                        p0: Vec4::new(mode.to_param(), 1.0, 0.0, 0.0),
                    },
                    a: fallbacks.black.clone(),
                    b: fallbacks.black.clone(),
                });
                (
                    Output::Single(images.add(target_image(resolution.0))),
                    Some(MaterialHandle::Blend(handle)),
                )
            }
            NodeKind::Feedback => {
                let a = images.add(target_image(resolution.0));
                let b = images.add(target_image(resolution.0));
                let handle = materials.feedback.add(FeedbackMaterial {
                    params: FeedbackParams {
                        p0: Vec4::new(0.9, 1.0, 0.0, 0.0),
                        p1: Vec4::new(1.0, 0.0, aspect, 0.0),
                    },
                    source: fallbacks.black.clone(),
                    previous: b.clone(),
                });
                (
                    Output::PingPong {
                        images: [a, b],
                        write: 0,
                    },
                    Some(MaterialHandle::Feedback(handle)),
                )
            }
            NodeKind::ColorGrade { lut } => {
                let (lut_handle, use_lut) = match lut {
                    Some(path) => (
                        asset_server
                            .load_builder()
                            .with_settings(|settings: &mut ImageLoaderSettings| {
                                settings.sampler = ImageSampler::linear()
                            })
                            .load(path.clone()),
                        1.0,
                    ),
                    None => (fallbacks.identity_lut.clone(), 0.0),
                };
                let handle = materials.color_grade.add(ColorGradeMaterial {
                    params: ColorGradeParams {
                        p0: Vec4::new(0.0, 1.0, 0.0, 0.0),
                        p1: Vec4::new(0.0, 0.0, use_lut, 0.0),
                    },
                    source: fallbacks.black.clone(),
                    lut: lut_handle,
                });
                (
                    Output::Single(images.add(target_image(resolution.0))),
                    Some(MaterialHandle::ColorGrade(handle)),
                )
            }
        };

        let pass = material.map(|material| {
            let target = match &output {
                Output::Single(h) => h.clone(),
                Output::PingPong { images, write } => images[*write].clone(),
            };
            let quad = spawn_quad(&mut commands, &quad_mesh, &material, layer.clone(), id);
            let camera = commands
                .spawn((
                    Name::new(format!("pass camera: {id}")),
                    Camera3d::default(),
                    Camera {
                        order: camera_order,
                        clear_color: ClearColorConfig::Custom(Color::BLACK),
                        ..Default::default()
                    },
                    Hdr,
                    Tonemapping::None,
                    Msaa::Off,
                    Projection::Orthographic(OrthographicProjection {
                        scaling_mode: ScalingMode::Fixed {
                            width: 2.0,
                            height: 2.0,
                        },
                        near: -1.0,
                        far: 1.0,
                        ..OrthographicProjection::default_3d()
                    }),
                    Transform::from_xyz(0.0, 0.0, 0.5).looking_at(Vec3::ZERO, Vec3::Y),
                    RenderTarget::Image(target.into()),
                    layer.clone(),
                ))
                .id();
            Pass {
                camera,
                quad,
                material,
            }
        });

        runtime.index.insert(id.clone(), runtime.nodes.len());
        runtime.nodes.push(NodeRuntime {
            id: id.clone(),
            kind: spec.kind.clone(),
            enabled: spec.enabled,
            inputs: spec.inputs.clone(),
            output,
            pass,
        });
    }

    info!(
        "built graph `{}`: {} nodes, output `{}`\n{}",
        graph.name,
        runtime.nodes.len(),
        graph.output,
        runtime.describe()
    );
    commands.insert_resource(runtime);
}

fn spawn_quad(
    commands: &mut Commands,
    mesh: &Handle<Mesh>,
    material: &MaterialHandle,
    layer: RenderLayers,
    id: &NodeId,
) -> Entity {
    let mut entity = commands.spawn((
        Name::new(format!("pass quad: {id}")),
        Mesh3d(mesh.clone()),
        Transform::IDENTITY,
        layer,
    ));
    match material {
        MaterialHandle::Generator(h) => entity.insert(MeshMaterial3d(h.clone())),
        MaterialHandle::Warp(h) => entity.insert(MeshMaterial3d(h.clone())),
        MaterialHandle::Blend(h) => entity.insert(MeshMaterial3d(h.clone())),
        MaterialHandle::Feedback(h) => entity.insert(MeshMaterial3d(h.clone())),
        MaterialHandle::ColorGrade(h) => entity.insert(MeshMaterial3d(h.clone())),
    };
    entity.id()
}

/// Advance ping-pong buffers: the target written last frame becomes `previous`.
pub fn swap_feedback(mut runtime: ResMut<Runtime>, mut cameras: Query<&mut RenderTarget>) {
    runtime.frame += 1;
    for node in runtime.nodes.iter_mut() {
        let Output::PingPong { images, write } = &mut node.output else {
            continue;
        };
        *write = 1 - *write;
        if let Some(pass) = &node.pass
            && let Ok(mut target) = cameras.get_mut(pass.camera)
        {
            *target = RenderTarget::Image(images[*write].clone().into());
        }
    }
}

/// Point every material's texture inputs at the current outputs of its
/// upstream nodes. Only writes when a handle actually changed so material
/// bind groups are not rebuilt needlessly.
pub fn rewire(runtime: Res<Runtime>, fallbacks: Res<Fallbacks>, mut materials: NodeMaterials) {
    for node in &runtime.nodes {
        let Some(pass) = &node.pass else { continue };
        let input = |slot: usize| -> Handle<Image> {
            node.inputs
                .get(slot)
                .and_then(|id| runtime.output_of(id))
                .unwrap_or_else(|| fallbacks.black.clone())
        };
        match &pass.material {
            MaterialHandle::Generator(_) => {}
            MaterialHandle::Warp(h) => {
                let source = input(0);
                let has_disp = node.inputs.len() > 1;
                let displacement = if has_disp {
                    input(1)
                } else {
                    fallbacks.black.clone()
                };
                if let Some(m) = materials.warp.get(h)
                    && (m.source != source || m.displacement != displacement)
                    && let Some(mut m) = materials.warp.get_mut(h)
                {
                    m.source = source;
                    m.displacement = displacement;
                }
            }
            MaterialHandle::Blend(h) => {
                let (a, b) = (input(0), input(1));
                if let Some(m) = materials.blend.get(h)
                    && (m.a != a || m.b != b)
                    && let Some(mut m) = materials.blend.get_mut(h)
                {
                    m.a = a;
                    m.b = b;
                }
            }
            MaterialHandle::Feedback(h) => {
                let source = input(0);
                let previous = node
                    .previous_output()
                    .cloned()
                    .unwrap_or_else(|| fallbacks.black.clone());
                if let Some(m) = materials.feedback.get(h)
                    && (m.source != source || m.previous != previous)
                    && let Some(mut m) = materials.feedback.get_mut(h)
                {
                    m.source = source;
                    m.previous = previous;
                }
            }
            MaterialHandle::ColorGrade(h) => {
                let source = input(0);
                if let Some(m) = materials.color_grade.get(h)
                    && m.source != source
                    && let Some(mut m) = materials.color_grade.get_mut(h)
                {
                    m.source = source;
                }
            }
        }
    }
}

/// Write resolved parameter values into the materials.
pub fn apply_params(
    runtime: Res<Runtime>,
    state: Res<ParamState>,
    time: Res<Time>,
    resolution: Res<NodeResolution>,
    mut materials: NodeMaterials,
) {
    let t = time.elapsed_secs();
    let aspect = resolution.aspect();
    let values = &state.resolved;
    let get = |id: &NodeId, name: &str, fallback: f32| -> f32 {
        values
            .get(&ParamPath::new(id.clone(), name))
            .copied()
            .unwrap_or(fallback)
    };

    for node in &runtime.nodes {
        let Some(pass) = &node.pass else { continue };
        let id = &node.id;
        match &pass.material {
            MaterialHandle::Generator(h) => {
                let Some(mut m) = materials.generator.get_mut(h) else {
                    continue;
                };
                let kind = m.params.p0.x;
                m.params = GeneratorParams {
                    p0: Vec4::new(kind, t, get(id, "scale", 3.0), get(id, "speed", 0.3)),
                    p1: Vec4::new(
                        get(id, "r", 1.0),
                        get(id, "g", 1.0),
                        get(id, "b", 1.0),
                        get(id, "octaves", 4.0),
                    ),
                    p2: Vec4::new(get(id, "contrast", 1.0), aspect, 0.0, 0.0),
                };
            }
            MaterialHandle::Warp(h) => {
                let Some(mut m) = materials.warp.get_mut(h) else {
                    continue;
                };
                let use_disp = if node.inputs.len() > 1 { 1.0 } else { 0.0 };
                m.params = WarpParams {
                    p0: Vec4::new(
                        get(id, "amount", 0.15),
                        get(id, "scale", 2.0),
                        get(id, "speed", 0.25),
                        get(id, "twist", 0.0),
                    ),
                    p1: Vec4::new(t, use_disp, aspect, 0.0),
                };
            }
            MaterialHandle::Blend(h) => {
                let Some(mut m) = materials.blend.get_mut(h) else {
                    continue;
                };
                let default_mode = m.params.p0.x;
                m.params = BlendParams {
                    p0: Vec4::new(get(id, "mode", default_mode), get(id, "mix", 1.0), 0.0, 0.0),
                };
            }
            MaterialHandle::Feedback(h) => {
                let Some(mut m) = materials.feedback.get_mut(h) else {
                    continue;
                };
                m.params = FeedbackParams {
                    p0: Vec4::new(
                        get(id, "decay", 0.92),
                        get(id, "zoom", 1.01),
                        get(id, "rotate", 0.0),
                        get(id, "hue_shift", 0.0),
                    ),
                    p1: Vec4::new(get(id, "mix", 1.0), t, aspect, 0.0),
                };
            }
            MaterialHandle::ColorGrade(h) => {
                let Some(mut m) = materials.color_grade.get_mut(h) else {
                    continue;
                };
                let use_lut = m.params.p1.z;
                m.params = ColorGradeParams {
                    p0: Vec4::new(
                        get(id, "hue", 0.0),
                        get(id, "saturation", 1.0),
                        get(id, "posterize", 0.0),
                        get(id, "palette", 0.0),
                    ),
                    p1: Vec4::new(
                        get(id, "palette_mix", 0.0),
                        get(id, "lut_mix", 0.0),
                        use_lut,
                        0.0,
                    ),
                };
            }
        }
    }
}
