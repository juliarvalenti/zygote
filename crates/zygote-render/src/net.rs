//! UDP parameter input (protocol v2).

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowPosition, WindowResolution};
use zygote_core::{Message, PROTOCOL_VERSION, ParamPath, ParamReceiver, ParamValue};

use crate::params::{ParamState, Transport};
use crate::plugin::{GraphRes, LibraryRes};

#[derive(Resource, Clone, Debug)]
pub struct NetConfig {
    pub port: u16,
    /// Where image sources live, so their preview paths can be sent to UIs.
    pub assets_dir: Option<std::path::PathBuf>,
}

#[derive(Resource)]
pub struct Net(pub ParamReceiver);

pub fn bind(mut commands: Commands, config: Res<NetConfig>) {
    match ParamReceiver::bind(config.port) {
        Ok(rx) => {
            info!(
                "listening for parameter messages on udp://127.0.0.1:{} (protocol {PROTOCOL_VERSION})",
                config.port
            );
            commands.insert_resource(Net(rx));
        }
        Err(e) => {
            error!(
                "could not bind udp port {}: {e}. Is another renderer running? Pass --port to use a different one.",
                config.port
            );
        }
    }
}

pub fn poll(
    net: Option<ResMut<Net>>,
    config: Res<NetConfig>,
    graph: Res<GraphRes>,
    library: Res<LibraryRes>,
    mut state: ResMut<ParamState>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let Some(mut net) = net else { return };
    for (msg, from) in net.0.poll() {
        match msg {
            Message::Hello { client, protocol } => {
                if protocol != PROTOCOL_VERSION {
                    warn!(
                        "client `{client}` speaks protocol {protocol}, this renderer speaks {PROTOCOL_VERSION}; describing anyway"
                    );
                } else {
                    info!("client `{client}` connected from {from}");
                }
                let structure = Message::structure(
                    graph.structure_with_assets(&library, config.assets_dir.as_deref()),
                );
                if let Err(e) = net.0.send_to(&structure, from) {
                    warn!("failed to send graph structure to {from}: {e}");
                }
                let params = graph.describe_params(&library);
                for chunk in Message::describe(&graph.name, &params) {
                    if let Err(e) = net.0.send_to(&chunk, from) {
                        warn!("failed to describe graph to {from}: {e}");
                    }
                }
            }
            Message::SetParam { path, value } => set(&graph, &library, &mut state, path, value),
            Message::SetParams { values } => {
                for (path, value) in values {
                    set(&graph, &library, &mut state, path, value);
                }
            }
            Message::ClearParam { path } => {
                state.overrides.remove(&path);
            }
            Message::ClearAll => state.overrides.clear(),
            Message::Transport { time, playing } => {
                state.transport = Some(Transport {
                    time,
                    playing,
                    received: std::time::Instant::now(),
                });
            }
            Message::Arrange { bounds } => {
                for mut window in &mut windows {
                    match bounds {
                        Some(b) => {
                            info!(
                                "arranging output window at {},{} {}x{}",
                                b.x, b.y, b.width, b.height
                            );
                            window.position = WindowPosition::At(IVec2::new(b.x, b.y));
                            window.resolution = WindowResolution::new(b.width, b.height);
                        }
                        None => {
                            info!("releasing output window to its default placement");
                            window.position = WindowPosition::Automatic;
                            window.resolution = WindowResolution::new(1280, 720);
                        }
                    }
                }
            }
            Message::Modulation { modulation } => {
                debug!(
                    "modulation: {} sources, {} assignments",
                    modulation.sources.len(),
                    modulation.assignments.len()
                );
                state.modulation = modulation;
            }
            Message::Gate { event } => state.gates.push(event),
            Message::Ping => {
                if let Err(e) = net.0.send_to(&Message::pong(), from) {
                    debug!("failed to answer ping from {from}: {e}");
                }
            }
            Message::Describe { .. } | Message::Structure { .. } | Message::Pong { .. } => {}
        }
    }
}

fn set(
    graph: &GraphRes,
    library: &LibraryRes,
    state: &mut ParamState,
    path: ParamPath,
    value: ParamValue,
) {
    match graph.param_spec(library, &path) {
        Ok(spec) => {
            state.overrides.insert(path, spec.conform(&value));
        }
        Err(_) => debug!("ignoring unknown parameter `{path}`"),
    }
}
