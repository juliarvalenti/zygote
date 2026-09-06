//! An analog XY oscilloscope. Two oscillators drive the beam, a third pushes
//! the figure toward and away from the glass, and the trace only exists
//! because the phosphor remembers it: each frame draws just the arc the beam
//! swept since the last one, and a feedback pass lets it fade.
//!
//! Everything is WGSL under `assets/nodes/`. The show file
//! (`scope.show.json`) steps through harmonic ratios with ramps, so the knot
//! unties itself between cues, and puts a slow LFO on the phase so the figure
//! turns.

use zygote_render::prelude::*;

fn main() {
    ZygoteApp::new()
        .asset_root(env!("CARGO_MANIFEST_DIR"))
        .node_dir("nodes")
        .graph_file("graphs/main.json")
        .parse_args()
        .run();
}
