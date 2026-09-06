//! Zygote timeline: a gpui-kit host window for sequencing node parameters.
//!
//! The UI knows nothing about textures or shaders. It holds a
//! [`zygote_core::Timeline`] of cues (snapshots of parameter values), a
//! transport, and a slider per parameter. Every tick it evaluates the timeline,
//! applies manual overrides on top, and pushes the resulting numbers to the
//! renderer over UDP as `node.param = value` messages.
//!
//! ```text
//! zygote-timeline                                   # project browser
//! zygote-timeline [timeline.json] [--target 127.0.0.1:9471]  # straight into a show
//! ```

mod app;
mod projects;
mod shell;
mod supervisor;

fn main() {
    app::run();
}
