//! A webcam through the effect chain. The `camera` node reads device `"0"`
//! (the first camera); set `ZYGOTE_CAMERA=synthetic` to run it on the
//! built-in test feed on a machine without one. Cues step through a few
//! looks; keys ripple the warp and wipe the trails.

use zygote_render::prelude::*;

fn main() {
    ZygoteApp::new()
        .asset_root(env!("CARGO_MANIFEST_DIR"))
        .graph_file("graphs/main.json")
        .parse_args()
        .run();
}
