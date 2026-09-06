//! An etching from builtin nodes only. Fractal noise is warped by a
//! cellular field through `warp`'s displacement input, traced by
//! `edge_detect`, mirrored, reduced to two levels by ordered `dither`, and
//! finally used as the mask that prints ink over paper with `luma_mask` and
//! two `solid` colors. A slow LFO turns the twist; a key presses harder.

use zygote_render::prelude::*;

fn main() {
    ZygoteApp::new()
        .asset_root(env!("CARGO_MANIFEST_DIR"))
        .graph_file("graphs/main.json")
        .parse_args()
        .run();
}
