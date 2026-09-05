//! Project-facing builder and the nannou window.

use std::path::PathBuf;

use bevy::camera::{Hdr, PerspectiveProjection, Projection};
use bevy::core_pipeline::tonemapping::Tonemapping;
use nannou::prelude::*;
use zygote_core::{CpuSourceInfo, Graph, InputDef, NodeDef, NodeLibrary, NodeOrigin, NodeParams};

use crate::sources::{FrameInfo, SourceFactories, SourceFactory};

use crate::capture::{CapturePlugin, CaptureSettings};
use crate::plugin::{RenderSettings, ZygotePlugin};
use crate::{CAMERA_DISTANCE, CAMERA_FOV};

/// Declaration of one texture input of a [`RustNode`].
#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub name: &'static str,
    pub optional: bool,
}

impl Input {
    pub const fn required(name: &'static str) -> Self {
        Self {
            name,
            optional: false,
        }
    }

    pub const fn optional(name: &'static str) -> Self {
        Self {
            name,
            optional: true,
        }
    }
}

/// A node declared in Rust: the same closed vocabulary as a WGSL node file
/// (one fullscreen pass, named inputs, optional feedback tap, typed params),
/// with the parameters as a `#[derive(NodeParams)]` struct instead of a
/// `//!` header. No Bevy types appear here on purpose.
pub trait RustNode {
    /// Name used as the graph `type`.
    const NAME: &'static str;
    const DOC: &'static str = "";
    const INPUTS: &'static [Input] = &[];
    /// Bind last frame's own output as `previous(uv)`.
    const FEEDBACK: bool = false;
    /// WGSL fragment body. Parameters are `params.<field>`, inputs are
    /// `<name>(uv)`, frame data is `frame.time` / `frame.aspect`.
    const SHADER: &'static str;
    type Params: NodeParams;

    fn definition() -> NodeDef {
        NodeDef {
            name: Self::NAME.to_owned(),
            doc: Self::DOC.to_owned(),
            inputs: Self::INPUTS
                .iter()
                .map(|i| InputDef {
                    name: i.name.to_owned(),
                    optional: i.optional,
                })
                .collect(),
            feedback: Self::FEEDBACK,
            params: Self::Params::specs(),
            source: Self::SHADER.to_owned(),
            origin: NodeOrigin::Rust(std::any::type_name::<Self>().to_owned()),
            cpu_source: None,
        }
    }
}

/// A texture produced on the CPU every frame: simulations, decoders,
/// anything with state a fullscreen pass cannot hold. Declared like a
/// [`RustNode`] (typed params, no Bevy types); Zygote owns the texture,
/// uploads it, and wires it into the graph like any other node output.
pub trait RustSource: Send + Sync + 'static {
    /// Name used as the graph `type`.
    const NAME: &'static str;
    const DOC: &'static str = "";
    /// Texture size in pixels.
    const WIDTH: u32;
    const HEIGHT: u32;
    /// Sample with nearest filtering (pixel look) instead of linear.
    const NEAREST: bool = false;
    type Params: NodeParams;

    fn new() -> Self;

    /// Write RGBA8 pixels (`WIDTH * HEIGHT * 4` bytes, row-major) for this
    /// frame. `frame.dt` is 0 while the transport is paused.
    fn update(&mut self, params: &Self::Params, frame: &FrameInfo, pixels: &mut [u8]);

    fn definition() -> NodeDef {
        NodeDef {
            name: Self::NAME.to_owned(),
            doc: Self::DOC.to_owned(),
            inputs: Vec::new(),
            feedback: false,
            params: Self::Params::specs(),
            source: String::new(),
            origin: NodeOrigin::Rust(std::any::type_name::<Self>().to_owned()),
            cpu_source: Some(CpuSourceInfo {
                width: Self::WIDTH,
                height: Self::HEIGHT,
                nearest: Self::NEAREST,
            }),
        }
    }
}

/// Builder for a Zygote render process.
pub struct ZygoteApp {
    settings: RenderSettings,
    library: NodeLibrary,
    sources: SourceFactories,
    asset_root: Option<PathBuf>,
    node_dirs: Vec<PathBuf>,
    graph_file: Option<PathBuf>,
    capture: Option<CaptureSettings>,
    errors: Vec<String>,
}

impl Default for ZygoteApp {
    fn default() -> Self {
        Self::new()
    }
}

