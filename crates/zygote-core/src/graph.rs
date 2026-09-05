//! Node graph description.
//!
//! A [`Graph`] is an ordered, reconfigurable collection of [`NodeSpec`]s. Each
//! node has a stable string [`NodeId`], a [`NodeKind`] describing what it does,
//! a list of input node ids (one per input slot) and a set of parameter
//! overrides. Every parameter is addressable by a [`ParamPath`] of the form
//! `node_id.param_name`, which is what the timeline and the wire protocol use.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Stable identifier of a node inside a [`Graph`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub String);

impl NodeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for NodeId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for NodeId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Fully-qualified address of a parameter: `node.param`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamPath {
    pub node: NodeId,
    pub param: String,
}

impl ParamPath {
    pub fn new(node: impl Into<NodeId>, param: impl Into<String>) -> Self {
        Self {
            node: node.into(),
            param: param.into(),
        }
    }
}

impl fmt::Display for ParamPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.node, self.param)
    }
}

impl FromStr for ParamPath {
    type Err = GraphError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Split on the *last* dot so node ids may themselves contain dots.
        let (node, param) = s
            .rsplit_once('.')
            .ok_or_else(|| GraphError::InvalidParamPath(s.to_owned()))?;
        if node.is_empty() || param.is_empty() {
            return Err(GraphError::InvalidParamPath(s.to_owned()));
        }
        Ok(Self::new(node, param))
    }
}

impl Serialize for ParamPath {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ParamPath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Range and default of a single scalar parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamSpec {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Human readable description shown by inspectors / the timeline UI.
    pub doc: &'static str,
}

impl ParamSpec {
    pub const fn new(
        name: &'static str,
        min: f32,
        max: f32,
        default: f32,
        doc: &'static str,
    ) -> Self {
        Self {
            name,
            min,
            max,
            default,
            doc,
        }
    }

    pub fn clamp(&self, value: f32) -> f32 {
        value.clamp(self.min.min(self.max), self.max.max(self.min))
    }
}

/// A parameter together with its full address and current base value.
///
/// This is what the renderer sends to the UI when asked to describe itself, so
/// the UI can build controls without knowing anything about shaders.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamDescriptor {
    pub path: ParamPath,
    pub label: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub value: f32,
    pub doc: String,
}

/// Blend operator used by [`NodeKind::Blend`]. Exposed to the timeline as the
/// numeric `mode` parameter (see [`BlendMode::from_param`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    Multiply,
    Screen,
    Add,
    Alpha,
}

impl BlendMode {
    pub fn to_param(self) -> f32 {
        match self {
            BlendMode::Multiply => 0.0,
            BlendMode::Screen => 1.0,
            BlendMode::Add => 2.0,
            BlendMode::Alpha => 3.0,
        }
    }

    pub fn from_param(value: f32) -> Self {
        match value.round() as i32 {
            i32::MIN..=0 => BlendMode::Multiply,
            1 => BlendMode::Screen,
            2 => BlendMode::Add,
            _ => BlendMode::Alpha,
        }
    }
}

const SOLID_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("r", 0.0, 1.0, 1.0, "Red"),
    ParamSpec::new("g", 0.0, 1.0, 0.5, "Green"),
    ParamSpec::new("b", 0.0, 1.0, 0.1, "Blue"),
];
const TEST_PATTERN_PARAMS: &[ParamSpec] =
    &[ParamSpec::new("scale", 1.0, 32.0, 8.0, "Grid density")];
