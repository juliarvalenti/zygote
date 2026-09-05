//! Shared, graphics-free data model for Zygote.
//!
//! Everything in this crate is plain data and pure functions: node
//! definitions (the closed vocabulary of what a node may declare), typed
//! parameters, node graphs, modulators, the cue timeline and the wire protocol
//! the timeline UI uses to push parameter values into the renderer. Neither
//! the Bevy/nannou render process nor the gpui-kit UI process needs anything
//! from the other beyond what is defined here.

pub mod graph;
pub mod modulate;
pub mod node_def;
pub mod params;
pub mod protocol;
pub mod resolve;
pub mod timeline;

pub use graph::{
    Graph, GraphError, GraphStructure, InputLink, NodeId, NodeKind, NodeSpec, NodeSummary,
    ParamPath,
};
pub use modulate::{AudioBands, LfoShape, ModContext, Modulation, Modulator};
pub use node_def::{
    BUILTIN_NODES, HeaderError, InputDef, MAX_INPUTS, NodeDef, NodeLibrary, NodeOrigin,
    PREVIOUS_INPUT, UNIFORM_BYTES, UniformLayout, input_bindings, previous_bindings,
};
pub use params::{ParamDescriptor, ParamKind, ParamSpec, ParamType, ParamValue};
pub use protocol::{DEFAULT_PORT, Message, PROTOCOL_VERSION, ParamReceiver, ParamSender};
pub use resolve::{ResolvedParams, resolve_params};
pub use timeline::{Cue, Timeline, Transition};

/// Implemented by `#[derive(NodeParams)]` structs: a typed view of a node's
/// parameters whose declaration is also the node's [`ParamSpec`] list.
pub trait NodeParams: Sized + Default {
    /// Parameter declarations, in field order.
    fn specs() -> Vec<ParamSpec>;
    /// Build from resolved values (missing entries take the field default).
    fn from_values(values: &std::collections::BTreeMap<String, ParamValue>) -> Self;
    /// Current values keyed by parameter name.
    fn to_values(&self) -> std::collections::BTreeMap<String, ParamValue>;
}
