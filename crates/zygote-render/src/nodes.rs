//! Runtime instantiation of the node graph.
//!
//! Every shader node becomes: an offscreen `Camera3d` on its own render
//! layer, a fullscreen quad carrying a `NodeMaterial`, and one (or, for
//! feedback nodes, two ping-ponged) render-target `Image`s. Cameras are
//! ordered by the graph's topological order so each frame flows source →
//! output. Node definitions are compiled to shaders once per kind; project
//! node files are watched and recompiled when they change.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Hdr, OrthographicProjection, Projection, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::image::ImageLoaderSettings;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::Msaa;
use bevy::shader::Shader;
use zygote_core::{
    MAX_INPUTS, NodeDef, NodeId, NodeKind, NodeOrigin, ParamPath, ParamValue, UNIFORM_BYTES,
};

use crate::materials::{Fallbacks, FrameUniform, NodeMaterial, ParamsBlob, node_sampler};
use crate::params::ParamState;
use crate::plugin::{GraphRes, LibraryRes, NodeResolution};

/// First render layer used by node passes. Layer 0 belongs to the window.
const FIRST_NODE_LAYER: usize = 8;
/// Camera order of the first node pass; the window camera is at 0.
const FIRST_NODE_ORDER: isize = -1000;
/// How often project node files are checked for changes.
const HOT_RELOAD_INTERVAL: Duration = Duration::from_millis(500);

pub struct Pass {
    pub camera: Entity,
    pub quad: Entity,
    pub material: Handle<NodeMaterial>,
    /// Name of the definition this pass was built from.
    pub def: String,
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

/// All instantiated nodes plus lookup tables, addressable by node id.
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

