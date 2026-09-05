//! Per-frame parameter state: UDP overrides + modulators → resolved values.

use std::collections::BTreeMap;

use bevy::prelude::*;
use zygote_core::{ModContext, ParamPath, ParamValue, ResolvedParams, resolve_params};

use crate::plugin::{AudioBandsRes, GraphRes, LibraryRes};

#[derive(Resource, Default, Debug)]
pub struct ParamState {
    /// Values pushed by clients (timeline cues and manual overrides alike).
    /// These win over the graph's own base values.
    pub overrides: BTreeMap<ParamPath, ParamValue>,
    /// Fully resolved values for this frame.
    pub resolved: ResolvedParams,
    /// Last transport state reported by a client, if any.
    pub transport: Option<Transport>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transport {
    pub time: f32,
    pub playing: bool,
}

pub fn resolve(
    time: Res<Time>,
    graph: Res<GraphRes>,
    library: Res<LibraryRes>,
    audio: Res<AudioBandsRes>,
    mut state: ResMut<ParamState>,
) {
    let ctx = ModContext {
        time: time.elapsed_secs(),
        dt: time.delta_secs(),
        audio: **audio,
    };
    let resolved = resolve_params(&graph, &library, &ctx, &BTreeMap::new(), &state.overrides);
    state.resolved = resolved;
}
