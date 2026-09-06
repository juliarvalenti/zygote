//! Bevy plugin wiring the node graph, parameter resolution and networking.

use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::shader::Shader;
use zygote_core::{AudioBands, DEFAULT_PORT, Graph, NodeLibrary};

use crate::materials::{Fallbacks, NodeMaterial};
use crate::sources::{LiveSources, SourceFactories};
use crate::{net, nodes, params, scene, sources};

/// Everything the render process needs to know at startup.
#[derive(Clone, Debug)]
pub struct RenderSettings {
    pub graph: Graph,
    pub port: u16,
    /// Resolution of every node's render target.
    pub resolution: UVec2,
    /// Ignore transport messages and always run on the wall clock.
    pub free_run: bool,
    /// Exit when the process that started us goes away (a UI that launched
    /// this renderer), so a crashed or killed host leaves no orphan.
    pub exit_with_parent: bool,
    /// Directory image paths are relative to (`<asset root>/assets`). Sent
    /// to UIs so they can preview image sources; `None` sends no previews.
    pub assets_dir: Option<std::path::PathBuf>,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            graph: Graph::first_pass(),
            port: DEFAULT_PORT,
            resolution: UVec2::new(1280, 720),
            free_run: false,
            exit_with_parent: false,
            assets_dir: None,
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
    sources: SourceFactories,
}

impl ZygotePlugin {
    pub fn new(settings: RenderSettings, library: NodeLibrary, sources: SourceFactories) -> Self {
        Self {
            settings,
            library,
            sources,
        }
    }
}

impl Plugin for ZygotePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/common.wgsl");

        app.add_plugins(MaterialPlugin::<NodeMaterial>::default());

        if self.settings.exit_with_parent {
            app.add_systems(Update, exit_with_parent);
        }
        app.insert_resource(GraphRes(self.settings.graph.clone()))
            .insert_resource(LibraryRes(self.library.clone()))
            .insert_resource(NodeResolution(self.settings.resolution))
            .insert_resource(net::NetConfig {
                port: self.settings.port,
                assets_dir: self.settings.assets_dir.clone(),
            })
            .init_resource::<AudioBandsRes>()
            .insert_resource(params::FrameClock {
                free_run: self.settings.free_run,
                ..Default::default()
            })
            .init_resource::<params::ParamState>()
            .init_resource::<Fallbacks>()
            .init_resource::<nodes::NodeShaders>()
            .insert_resource(self.sources.clone())
            .init_resource::<LiveSources>();

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
        .add_systems(
            Update,
            (params::tick_clock, params::resolve)
                .chain()
                .in_set(ZygoteSet::Resolve),
        )
        .add_systems(
            Update,
            (
                nodes::hot_reload,
                nodes::watch_image_loads,
                nodes::swap_feedback,
                nodes::rewire,
                nodes::apply_params,
                sources::update_sources,
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

/// Quit once our parent process is gone. The parent pid is read on the
/// first run; when it changes the original parent has exited and we were
/// re-parented to init (or a subreaper).
#[cfg(unix)]
fn exit_with_parent(
    mut parent: Local<Option<i32>>,
    mut last_check: Local<f64>,
    time: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs_f64();
    if now - *last_check < 1.0 {
        return;
    }
    *last_check = now;
    // SAFETY: getppid has no preconditions and cannot fail.
    let current = unsafe { libc::getppid() };
    match *parent {
        None => *parent = Some(current),
        Some(original) if original != current => {
            info!("parent process {original} is gone; exiting");
            exit.write(AppExit::Success);
        }
        Some(_) => {}
    }
}

#[cfg(not(unix))]
fn exit_with_parent() {}
