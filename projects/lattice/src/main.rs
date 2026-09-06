//! Op-art from builtin nodes only. A checkerboard beats against a second,
//! slightly rotated grid (moiré); the interference folds through a
//! kaleidoscope and mirror tiles. A radial mask keeps the center sharp and
//! lets the edges go blocky through `pixelate`. The show file steps the
//! symmetry, turns the grids with LFOs, and binds keys that chunk the
//! pixels and kick the spin.

use zygote_render::prelude::*;

fn main() {
    ZygoteApp::new()
        .asset_root(env!("CARGO_MANIFEST_DIR"))
        .graph_file("graphs/main.json")
        .parse_args()
        .run();
}