const NOISE_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("scale", 0.25, 16.0, 3.0, "Spatial frequency"),
    ParamSpec::new("speed", 0.0, 4.0, 0.3, "Evolution speed"),
    ParamSpec::new("octaves", 1.0, 6.0, 4.0, "fBm octaves"),
    ParamSpec::new("contrast", 0.1, 4.0, 1.0, "Output contrast"),
];
const WARP_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("amount", 0.0, 1.0, 0.15, "Displacement strength (UV units)"),
    ParamSpec::new(
        "scale",
        0.25,
        16.0,
        2.0,
        "Noise frequency of the internal displacement field",
    ),
    ParamSpec::new(
        "speed",
        0.0,
        4.0,
        0.25,
        "Displacement field evolution speed",
    ),
    ParamSpec::new("twist", -3.0, 3.0, 0.0, "Radial twist around the center"),
];
const BLEND_PARAMS: &[ParamSpec] = &[
    ParamSpec::new(
        "mode",
        0.0,
        3.0,
        1.0,
        "0 multiply, 1 screen, 2 add, 3 alpha",
    ),
    ParamSpec::new("mix", 0.0, 1.0, 1.0, "Opacity of input B"),
];
const FEEDBACK_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("decay", 0.0, 1.0, 0.92, "Previous frame retention"),
    ParamSpec::new(
        "zoom",
        0.9,
        1.1,
        1.01,
        "Per-pass zoom of the previous frame",
    ),
    ParamSpec::new("rotate", -0.2, 0.2, 0.004, "Per-pass rotation (radians)"),
    ParamSpec::new(
        "hue_shift",
        -0.5,
        0.5,
        0.01,
        "Per-pass hue rotation (turns)",
    ),
    ParamSpec::new(
        "mix",
        0.0,
        1.0,
        1.0,
        "How much feedback is composited over the source",
    ),
];
const COLOR_GRADE_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("hue", -0.5, 0.5, 0.0, "Hue rotation (turns)"),
    ParamSpec::new("saturation", 0.0, 3.0, 1.0, "Saturation multiplier"),
    ParamSpec::new("posterize", 0.0, 32.0, 0.0, "Levels per channel, 0 = off"),
    ParamSpec::new("palette", 0.0, 4.0, 0.0, "Built-in palette index"),
    ParamSpec::new("palette_mix", 0.0, 1.0, 0.0, "Palette remap amount"),
    ParamSpec::new(
        "lut_mix",
        0.0,
        1.0,
        0.0,
        "LUT remap amount (needs a LUT image)",
    ),
];

/// What a node does. Structural configuration (file paths, etc.) lives here;
/// continuously controllable values are parameters (see [`NodeKind::params`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    /// Static image (PNG/JPEG) loaded from the asset directory.
    Image { path: String },
    /// Live camera input. Produces frames only when the renderer is built with
    /// a capture backend; otherwise it outputs a placeholder.
    Camera { device: u32 },
    /// Solid color.
    Solid,
    /// Procedural test pattern (color bars + grid + circle).
    TestPattern,
    /// Procedural fractal gradient-noise field.
    Noise,
    /// UV remapping / domain warp. Inputs: `[source, displacement?]`.
    Warp,
    /// Two-input compositing. Inputs: `[a, b]`.
    Blend { mode: BlendMode },
    /// Ping-pong feedback with a per-pass transform. Inputs: `[source]`.
    Feedback,
    /// Color grading: hue/saturation/posterize plus palette or LUT remap.
    /// Inputs: `[source]`. `lut` is an optional path to a horizontal LUT strip image.
    ColorGrade { lut: Option<String> },
}

