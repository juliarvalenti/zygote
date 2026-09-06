//! A project that depends on Zygote as a library and adds its own nodes.
//!
//! Two authoring forms, both the same closed vocabulary (one fullscreen pass,
//! named inputs, typed params):
//!
//! * `rgb_shift` is declared here in Rust: a `#[derive(NodeParams)]` struct is
//!   the whole parameter declaration; the WGSL body lives next to it.
//! * `scanlines` is a plain WGSL file under `assets/nodes/` with a `//!`
//!   header, loaded (and hot-reloaded) at runtime.
//!
//! Run with `cargo run -p zygote-demo` and drive it with `zygote-timeline`.

use zygote_render::prelude::*;

/// Chromatic aberration: slide the red and blue channels apart.
#[derive(NodeParams, Clone, Debug)]
pub struct RgbShift {
    /// Offset distance (fraction of frame height)
    #[param(default = 0.004, min = 0.0, max = 0.05)]
    pub amount: f32,
    /// Direction of the offset (turns)
    #[param(default = 0.0, min = 0.0, max = 1.0)]
    pub angle: f32,
    /// Let the offset grow with distance from the centre, like a cheap lens
    #[param(default = 0.5, min = 0.0, max = 1.0)]
    pub radial: f32,
    /// Blend between the shifted and the original image
    #[param(default = 1.0, min = 0.0, max = 1.0)]
    pub mix: f32,
}

impl RustNode for RgbShift {
    const NAME: &'static str = "rgb_shift";
    const DOC: &'static str = "Chromatic aberration: red and blue slide apart";
    const INPUTS: &'static [Input] = &[Input::required("source")];
    const SHADER: &'static str = include_str!("rgb_shift.wgsl");
    type Params = Self;
}

fn main() {
    ZygoteApp::new()
        .asset_root(env!("CARGO_MANIFEST_DIR"))
        .register::<RgbShift>()
        .node_dir("nodes")
        .graph_file("graphs/main.json")
        .parse_args()
        .run();
}
