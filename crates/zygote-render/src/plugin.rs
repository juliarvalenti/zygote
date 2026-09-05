//! Bevy plugin wiring the node graph, parameter resolution and networking.

use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::shader::Shader;
use zygote_core::{AudioBands, DEFAULT_PORT, Graph, NodeLibrary};

use crate::materials::{Fallbacks, NodeMaterial};
use crate::{net, nodes, params, scene};

/// Everything the render process needs to know at startup.
#[derive(Clone, Debug)]
pub struct RenderSettings {
    pub graph: Graph,
    pub port: u16,
    /// Resolution of every node's render target.
    pub resolution: UVec2,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            graph: Graph::first_pass(),
            port: DEFAULT_PORT,
            resolution: UVec2::new(1280, 720),
        }
    }
}

/// The loaded graph description.
#[derive(Resource, Clone, Debug, Deref, DerefMut)]
pub struct GraphRes(pub Graph);

/// Every node kind this renderer knows.
#[derive(Resource, Clone, Debug, Deref, DerefMut)]
pub struct LibraryRes(pub NodeLibrary);

#[derive(Resource, Clone, Copy, Debug)]
pub struct NodeResolution(pub UVec2);

impl NodeResolution {
    pub fn aspect(&self) -> f32 {
        self.0.x as f32 / self.0.y.max(1) as f32
    }
}

/// Live FFT band energies. Nothing writes to this yet; an audio-analysis
/// plugin only needs to update it each frame for `audio_band` modulators to work.
#[derive(Resource, Clone, Copy, Debug, Default, Deref, DerefMut)]
pub struct AudioBandsRes(pub AudioBands);

/// Keeps the shared WGSL module alive so `#import zygote::common` resolves.
#[derive(Resource)]
pub struct CommonShader(#[allow(dead_code)] pub Handle<Shader>);

/// System sets, in execution order.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZygoteSet {
    /// Read UDP messages into the parameter state.
    Network,
    /// Combine base values, overrides and modulators.
    Resolve,
    /// Push resolved values into materials, swap feedback buffers, rewire inputs.
    Apply,
}

pub struct ZygotePlugin {
    settings: RenderSettings,
    library: NodeLibrary,
}

impl ZygotePlugin {
    pub fn new(settings: RenderSettings, library: NodeLibrary) -> Self {
        Self { settings, library }
    }
}

impl Plugin for ZygotePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/common.wgsl");

        app.add_plugins(MaterialPlugin::<NodeMaterial>::default());

        app.insert_resource(GraphRes(self.settings.graph.clone()))
            .insert_resource(LibraryRes(self.library.clone()))
            .insert_resource(NodeResolution(self.settings.resolution))
            .insert_resource(net::NetConfig {
                port: self.settings.port,
            })
            .init_resource::<AudioBandsRes>()
            .init_resource::<params::ParamState>()
            .init_resource::<Fallbacks>()
            .init_resource::<nodes::NodeShaders>();

        app.configure_sets(
            Update,
            (ZygoteSet::Network, ZygoteSet::Resolve, ZygoteSet::Apply).chain(),
        );

        app.add_systems(
            PreStartup,
            (
                load_common_shader,
                Fallbacks::init,
                net::bind,
                nodes::build_runtime,
            )
                .chain(),
        )
        .add_systems(Startup, scene::spawn_display)
        .add_systems(Update, net::poll.in_set(ZygoteSet::Network))
        .add_systems(Update, params::resolve.in_set(ZygoteSet::Resolve))
        .add_systems(
            Update,
            (
                nodes::hot_reload,
                nodes::swap_feedback,
                nodes::rewire,
                nodes::apply_params,
            )
                .chain()
                .in_set(ZygoteSet::Apply),
        )
        .add_systems(Update, scene::track_output.after(ZygoteSet::Apply));
    }
}

fn load_common_shader(mut commands: Commands, asset_server: Res<AssetServer>) {
    let handle: Handle<Shader> = asset_server.load("embedded://zygote_render/shaders/common.wgsl");
    commands.insert_resource(CommonShader(handle));
}