    /// One line per node, for logs and inspectors.
    pub fn describe(&self) -> String {
        self.nodes
            .iter()
            .map(|n| {
                let pass = match &n.pass {
                    Some(p) => format!("pass `{}` camera {:?} quad {:?}", p.def, p.camera, p.quad),
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
}

/// Compiled shader per node definition, plus file-watch state.
#[derive(Resource, Default)]
pub struct NodeShaders {
    pub handles: BTreeMap<String, Handle<Shader>>,
    watched: Vec<WatchedFile>,
    last_check: Option<SystemTime>,
}

struct WatchedFile {
    def: String,
    path: PathBuf,
    modified: Option<SystemTime>,
}

impl NodeShaders {
    /// Compile (or fetch) the shader for a definition.
    fn shader_for(&mut self, def: &NodeDef, shaders: &mut Assets<Shader>) -> Handle<Shader> {
        if let Some(handle) = self.handles.get(&def.name) {
            return handle.clone();
        }
        let handle = shaders.add(compile(def));
        self.handles.insert(def.name.clone(), handle.clone());
        if let NodeOrigin::File(path) = &def.origin {
            self.watched.push(WatchedFile {
                def: def.name.clone(),
                path: path.clone(),
                modified: mtime(path),
            });
        }
        handle
    }
}

fn compile(def: &NodeDef) -> Shader {
    Shader::from_wgsl(
        def.wgsl_source(),
        format!("zygote://nodes/{}.wgsl", def.name),
    )
}

fn mtime(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn target_image(resolution: UVec2) -> Image {
    let mut image =
        Image::new_target_texture(resolution.x, resolution.y, TextureFormat::Rgba16Float, None);
    image.sampler = node_sampler();
    image
}

/// Instantiate the graph from the `GraphRes` and `LibraryRes` resources.
#[allow(clippy::too_many_arguments)]
pub fn build_runtime(
    mut commands: Commands,
    graph: Res<GraphRes>,
    library: Res<LibraryRes>,
    resolution: Res<NodeResolution>,
    fallbacks: Res<Fallbacks>,
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<NodeMaterial>>,
    mut shaders: ResMut<Assets<Shader>>,
    mut node_shaders: ResMut<NodeShaders>,
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

        let (output, pass) = match &spec.kind {
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
            NodeKind::Shader { node: def_name } => {
                let Some(def) = library.get(def_name) else {
                    error!("node `{id}`: unknown node kind `{def_name}`");
                    continue;
                };
                let shader = node_shaders.shader_for(def, &mut shaders);
                let mut material = NodeMaterial::new(shader, &fallbacks.black);
                material.frame.aspect = aspect;
                let material = materials.add(material);

                let output = if def.feedback {
                    Output::PingPong {
                        images: [
                            images.add(target_image(resolution.0)),
                            images.add(target_image(resolution.0)),
                        ],
                        write: 0,
                    }
                } else {
                    Output::Single(images.add(target_image(resolution.0)))
                };
                let target = match &output {
                    Output::Single(h) => h.clone(),
                    Output::PingPong { images, write } => images[*write].clone(),
                };

                let quad = commands
                    .spawn((
                        Name::new(format!("pass quad: {id}")),
                        Mesh3d(quad_mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::IDENTITY,
                        layer.clone(),
                    ))
                    .id();
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
                (
                    output,
                    Some(Pass {
                        camera,
                        quad,
                        material,
                        def: def.name.clone(),
                    }),
                )
            }
        };

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

/// Recompile project node files that changed on disk. Header changes that
/// alter inputs or parameters need a restart; the body is live.
pub fn hot_reload(
    time: Res<Time<Real>>,
    mut node_shaders: ResMut<NodeShaders>,
    mut library: ResMut<LibraryRes>,
    mut shaders: ResMut<Assets<Shader>>,
) {
    let now = SystemTime::now();
    if let Some(last) = node_shaders.last_check
        && now.duration_since(last).unwrap_or_default() < HOT_RELOAD_INTERVAL
    {
        return;
    }
    node_shaders.last_check = Some(now);
    let _ = time;

    let mut recompiled = Vec::new();
    for watched in node_shaders.watched.iter_mut() {
        let modified = mtime(&watched.path);
        if modified == watched.modified {
            continue;
        }
        watched.modified = modified;
        let Some(old) = library.get(&watched.def).cloned() else {
            continue;
        };
        match NodeDef::load_file(&watched.path) {
            Ok(mut def) => {
                def.name = old.name.clone();
                if def.inputs != old.inputs
                    || def.params != old.params
                    || def.feedback != old.feedback
                {
                    warn!(
                        "node `{}`: header changed (inputs/params/feedback); restart the renderer to apply structural changes. Body reloaded.",
                        def.name
                    );
                    def.inputs = old.inputs.clone();
                    def.params = old.params.clone();
                    def.feedback = old.feedback;
                }
                recompiled.push((def.name.clone(), compile(&def)));
                library.insert(def);
            }
            Err(e) => error!("node `{}`: {e}", watched.def),
        }
    }
    for (name, shader) in recompiled {
        if let Some(handle) = node_shaders.handles.get(&name) {
            match shaders.insert(handle.id(), shader) {
                Ok(()) => info!("node `{name}`: shader reloaded"),
                Err(e) => error!("node `{name}`: could not replace shader: {e}"),
            }
        }
    }
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
/// upstream nodes. Only writes when a handle actually changed so bind groups
/// are not rebuilt needlessly.
pub fn rewire(
    runtime: Res<Runtime>,
    fallbacks: Res<Fallbacks>,
    mut materials: ResMut<Assets<NodeMaterial>>,
) {
    for node in &runtime.nodes {
        let Some(pass) = &node.pass else { continue };
        let mut wanted: Vec<Handle<Image>> = Vec::with_capacity(MAX_INPUTS + 1);
        for slot in 0..MAX_INPUTS {
            wanted.push(
                node.inputs
                    .get(slot)
                    .and_then(|id| runtime.output_of(id))
                    .unwrap_or_else(|| fallbacks.black.clone()),
            );
        }
        let previous = node
            .previous_output()
            .cloned()
            .unwrap_or_else(|| fallbacks.black.clone());

        let Some(current) = materials.get(&pass.material) else {
            continue;
        };
        let unchanged = (0..MAX_INPUTS).all(|slot| current.input(slot) == Some(&wanted[slot]))
            && current.previous == previous;
        if unchanged {
            continue;
        }
        let Some(mut material) = materials.get_mut(&pass.material) else {
            continue;
        };
        for (slot, handle) in wanted.into_iter().enumerate() {
            if let Some(input) = material.input_mut(slot) {
                *input = handle;
            }
        }
        material.previous = previous;
    }
}

/// Write resolved parameter values and frame data into the materials.
pub fn apply_params(
    runtime: Res<Runtime>,
    library: Res<LibraryRes>,
    state: Res<ParamState>,
    time: Res<Time>,
    resolution: Res<NodeResolution>,
    mut materials: ResMut<Assets<NodeMaterial>>,
) {
    let aspect = resolution.aspect();
    let frame_base = FrameUniform {
        time: time.elapsed_secs(),
        dt: time.delta_secs(),
        aspect,
        index: runtime.frame as f32,
        ..Default::default()
    };

    for node in &runtime.nodes {
        let Some(pass) = &node.pass else { continue };
        let Some(def) = library.get(&pass.def) else {
            continue;
        };

        let values: BTreeMap<String, ParamValue> = def
            .params
            .iter()
            .filter_map(|spec| {
                state
                    .resolved
                    .get(&ParamPath::new(node.id.clone(), spec.name.clone()))
                    .map(|v| (spec.name.clone(), v.clone()))
            })
            .collect();
        let mut bytes = [0u8; UNIFORM_BYTES];
        def.write_uniform(&values, &mut bytes);

        let mut connected = 0u32;
        for slot in 0..node.inputs.len().min(MAX_INPUTS) {
            connected |= 1 << slot;
        }

        let Some(mut material) = materials.get_mut(&pass.material) else {
            continue;
        };
        material.params = ParamsBlob::from_bytes(&bytes);
        material.frame = FrameUniform {
            connected,
            ..frame_base
        };
    }
}