impl NodeKind {
    /// Short human readable name.
    pub fn label(&self) -> &'static str {
        match self {
            NodeKind::Image { .. } => "Image",
            NodeKind::Camera { .. } => "Camera",
            NodeKind::Solid => "Solid",
            NodeKind::TestPattern => "Test Pattern",
            NodeKind::Noise => "Noise",
            NodeKind::Warp => "Warp",
            NodeKind::Blend { .. } => "Blend",
            NodeKind::Feedback => "Feedback",
            NodeKind::ColorGrade { .. } => "Color Grade",
        }
    }

    /// Number of texture inputs the node consumes. Optional inputs count.
    pub fn input_count(&self) -> usize {
        match self {
            NodeKind::Image { .. }
            | NodeKind::Camera { .. }
            | NodeKind::Solid
            | NodeKind::TestPattern
            | NodeKind::Noise => 0,
            NodeKind::Warp => 2,
            NodeKind::Blend { .. } => 2,
            NodeKind::Feedback => 1,
            NodeKind::ColorGrade { .. } => 1,
        }
    }

    /// Names of the input slots, in slot order.
    pub fn input_names(&self) -> &'static [&'static str] {
        match self {
            NodeKind::Warp => &["source", "displacement"],
            NodeKind::Blend { .. } => &["a", "b"],
            NodeKind::Feedback | NodeKind::ColorGrade { .. } => &["source"],
            _ => &[],
        }
    }

    /// Number of inputs that must be connected for the node to be valid.
    pub fn required_inputs(&self) -> usize {
        match self {
            NodeKind::Warp => 1,
            other => other.input_count(),
        }
    }

    /// Parameter specs for this kind of node.
    pub fn params(&self) -> &'static [ParamSpec] {
        match self {
            NodeKind::Image { .. } | NodeKind::Camera { .. } => &[],
            NodeKind::Solid => SOLID_PARAMS,
            NodeKind::TestPattern => TEST_PATTERN_PARAMS,
            NodeKind::Noise => NOISE_PARAMS,
            NodeKind::Warp => WARP_PARAMS,
            NodeKind::Blend { .. } => BLEND_PARAMS,
            NodeKind::Feedback => FEEDBACK_PARAMS,
            NodeKind::ColorGrade { .. } => COLOR_GRADE_PARAMS,
        }
    }

    pub fn param(&self, name: &str) -> Option<&'static ParamSpec> {
        self.params().iter().find(|p| p.name == name)
    }
}

