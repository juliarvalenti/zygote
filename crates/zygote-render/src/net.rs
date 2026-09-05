//! UDP parameter input.

use bevy::prelude::*;
use zygote_core::{Message, ParamReceiver};

use crate::params::{ParamState, Transport};
use crate::plugin::GraphRes;

#[derive(Resource, Clone, Copy, Debug)]
pub struct NetConfig {
    pub port: u16,
}

#[derive(Resource)]
pub struct Net(pub ParamReceiver);

pub fn bind(mut commands: Commands, config: Res<NetConfig>) {
    match ParamReceiver::bind(config.port) {
        Ok(rx) => {
            info!(
                "listening for parameter messages on udp://127.0.0.1:{}",
                config.port
            );
            commands.insert_resource(Net(rx));
        }
        Err(e) => {
            error!(
                "could not bind udp port {}: {e}. Parameters can only be changed from the graph file.",
                config.port
            );
        }
    }
}

pub fn poll(net: Option<ResMut<Net>>, graph: Res<GraphRes>, mut state: ResMut<ParamState>) {
    let Some(mut net) = net else { return };
    for (msg, from) in net.0.poll() {
        match msg {
            Message::Hello { client } => {
                info!("client `{client}` connected from {from}");
                let params = graph.describe_params();
                for chunk in Message::describe(&graph.name, &params) {
                    if let Err(e) = net.0.send_to(&chunk, from) {
                        warn!("failed to describe graph to {from}: {e}");
                    }
                }
            }
            Message::SetParam { path, value } => {
                set(&graph, &mut state, path, value);
            }
            Message::SetParams { values } => {
                for (path, value) in values {
                    set(&graph, &mut state, path, value);
                }
            }
            Message::ClearParam { path } => {
                state.overrides.remove(&path);
            }
            Message::ClearAll => state.overrides.clear(),
            Message::Transport { time, playing } => {
                state.transport = Some(Transport { time, playing });
            }
            Message::Describe { .. } => {}
        }
    }
}

fn set(graph: &GraphRes, state: &mut ParamState, path: zygote_core::ParamPath, value: f32) {
    match graph.param_spec(&path) {
        Some(spec) => {
            state.overrides.insert(path, spec.clamp(value));
        }
        None => debug!("ignoring unknown parameter `{path}`"),
    }
}
