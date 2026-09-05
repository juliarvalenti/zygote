//! Monochrome, minimal, pulsing geometry and 3D noise haze from a central
//! point. Every node in this show is a WGSL file under `assets/nodes/`, so the
//! whole piece is editable while it runs (save a node file, the pass reloads).

use zygote_render::prelude::*;

fn main() {
    ZygoteApp::new()
        .asset_root(env!("CARGO_MANIFEST_DIR"))
        .node_dir("nodes")
        .graph_file("graphs/main.json")
        .parse_args()
        .run();
}
