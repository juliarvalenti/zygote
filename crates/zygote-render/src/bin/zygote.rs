//! The stock Zygote renderer: builtin nodes, graph from a file or the
//! built-in defaults, assets from the engine crate.
//!
//! ```text
//! zygote [graph.json] [--port N] [--size WxH] [--assets DIR] [--nodes DIR]
//!        [--showcase] [--capture out.png [--frames N]]
//! ```
//!
//! Keys: `S` saves a screenshot, `Esc` quits.

fn main() {
    zygote_render::ZygoteApp::new().parse_args().run();
}
