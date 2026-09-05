//! Per-frame parameter state: UDP overrides + modulators → resolved values.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use zygote_core::{ModContext, ParamPath, ParamValue, ResolvedParams, resolve_params};

use crate::plugin::{AudioBandsRes, GraphRes, LibraryRes};

/// After this long without a transport message the clock free-runs again.
const TRANSPORT_TIMEOUT: Duration = Duration::from_secs(3);

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
    pub received: Instant,
}

/// The time every node shader and modulator sees.
///
/// While a client keeps sending `Transport` messages the clock follows the
/// client's playhead: pausing freezes the picture, scrubbing scrubs it. With
/// no client (or `--free-run`) it is the wall clock since start.
#[derive(Resource, Clone, Copy, Debug)]
pub struct FrameClock {
    pub time: f32,
    pub dt: f32,
    pub driven: bool,
    pub free_run: bool,
}

impl Default for FrameClock {
    fn default() -> Self {
        Self {
            time: 0.0,
            dt: 0.0,
            driven: false,
            free_run: false,
        }
    }
}

pub fn tick_clock(time: Res<Time>, state: Res<ParamState>, mut clock: ResMut<FrameClock>) {
    let transport = state
        .transport
        .filter(|t| !clock.free_run && t.received.elapsed() < TRANSPORT_TIMEOUT);
    match transport {
        Some(t) => {
            // dt is the playhead's advance, so a paused or rewound transport
            // yields a zero-length frame and time-scaled effects hold still.
            let dt = if clock.driven {
                (t.time - clock.time).max(0.0)
            } else {
                0.0
            };
            clock.dt = dt.min(0.5);
            clock.time = t.time;
            clock.driven = true;
        }
        None => {
            if clock.driven {
                info!(
                    "no transport for {TRANSPORT_TIMEOUT:?}; clock free-running from {:.2}s",
                    clock.time
                );
            }
            clock.driven = false;
            clock.dt = time.delta_secs();
            clock.time += time.delta_secs();
        }
    }
}

pub fn resolve(
    clock: Res<FrameClock>,
    graph: Res<GraphRes>,
    library: Res<LibraryRes>,
    audio: Res<AudioBandsRes>,
    mut state: ResMut<ParamState>,
) {
    let ctx = ModContext {
        time: clock.time,
        dt: clock.dt,
        audio: **audio,
    };
    let resolved = resolve_params(&graph, &library, &ctx, &BTreeMap::new(), &state.overrides);
    state.resolved = resolved;
}