impl ZygoteApp {
    /// Builtin node library, default first-pass graph, default port.
    pub fn new() -> Self {
        Self {
            settings: RenderSettings::default(),
            library: NodeLibrary::builtin(),
            sources: SourceFactories::default(),
            asset_root: None,
            node_dirs: Vec::new(),
            graph_file: None,
            capture: None,
            errors: Vec::new(),
        }
    }

    /// Directory containing `assets/` (images, node files, graphs). Projects
    /// pass `env!("CARGO_MANIFEST_DIR")`.
    pub fn asset_root(mut self, dir: impl Into<PathBuf>) -> Self {
        self.asset_root = Some(dir.into());
        self
    }

    /// Load every `*.wgsl` node file in `dir` (relative to `assets/`).
    /// Files are watched and hot-reloaded while running.
    pub fn node_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.node_dirs.push(dir.into());
        self
    }

    /// Register a Rust-declared node kind.
    pub fn register<N: RustNode>(mut self) -> Self {
        let def = N::definition();
        if let Err(e) = def.validate() {
            self.errors.push(format!("node `{}`: {e}", N::NAME));
        }
        self.library.insert(def);
        self
    }

    /// Register a CPU texture source.
    pub fn register_source<S: RustSource>(mut self) -> Self {
        let def = S::definition();
        if let Err(e) = def.validate() {
            self.errors.push(format!("source `{}`: {e}", S::NAME));
        }
        self.library.insert(def);
        self.sources.insert(S::NAME, SourceFactory::of::<S>());
        self
    }

    /// Register an already-built definition.
    pub fn register_def(mut self, def: NodeDef) -> Self {
        if let Err(e) = def.validate() {
            self.errors.push(format!("node `{}`: {e}", def.name));
        }
        self.library.insert(def);
        self
    }

    pub fn graph(mut self, graph: Graph) -> Self {
        self.settings.graph = graph;
        self.graph_file = None;
        self
    }

    /// Graph JSON file, relative to `assets/` (or absolute / cwd-relative).
    pub fn graph_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.graph_file = Some(path.into());
        self
    }

    /// Always run on the wall clock, ignoring the UI's transport.
    pub fn free_run(mut self) -> Self {
        self.settings.free_run = true;
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.settings.port = port;
        self
    }

    pub fn resolution(mut self, width: u32, height: u32) -> Self {
        self.settings.resolution = UVec2::new(width, height);
        self
    }

    /// Render `frame` frames, save a screenshot, exit.
    pub fn capture(mut self, path: impl Into<String>, frame: u64) -> Self {
        self.capture = Some(CaptureSettings {
            path: path.into(),
            frame,
            every: 0,
        });
        self
    }

    /// Apply command line overrides:
    ///
    /// ```text
    /// [graph.json] [--port N] [--size WxH] [--assets DIR] [--nodes DIR]
    /// [--showcase] [--free-run] [--capture out.png [--frames N] [--every K]]
    /// ```
    pub fn parse_args(mut self) -> Self {
        let mut args = std::env::args().skip(1);
        let mut capture_path: Option<String> = None;
        let mut capture_frame: u64 = 120;
        let mut capture_every: u64 = 0;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--port" => {
                    self.settings.port = args
                        .next()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or_else(|| usage("--port needs a number"));
                }
                "--size" => {
                    let value = args.next().unwrap_or_else(|| usage("--size needs WxH"));
                    let (w, h) = value
                        .split_once('x')
                        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                        .unwrap_or_else(|| usage("--size needs WxH"));
                    self.settings.resolution = UVec2::new(w, h);
                }
                "--assets" => {
                    self.asset_root = Some(PathBuf::from(
                        args.next()
                            .unwrap_or_else(|| usage("--assets needs a directory")),
                    ));
                }
                "--nodes" => {
                    self.node_dirs.push(PathBuf::from(
                        args.next()
                            .unwrap_or_else(|| usage("--nodes needs a directory")),
                    ));
                }
                "--capture" => {
                    capture_path = Some(
                        args.next()
                            .unwrap_or_else(|| usage("--capture needs a path")),
                    );
                }
                "--frames" => {
                    capture_frame = args
                        .next()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or_else(|| usage("--frames needs a number"));
                }
                "--every" => {
                    capture_every = args
                        .next()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or_else(|| usage("--every needs a number"));
                }
                "--showcase" => {
                    self.settings.graph = Graph::showcase();
                    self.graph_file = None;
                }
                "--free-run" => self.settings.free_run = true,
                "-h" | "--help" => usage(""),
                path if !path.starts_with('-') => self.graph_file = Some(PathBuf::from(path)),
                other => usage(&format!("unknown argument {other}")),
            }
        }
        if let Some(path) = capture_path {
            self.capture = Some(CaptureSettings {
                path,
                frame: capture_frame,
                every: capture_every,
            });
        }
        self
    }

    /// Resolve files, validate, open the window. Does not return until exit.
    pub fn run(mut self) {
        let asset_root = resolve_asset_root(self.asset_root.clone());
        let assets = asset_root.join("assets");

        for dir in std::mem::take(&mut self.node_dirs) {
            let full = if dir.is_absolute() {
                dir.clone()
            } else {
                assets.join(&dir)
            };
            let (loaded, errors) = self.library.load_dir(&full);
            for name in loaded {
                eprintln!("loaded node `{name}` from {}", full.display());
            }
            for (path, e) in errors {
                eprintln!("error: node file {}: {e}", path.display());
            }
        }

        if let Some(path) = self.graph_file.take() {
            let candidates = [
                path.clone(),
                assets.join(&path),
                assets.join("graphs").join(&path),
            ];
            let Some(found) = candidates.iter().find(|p| p.is_file()) else {
                usage(&format!("graph file not found: {}", path.display()));
            };
            let json = std::fs::read_to_string(found)
                .unwrap_or_else(|e| usage(&format!("cannot read {}: {e}", found.display())));
            self.settings.graph = Graph::from_json(&json)
                .unwrap_or_else(|e| usage(&format!("invalid graph {}: {e}", found.display())));
        }

        if !self.errors.is_empty() {
            for e in &self.errors {
                eprintln!("error: {e}");
            }
            std::process::exit(2);
        }
        if let Err(e) = self.settings.graph.validate(&self.library) {
            usage(&format!(
                "graph `{}` is invalid: {e}",
                self.settings.graph.name
            ));
        }

        let mut builder = nannou::app(model)
            .update(update)
            .add_plugin(ZygotePlugin::new(self.settings, self.library, self.sources));
        if let Some(capture) = self.capture {
            builder = builder.add_plugin(CapturePlugin(capture));
        }
        builder.run();
    }
}

