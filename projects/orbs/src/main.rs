//! Bloom is a graph, not a node: `blur` with a brightness threshold, added
//! back over the source with `blend`. Here the source is `voronoi` in its
//! bubbles mode, colored by a palette and smeared upward by `streak`.
//! LFOs breathe the cell jitter and the bloom radius; a key flashes the
//! bloom gain.

use zygote_render::prelude::*;

fn main() {
    ZygoteApp::new()
        .asset_root(env!("CARGO_MANIFEST_DIR"))
        .graph_file("graphs/main.json")
        .parse_args()
        .run();
}
