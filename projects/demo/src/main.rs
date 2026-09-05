//! A project that depends on Zygote as a library and adds its own nodes.
//!
//! Two authoring forms, both the same closed vocabulary (one fullscreen pass,
//! named inputs, typed params):
//!
//! * `kaleido` is declared here in Rust: a `#[derive(NodeParams)]` struct is
//!   the whole parameter declaration; the WGSL body lives next to it.
//! * `scanlines` is a plain WGSL file under `assets/nodes/` with a `//!`
//!   header, loaded (and hot-reloaded) at runtime.
//!
//! Run with `cargo run -p zygote-demo` and drive it with `zygote-timeline`.

use zygote_render::prelude::*;

/// Mirror the source into `segments` wedges around a centre point.
#[derive(NodeParams, Clone, Debug)]
pub struct Kaleido {
    /// Number of mirror wedges
    #[param(default = 6.0, min = 1.0, max = 24.0)]
    pub segments: f32,
    /// Rotation of the wedge pattern (turns)
    #[param(default = 0.0, min = -1.0, max = 1.0)]
    pub spin: f32,
    /// Centre of the mirror in UV space
    #[param(default = [0.5, 0.5], min = 0.0, max = 1.0)]
    pub centre: [f32; 2],
    /// Multiply the result by this tint
    #[param(default = "#ffffff")]
    pub tint: [f32; 4],
    /// Mirror every other wedge instead of repeating
    #[param(default = true)]
    pub mirror: bool,
}

impl RustNode for Kaleido {
    const NAME: &'static str = "kaleido";
    const DOC: &'static str = "Kaleidoscope: mirrors the source into wedges";
    const INPUTS: &'static [Input] = &[Input::required("source")];
    const SHADER: &'static str = include_str!("kaleido.wgsl");
    type Params = Self;
}

fn main() {
    ZygoteApp::new()
        .asset_root(env!("CARGO_MANIFEST_DIR"))
        .register::<Kaleido>()
        .node_dir("nodes")
        .graph_file("graphs/main.json")
        .parse_args()
        .run();
}