fn usage(error: &str) -> ! {
    if !error.is_empty() {
        eprintln!("error: {error}\n");
    }
    eprintln!(
        "usage: <project> [graph.json] [--port N] [--size WxH] [--assets DIR] [--nodes DIR] [--showcase] [--free-run] [--capture out.png [--frames N] [--every K]]"
    );
    std::process::exit(if error.is_empty() { 0 } else { 2 });
}

/// Bevy resolves `assets/` from `BEVY_ASSET_ROOT`, then the cargo manifest
/// dir, then the executable. Pick a root that actually has an `assets/`
/// directory and pin it before the app starts. Returns the chosen root.
fn resolve_asset_root(explicit: Option<PathBuf>) -> PathBuf {
    let candidates = [
        explicit,
        std::env::var_os("BEVY_ASSET_ROOT").map(PathBuf::from),
        std::env::current_dir().ok(),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
    ];
    let root = candidates
        .into_iter()
        .flatten()
        .find(|root| root.join("assets").is_dir())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    // SAFETY: called from `run` before any other thread exists.
    unsafe { std::env::set_var("BEVY_ASSET_ROOT", &root) };
    root
}

struct Model {
    _window: Entity,
}

fn model(app: &App) -> Model {
    // 3D from the start: a perspective camera with a depth buffer looking down
    // -Z at the display quad spawned by `scene::spawn_display`.
    let camera = app
        .new_camera()
        .projection(Projection::Perspective(PerspectiveProjection {
            fov: CAMERA_FOV,
            near: 0.05,
            far: 100.0,
            ..Default::default()
        }))
        .xyz(Vec3::new(0.0, 0.0, CAMERA_DISTANCE))
        .tonemapping(Tonemapping::None)
        .clear_color(ClearColorConfig::Custom(Color::BLACK))
        .build();
    app.command_scope(|mut commands| {
        commands.entity(camera).insert(Hdr);
    });

    let window = app
        .new_window()
        .window(bevy::window::Window {
            title: "Zygote".to_owned(),
            ..Default::default()
        })
        .camera(camera)
        .primary()
        .key_pressed(key_pressed)
        .build();

    Model { _window: window }
}

fn update(_app: &App, _model: &mut Model) {}

fn key_pressed(app: &App, _model: &mut Model, key: KeyCode) {
    match key {
        KeyCode::KeyS => {
            let path = format!(
                "zygote-{}.png",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            );
            info!("saving screenshot to {path}");
            app.main_window().save_screenshot(path);
        }
        KeyCode::Escape => app.quit(),
        _ => {}
    }
}
