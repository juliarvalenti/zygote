//! CPU texture sources: type-erased [`RustSource`](crate::RustSource)
//! instances whose pixels are uploaded into their node's output image.

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::*;
use zygote_core::{NodeId, NodeParams, ParamPath, ParamValue};

use crate::RustSource;
use crate::nodes::Runtime;
use crate::params::{FrameClock, ParamState};

/// Frame data handed to a CPU source.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameInfo {
    /// Transport seconds.
    pub time: f32,
    /// Seconds since the previous frame; 0 while paused.
    pub dt: f32,
    pub index: u64,
}

pub trait ErasedSource: Send + Sync {
    fn update(
        &mut self,
        values: &BTreeMap<String, ParamValue>,
        frame: &FrameInfo,
        pixels: &mut [u8],
    );
}

struct Typed<S: RustSource>(S);

impl<S: RustSource> ErasedSource for Typed<S> {
    fn update(
        &mut self,
        values: &BTreeMap<String, ParamValue>,
        frame: &FrameInfo,
        pixels: &mut [u8],
    ) {
        let params = S::Params::from_values(values);
        self.0.update(&params, frame, pixels);
    }
}

/// Creates a fresh source instance per graph node.
#[derive(Clone)]
pub struct SourceFactory(Arc<dyn Fn() -> Box<dyn ErasedSource> + Send + Sync>);

impl SourceFactory {
    pub fn of<S: RustSource>() -> Self {
        Self(Arc::new(|| Box::new(Typed(S::new()))))
    }

    pub fn create(&self) -> Box<dyn ErasedSource> {
        (self.0)()
    }
}

#[derive(Resource, Clone, Default)]
pub struct SourceFactories(BTreeMap<String, SourceFactory>);

impl SourceFactories {
    pub fn insert(&mut self, name: &str, factory: SourceFactory) {
        self.0.insert(name.to_owned(), factory);
    }

    pub fn get(&self, name: &str) -> Option<&SourceFactory> {
        self.0.get(name)
    }
}

/// One live CPU source bound to a node.
pub struct LiveSource {
    pub node: NodeId,
    pub def: String,
    pub image: Handle<Image>,
    pub source: Box<dyn ErasedSource>,
}

#[derive(Resource, Default)]
pub struct LiveSources(pub Vec<LiveSource>);

/// Run every CPU source for this frame and upload its pixels.
pub fn update_sources(
    mut live: ResMut<LiveSources>,
    runtime: Res<Runtime>,
    library: Res<crate::plugin::LibraryRes>,
    state: Res<ParamState>,
    clock: Res<FrameClock>,
    mut images: ResMut<Assets<Image>>,
) {
    let frame = FrameInfo {
        time: clock.time,
        dt: clock.dt,
        index: runtime.frame,
    };
    for live in live.0.iter_mut() {
        let Some(def) = library.get(&live.def) else {
            continue;
        };
        let values: BTreeMap<String, ParamValue> = def
            .params
            .iter()
            .filter_map(|spec| {
                state
                    .resolved
                    .get(&ParamPath::new(live.node.clone(), spec.name.clone()))
                    .map(|v| (spec.name.clone(), v.clone()))
            })
            .collect();
        let Some(mut image) = images.get_mut(&live.image) else {
            continue;
        };
        let Some(data) = image.data.as_mut() else {
            continue;
        };
        live.source.update(&values, &frame, data);
    }
}