/// One node in the graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: NodeId,
    #[serde(flatten)]
    pub kind: NodeKind,
    /// Upstream node per input slot, in slot order.
    #[serde(default)]
    pub inputs: Vec<NodeId>,
    /// Base parameter values. Missing entries fall back to the spec default.
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
    /// A disabled node passes its first input through unchanged.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl NodeSpec {
    pub fn new(id: impl Into<NodeId>, kind: NodeKind) -> Self {
        Self {
            id: id.into(),
            kind,
            inputs: Vec::new(),
            params: BTreeMap::new(),
            enabled: true,
        }
    }

    pub fn with_inputs<I, T>(mut self, inputs: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<NodeId>,
    {
        self.inputs = inputs.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_param(mut self, name: &str, value: f32) -> Self {
        self.params.insert(name.to_owned(), value);
        self
    }

    /// Base value of a parameter (override or spec default).
    pub fn param_value(&self, name: &str) -> Option<f32> {
        let spec = self.kind.param(name)?;
        Some(self.params.get(name).copied().unwrap_or(spec.default))
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GraphError {
    #[error("unknown node `{0}`")]
    UnknownNode(NodeId),
    #[error("node `{0}` has no parameter `{1}`")]
    UnknownParam(NodeId, String),
    #[error("invalid parameter path `{0}` (expected `node.param`)")]
    InvalidParamPath(String),
    #[error("duplicate node id `{0}`")]
    DuplicateNode(NodeId),
    #[error("node `{node}` slot {slot} references unknown node `{target}`")]
    DanglingInput {
        node: NodeId,
        slot: usize,
        target: NodeId,
    },
    #[error("node `{node}` needs {required} input(s) but has {given}")]
    MissingInputs {
        node: NodeId,
        required: usize,
        given: usize,
    },
    #[error("graph contains a cycle involving `{0}`")]
    Cycle(NodeId),
    #[error("graph output `{0}` does not exist")]
    MissingOutput(NodeId),
}

/// A reconfigurable signal chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    #[serde(default)]
    pub name: String,
    pub nodes: Vec<NodeSpec>,
    /// The node whose texture is shown in the window.
    pub output: NodeId,
    /// Time / LFO / audio modulations applied on top of base values.
    #[serde(default)]
    pub modulations: Vec<crate::modulate::Modulation>,
}

impl Graph {
    pub fn new(name: impl Into<String>, output: impl Into<NodeId>) -> Self {
        Self {
            name: name.into(),
            nodes: Vec::new(),
            output: output.into(),
            modulations: Vec::new(),
        }
    }

    /// The first-pass deliverable: image → warp → feedback → output.
    pub fn first_pass() -> Self {
        let mut graph = Graph::new("first-pass", "feedback");
        graph.nodes.push(NodeSpec::new(
            "image",
            NodeKind::Image {
                path: "images/sample.png".to_owned(),
            },
        ));
        graph.nodes.push(
            NodeSpec::new("warp", NodeKind::Warp)
                .with_inputs(["image"])
                .with_param("amount", 0.12)
                .with_param("scale", 2.5),
        );
        graph.nodes.push(
            NodeSpec::new("feedback", NodeKind::Feedback)
                .with_inputs(["warp"])
                .with_param("decay", 0.9)
                .with_param("zoom", 1.012),
        );
        graph
    }

    /// A richer demonstration graph exercising every node kind.
    pub fn showcase() -> Self {
        let mut graph = Graph::new("showcase", "grade");
        graph.nodes.push(NodeSpec::new(
            "image",
            NodeKind::Image {
                path: "images/sample.png".to_owned(),
            },
        ));
        graph
            .nodes
            .push(NodeSpec::new("noise", NodeKind::Noise).with_param("scale", 2.0));
        graph
            .nodes
            .push(NodeSpec::new("warp", NodeKind::Warp).with_inputs(["image", "noise"]));
        graph.nodes.push(
            NodeSpec::new(
                "blend",
                NodeKind::Blend {
                    mode: BlendMode::Screen,
                },
            )
            .with_inputs(["warp", "noise"])
            .with_param("mix", 0.25),
        );
        graph
            .nodes
            .push(NodeSpec::new("feedback", NodeKind::Feedback).with_inputs(["blend"]));
        graph.nodes.push(
            NodeSpec::new("grade", NodeKind::ColorGrade { lut: None })
                .with_inputs(["feedback"])
                .with_param("saturation", 1.2),
        );
        graph.modulations.push(crate::modulate::Modulation {
            target: ParamPath::new("warp", "amount"),
            source: crate::modulate::Modulator::Lfo {
                rate_hz: 0.1,
                phase: 0.0,
                shape: crate::modulate::LfoShape::Sine,
            },
            depth: 0.05,
        });
        graph
    }

    pub fn node(&self, id: &NodeId) -> Option<&NodeSpec> {
        self.nodes.iter().find(|n| &n.id == id)
    }

    pub fn node_mut(&mut self, id: &NodeId) -> Option<&mut NodeSpec> {
        self.nodes.iter_mut().find(|n| &n.id == id)
    }

    pub fn add_node(&mut self, node: NodeSpec) -> Result<(), GraphError> {
        if self.node(&node.id).is_some() {
            return Err(GraphError::DuplicateNode(node.id));
        }
        self.nodes.push(node);
        Ok(())
    }

    pub fn remove_node(&mut self, id: &NodeId) -> Option<NodeSpec> {
        let idx = self.nodes.iter().position(|n| &n.id == id)?;
        let removed = self.nodes.remove(idx);
        for node in &mut self.nodes {
            node.inputs.retain(|input| input != id);
        }
        self.modulations.retain(|m| &m.target.node != id);
        Some(removed)
    }

    /// Set a node's base parameter value (clamped to its spec range).
    pub fn set_param(&mut self, path: &ParamPath, value: f32) -> Result<f32, GraphError> {
        let node = self
            .node_mut(&path.node)
            .ok_or_else(|| GraphError::UnknownNode(path.node.clone()))?;
        let spec = node
            .kind
            .param(&path.param)
            .ok_or_else(|| GraphError::UnknownParam(path.node.clone(), path.param.clone()))?;
        let value = spec.clamp(value);
        node.params.insert(path.param.clone(), value);
        Ok(value)
    }

    /// Base value of a parameter (override or default).
    pub fn param_value(&self, path: &ParamPath) -> Result<f32, GraphError> {
        let node = self
            .node(&path.node)
            .ok_or_else(|| GraphError::UnknownNode(path.node.clone()))?;
        node.param_value(&path.param)
            .ok_or_else(|| GraphError::UnknownParam(path.node.clone(), path.param.clone()))
    }

    pub fn param_spec(&self, path: &ParamPath) -> Option<&'static ParamSpec> {
        self.node(&path.node)?.kind.param(&path.param)
    }

    /// Every addressable parameter in graph order.
    pub fn describe_params(&self) -> Vec<ParamDescriptor> {
        let mut out = Vec::new();
        for node in &self.nodes {
            for spec in node.kind.params() {
                let value = node.params.get(spec.name).copied().unwrap_or(spec.default);
                out.push(ParamDescriptor {
                    path: ParamPath::new(node.id.clone(), spec.name),
                    label: format!("{} · {}", node.id, spec.name),
                    min: spec.min,
                    max: spec.max,
                    default: spec.default,
                    value,
                    doc: spec.doc.to_owned(),
                });
            }
        }
        out
    }

    /// All base parameter values, keyed by path.
    pub fn base_values(&self) -> BTreeMap<ParamPath, f32> {
        self.describe_params()
            .into_iter()
            .map(|d| (d.path, d.value))
            .collect()
    }

    /// Structural validation: ids unique, inputs resolvable and sufficient,
    /// output exists, no cycles. Feedback is internal to the feedback node, so
    /// a valid graph is always a DAG.
    pub fn validate(&self) -> Result<(), GraphError> {
        let mut seen = BTreeSet::new();
        for node in &self.nodes {
            if !seen.insert(&node.id) {
                return Err(GraphError::DuplicateNode(node.id.clone()));
            }
        }
        for node in &self.nodes {
            for (slot, input) in node.inputs.iter().enumerate() {
                if self.node(input).is_none() {
                    return Err(GraphError::DanglingInput {
                        node: node.id.clone(),
                        slot,
                        target: input.clone(),
                    });
                }
            }
            let required = node.kind.required_inputs();
            if node.inputs.len() < required {
                return Err(GraphError::MissingInputs {
                    node: node.id.clone(),
                    required,
                    given: node.inputs.len(),
                });
            }
        }
        if self.node(&self.output).is_none() {
            return Err(GraphError::MissingOutput(self.output.clone()));
        }
        self.topo_order().map(|_| ())
    }

    /// Nodes in dependency order (inputs before consumers). Stable with respect
    /// to declaration order for independent nodes.
    pub fn topo_order(&self) -> Result<Vec<NodeId>, GraphError> {
        let mut indegree: BTreeMap<&NodeId, usize> = BTreeMap::new();
        let mut consumers: BTreeMap<&NodeId, Vec<&NodeId>> = BTreeMap::new();
        for node in &self.nodes {
            indegree.entry(&node.id).or_insert(0);
            for input in &node.inputs {
                *indegree.entry(&node.id).or_insert(0) += 1;
                consumers.entry(input).or_default().push(&node.id);
            }
        }
        let mut ready: VecDeque<&NodeId> = self
            .nodes
            .iter()
            .map(|n| &n.id)
            .filter(|id| indegree.get(id).copied().unwrap_or(0) == 0)
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(id) = ready.pop_front() {
            order.push(id.clone());
            if let Some(next) = consumers.get(id) {
                for consumer in next {
                    let deg = indegree.get_mut(consumer).expect("indegree present");
                    *deg -= 1;
                    if *deg == 0 {
                        ready.push_back(consumer);
                    }
                }
            }
        }
        if order.len() != self.nodes.len() {
            let stuck = self
                .nodes
                .iter()
                .find(|n| !order.contains(&n.id))
                .map(|n| n.id.clone())
                .expect("some node is not ordered");
            return Err(GraphError::Cycle(stuck));
        }
        Ok(order)
    }

    /// Longest-path depth of every node: sources are 0, a node is one deeper
    /// than its deepest input. Useful for laying the graph out in columns.
    /// Requires a valid (acyclic) graph; returns an empty map otherwise.
    pub fn depths(&self) -> BTreeMap<NodeId, usize> {
        let Ok(order) = self.topo_order() else {
            return BTreeMap::new();
        };
        let mut depths: BTreeMap<NodeId, usize> = BTreeMap::new();
        for id in order {
            let node = self.node(&id).expect("ordered node exists");
            let depth = node
                .inputs
                .iter()
                .filter_map(|input| depths.get(input))
                .map(|d| d + 1)
                .max()
                .unwrap_or(0);
            depths.insert(id, depth);
        }
        depths
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_path_roundtrip() {
        let path: ParamPath = "warp.amount".parse().unwrap();
        assert_eq!(path.node, NodeId::new("warp"));
        assert_eq!(path.param, "amount");
        assert_eq!(path.to_string(), "warp.amount");
        let json = serde_json::to_string(&path).unwrap();
        assert_eq!(json, "\"warp.amount\"");
        let back: ParamPath = serde_json::from_str(&json).unwrap();
        assert_eq!(back, path);
        assert!("noparam".parse::<ParamPath>().is_err());
    }

    #[test]
    fn first_pass_is_valid_and_ordered() {
        let graph = Graph::first_pass();
        graph.validate().unwrap();
        let order = graph.topo_order().unwrap();
        assert_eq!(
            order,
            vec![
                NodeId::new("image"),
                NodeId::new("warp"),
                NodeId::new("feedback")
            ]
        );
    }

    #[test]
    fn showcase_is_valid() {
        Graph::showcase().validate().unwrap();
    }

    #[test]
    fn detects_cycles_and_dangling_inputs() {
        let mut graph = Graph::first_pass();
        graph.node_mut(&NodeId::new("image")).unwrap().inputs = vec![NodeId::new("feedback")];
        assert!(matches!(graph.validate(), Err(GraphError::Cycle(_))));

        let mut graph = Graph::first_pass();
        graph.node_mut(&NodeId::new("warp")).unwrap().inputs = vec![NodeId::new("nope")];
        assert!(matches!(
            graph.validate(),
            Err(GraphError::DanglingInput { .. })
        ));
    }

    #[test]
    fn set_param_clamps_and_reports_unknowns() {
        let mut graph = Graph::first_pass();
        let path = ParamPath::new("warp", "amount");
        assert_eq!(graph.set_param(&path, 5.0).unwrap(), 1.0);
        assert_eq!(graph.param_value(&path).unwrap(), 1.0);
        assert_eq!(
            graph.set_param(&ParamPath::new("warp", "bogus"), 1.0),
            Err(GraphError::UnknownParam(
                NodeId::new("warp"),
                "bogus".into()
            ))
        );
    }

    #[test]
    fn depths_follow_longest_path() {
        let graph = Graph::showcase();
        let depths = graph.depths();
        assert_eq!(depths[&NodeId::new("image")], 0);
        assert_eq!(depths[&NodeId::new("noise")], 0);
        assert_eq!(depths[&NodeId::new("warp")], 1);
        assert_eq!(depths[&NodeId::new("blend")], 2);
        assert_eq!(depths[&NodeId::new("feedback")], 3);
        assert_eq!(depths[&NodeId::new("grade")], 4);
        assert_eq!(NodeKind::Warp.input_names(), &["source", "displacement"]);
    }

    #[test]
    fn json_roundtrip() {
        let graph = Graph::showcase();
        let json = graph.to_json().unwrap();
        let back = Graph::from_json(&json).unwrap();
        assert_eq!(back, graph);
    }
}
