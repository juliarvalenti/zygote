//! Mycelium: a hyphal network grows across a coarse grid while nutrient
//! agents trace its structure. The simulation is CPU state, so it is a
//! [`RustSource`]: Zygote owns the texture and the graph, the node owns the
//! grid. Growth speed, flow and sensing are parameters, which makes them
//! cue-able and LFO-able like anything else.

mod sim;

use zygote_render::prelude::*;

use crate::sim::Mycelium;

fn main() {
    ZygoteApp::new()
        .asset_root(env!("CARGO_MANIFEST_DIR"))
        .register_source::<Mycelium>()
        .graph_file("graphs/main.json")
        .parse_args()
        .run();
}
