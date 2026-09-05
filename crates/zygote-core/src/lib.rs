//! Shared, graphics-free data model for Zygote.
//!
//! Everything in this crate is plain data: node graph descriptions, parameter
//! addressing, modulators, the cue timeline and the wire protocol that the
//! timeline UI uses to push parameter values into the renderer. Neither the
//! Bevy/nannou render process nor the gpui-kit UI process needs anything from
//! the other beyond what is defined here.

pub mod graph;
pub mod modulate;
pub mod protocol;
pub mod resolve;
pub mod timeline;

pub use graph::{
    BlendMode, Graph, GraphError, NodeId, NodeKind, NodeSpec, ParamDescriptor, ParamPath, ParamSpec,
};
pub use modulate::{AudioBands, LfoShape, ModContext, Modulation, Modulator};
pub use protocol::{DEFAULT_PORT, Message, ParamReceiver, ParamSender};
pub use resolve::{ResolvedParams, resolve_params};
pub use timeline::{Cue, Timeline, Transition};
