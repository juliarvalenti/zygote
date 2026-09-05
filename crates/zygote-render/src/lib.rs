//! Zygote render engine.
//!
//! A project is a binary that configures a [`ZygoteApp`]: which graph to
//! load, where its assets and node files live, and which Rust-declared nodes
//! to register. Everything a node may declare is data (see
//! [`zygote_core::NodeDef`]); node code never touches Bevy.
//!
//! ```ignore
//! use zygote_render::prelude::*;
//!
//! #[derive(NodeParams, Clone)]
//! struct Kaleido {
//!     #[param(default = 6.0, min = 1.0, max = 24.0)]
//!     segments: f32,
//! }
//!
//! impl RustNode for Kaleido {
//!     const NAME: &'static str = "kaleido";
//!     const INPUTS: &'static [Input] = &[Input::required("source")];
//!     const SHADER: &'static str = include_str!("kaleido.wgsl");
//!     type Params = Self;
//! }
//!
//! fn main() {
//!     ZygoteApp::new()
//!         .asset_root(env!("CARGO_MANIFEST_DIR"))
//!         .register::<Kaleido>()
//!         .graph_file("graphs/main.json")
//!         .parse_args()
//!         .run();
//! }
//! ```

mod app;
mod capture;
mod materials;
mod net;
mod nodes;
mod output_window;
mod params;
mod plugin;
mod scene;
mod sources;

pub use app::{Input, RustNode, RustSource, ZygoteApp};
pub use capture::CaptureSettings;
pub use plugin::RenderSettings;
pub use sources::FrameInfo;
/// The data model, re-exported so projects need only this crate.
pub use zygote_core as core;

/// Everything a project binary needs.
pub mod prelude {
    pub use crate::{FrameInfo, Input, RustNode, RustSource, ZygoteApp};
    pub use zygote_core::{
        Graph, NodeDef, NodeId, NodeKind, NodeLibrary, NodeParams, NodeSpec, ParamPath, ParamSpec,
        ParamValue,
    };
    pub use zygote_macros::NodeParams;
}

/// Distance from the perspective camera to the display quad.
pub const CAMERA_DISTANCE: f32 = 3.0;
/// Vertical field of view of the display camera.
pub const CAMERA_FOV: f32 = 45.0_f32.to_radians();
