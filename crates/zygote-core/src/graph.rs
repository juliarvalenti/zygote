//! Node graph description.
//!
//! A [`Graph`] is an ordered, reconfigurable collection of [`NodeSpec`]s. Each
//! node has a stable string [`NodeId`], a [`NodeKind`] naming what it is, the
//! upstream node per input slot and typed parameter values. Every parameter is
//! addressable by a [`ParamPath`] of the form `node_id.param_name`.
//!
//! Graphs are pure data; what a node kind *means* (inputs, parameters, shader)
//! is looked up in a [`NodeLibrary`].

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::node_def::{NodeDef, NodeLibrary};
use crate::params::{ParamDescriptor, ParamSpec, ParamValue};

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

/// What a node is.
///
/// Two structural kinds are CPU-backed sources the renderer implements
/// itself. Everything else is a [`NodeKind::Shader`]: one fullscreen pass
/// defined by a [`NodeDef`] in the library (builtin, project file, or Rust).
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// Static image (PNG/JPEG) from the asset directory.
    Image { path: String },
    /// Live camera input. Placeholder until a capture backend exists.
    Camera { device: u32 },
    /// A node definition by name: `"warp"`, `"my_project_node"`, …
    Shader { node: String },
}

impl NodeKind {
    pub fn shader(node: impl Into<String>) -> Self {
        NodeKind::Shader { node: node.into() }
    }

    /// The `type` value used in graph files.
    pub fn type_name(&self) -> &str {
        match self {
            NodeKind::Image { .. } => "image",
            NodeKind::Camera { .. } => "camera",
            NodeKind::Shader { node } => node,
        }
    }

    /// Short human readable label.
    pub fn label(&self) -> String {
        match self {
            NodeKind::Image { path } => format!("image · {path}"),
            NodeKind::Camera { device } => format!("camera · device {device}"),
            NodeKind::Shader { node } => node.clone(),
        }
    }

    pub fn is_source(&self) -> bool {
        matches!(self, NodeKind::Image { .. } | NodeKind::Camera { .. })
    }

    /// Definition of this kind, if it is a shader node the library knows.
    pub fn def<'l>(&self, library: &'l NodeLibrary) -> Option<&'l NodeDef> {
        match self {
            NodeKind::Shader { node } => library.get(node),
            _ => None,
        }
    }
}

/// File representation: `{"type": "image", "path": ...}`, `{"type": "camera",
/// "device": 0}` or `{"type": "<node name>"}`.
#[derive(Serialize, Deserialize)]
struct KindRepr {
    #[serde(rename = "type")]
    ty: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device: Option<u32>,
}

impl Serialize for NodeKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let repr = match self {
            NodeKind::Image { path } => KindRepr {
                ty: "image".into(),
                path: Some(path.clone()),
                device: None,
            },
            NodeKind::Camera { device } => KindRepr {
                ty: "camera".into(),
                path: None,
                device: Some(*device),
            },
            NodeKind::Shader { node } => KindRepr {
                ty: node.clone(),
                path: None,
                device: None,
            },
        };
        repr.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for NodeKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let repr = KindRepr::deserialize(deserializer)?;
        Ok(match repr.ty.as_str() {
            "image" => NodeKind::Image {
                path: repr
                    .path
                    .ok_or_else(|| serde::de::Error::custom("image node needs `path`"))?,
            },
            "camera" => NodeKind::Camera {
                device: repr.device.unwrap_or(0),
            },
            other => NodeKind::Shader {
                node: other.to_owned(),
            },
        })
    }
}

/// One node in the graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: NodeId,
    #[serde(flatten)]
    pub kind: NodeKind,
    /// Upstream node per input slot, in slot order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<NodeId>,
    /// Base parameter values. Missing entries fall back to the definition's default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamValue>,
    /// A disabled node passes its first input through unchanged.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
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

    pub fn shader(id: impl Into<NodeId>, node: impl Into<String>) -> Self {
        Self::new(id, NodeKind::shader(node))
    }

    pub fn with_inputs<I, T>(mut self, inputs: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<NodeId>,
    {
        self.inputs = inputs.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_param(mut self, name: &str, value: impl Into<ParamValue>) -> Self {
        self.params.insert(name.to_owned(), value.into());
        self
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GraphError {
    #[error("unknown node `{0}`")]
    UnknownNode(NodeId),
    #[error("node `{0}` uses unknown node kind `{1}`")]
    UnknownKind(NodeId, String),
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
    #[error("node `{node}` accepts {accepted} input(s) but has {given}")]
    TooManyInputs {
        node: NodeId,
        accepted: usize,
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
}

/// UI-facing summary of a graph: what to draw, nothing about how to render it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphStructure {
    pub name: String,
    pub output: NodeId,
    pub nodes: Vec<NodeSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeSummary {
    pub id: NodeId,
    /// Kind label (`warp`, `image · images/x.png`).
    pub kind: String,
    pub doc: String,
    /// One entry per declared input slot.
    pub inputs: Vec<InputLink>,
    pub feedback: bool,
    pub enabled: bool,
    /// Absolute path of a file on this machine that previews the node's
    /// output (an image source's file). The UI may show it; nothing about
    /// how the node renders travels with it. Set only by a renderer that
    /// knows where its assets live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputLink {
    pub name: String,
    pub optional: bool,
    pub from: Option<NodeId>,
}

impl Graph {
    pub fn new(name: impl Into<String>, output: impl Into<NodeId>) -> Self {
        Self {
            name: name.into(),
            nodes: Vec::new(),
            output: output.into(),
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
            NodeSpec::shader("warp", "warp")
                .with_inputs(["image"])
                .with_param("amount", 0.12)
                .with_param("scale", 2.5),
        );
        graph.nodes.push(
            NodeSpec::shader("feedback", "feedback")
                .with_inputs(["warp"])
                .with_param("decay", 0.9)
                .with_param("zoom", 1.012),
        );
        graph
    }

    /// A richer demonstration graph exercising every builtin node.
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
            .push(NodeSpec::shader("noise", "noise").with_param("scale", 2.0));
        graph.nodes.push(
            NodeSpec::shader("warp", "warp")
                .with_inputs(["image", "noise"])
                .with_param("amount", 0.08),
        );
        graph.nodes.push(
            NodeSpec::shader("blend", "blend")
                .with_inputs(["warp", "noise"])
                .with_param("mode", "screen")
                .with_param("mix", 0.25),
        );
        graph
            .nodes
            .push(NodeSpec::shader("feedback", "feedback").with_inputs(["blend"]));
        graph.nodes.push(
            NodeSpec::shader("grade", "color_grade")
                .with_inputs(["feedback"])
                .with_param("saturation", 1.2)
                .with_param("preset", "warm")
                .with_param("palette_mix", 0.3),
        );
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
        Some(removed)
    }

    /// Definition of a node's kind, or an error naming the node.
    pub fn def<'l>(
        &self,
        library: &'l NodeLibrary,
        id: &NodeId,
    ) -> Result<Option<&'l NodeDef>, GraphError> {
        let node = self
            .node(id)
            .ok_or_else(|| GraphError::UnknownNode(id.clone()))?;
        match &node.kind {
            NodeKind::Shader { node: name } => library
                .get(name)
                .map(Some)
                .ok_or_else(|| GraphError::UnknownKind(id.clone(), name.clone())),
            _ => Ok(None),
        }
    }

    pub fn param_spec<'l>(
        &self,
        library: &'l NodeLibrary,
        path: &ParamPath,
    ) -> Result<&'l ParamSpec, GraphError> {
        let def = self
            .def(library, &path.node)?
            .ok_or_else(|| GraphError::UnknownParam(path.node.clone(), path.param.clone()))?;
        def.param(&path.param)
            .ok_or_else(|| GraphError::UnknownParam(path.node.clone(), path.param.clone()))
    }

    /// Set a node's base parameter value (conformed to its type).
    pub fn set_param(
        &mut self,
        library: &NodeLibrary,
        path: &ParamPath,
        value: ParamValue,
    ) -> Result<ParamValue, GraphError> {
        let value = self.param_spec(library, path)?.conform(&value);
        let node = self
            .node_mut(&path.node)
            .ok_or_else(|| GraphError::UnknownNode(path.node.clone()))?;
        node.params.insert(path.param.clone(), value.clone());
        Ok(value)
    }

    /// Base value of a parameter (override or default).
    pub fn param_value(
        &self,
        library: &NodeLibrary,
        path: &ParamPath,
    ) -> Result<ParamValue, GraphError> {
        let spec = self.param_spec(library, path)?;
        let node = self.node(&path.node).expect("param_spec checked the node");
        Ok(node
            .params
            .get(&path.param)
            .map(|v| spec.conform(v))
            .unwrap_or_else(|| spec.default.clone()))
    }

    /// Every addressable parameter in graph order.
    pub fn describe_params(&self, library: &NodeLibrary) -> Vec<ParamDescriptor> {
        let mut out = Vec::new();
        for node in &self.nodes {
            let Some(def) = node.kind.def(library) else {
                continue;
            };
            for spec in &def.params {
                let value = node
                    .params
                    .get(&spec.name)
                    .map(|v| spec.conform(v))
                    .unwrap_or_else(|| spec.default.clone());
                out.push(ParamDescriptor {
                    path: ParamPath::new(node.id.clone(), spec.name.clone()),
                    node_kind: def.name.clone(),
                    ty: spec.ty.clone(),
                    default: spec.default.clone(),
                    value,
                    doc: spec.doc.clone(),
                });
            }
        }
        out
    }

    /// All base parameter values, keyed by path.
    pub fn base_values(&self, library: &NodeLibrary) -> BTreeMap<ParamPath, ParamValue> {
        self.describe_params(library)
            .into_iter()
            .map(|d| (d.path, d.value))
            .collect()
    }

    /// UI-facing structure.
    pub fn structure(&self, library: &NodeLibrary) -> GraphStructure {
        self.structure_with_assets(library, None)
    }

    /// [`Graph::structure`] with image sources resolved against an assets
    /// directory so the UI can preview them.
    pub fn structure_with_assets(
        &self,
        library: &NodeLibrary,
        assets: Option<&std::path::Path>,
    ) -> GraphStructure {
        let nodes = self
            .nodes
            .iter()
            .map(|node| {
                let def = node.kind.def(library);
                let inputs = match def {
                    Some(def) => def
                        .inputs
                        .iter()
                        .enumerate()
                        .map(|(slot, input)| InputLink {
                            name: input.name.clone(),
                            optional: input.optional,
                            from: node.inputs.get(slot).cloned(),
                        })
                        .collect(),
                    None => node
                        .inputs
                        .iter()
                        .enumerate()
                        .map(|(slot, from)| InputLink {
                            name: format!("in{slot}"),
                            optional: false,
                            from: Some(from.clone()),
                        })
                        .collect(),
                };
                let preview = match (&node.kind, assets) {
                    (NodeKind::Image { path }, Some(assets)) => {
                        let file = assets.join(path);
                        let file = std::fs::canonicalize(&file).unwrap_or(file);
                        Some(file.to_string_lossy().into_owned())
                    }
                    _ => None,
                };
                NodeSummary {
                    id: node.id.clone(),
                    kind: node.kind.label(),
                    doc: def.map(|d| d.doc.clone()).unwrap_or_default(),
                    inputs,
                    feedback: def.is_some_and(|d| d.feedback),
                    enabled: node.enabled,
                    preview,
                }
            })
            .collect();
        GraphStructure {
            name: self.name.clone(),
            output: self.output.clone(),
            nodes,
        }
    }

    /// Structural validation against a library: ids unique, kinds known,
    /// inputs resolvable and within the definition's slots, output exists,
    /// no cycles. Feedback is internal to a node, so a valid graph is a DAG.
    pub fn validate(&self, library: &NodeLibrary) -> Result<(), GraphError> {
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
            let (required, accepted) = match self.def(library, &node.id)? {
                Some(def) => (def.required_inputs(), def.inputs.len()),
                None => (0, 0),
            };
            if node.inputs.len() < required {
                return Err(GraphError::MissingInputs {
                    node: node.id.clone(),
                    required,
                    given: node.inputs.len(),
                });
            }
            if node.inputs.len() > accepted {
                return Err(GraphError::TooManyInputs {
                    node: node.id.clone(),
                    accepted,
                    given: node.inputs.len(),
                });
            }
            for name in node.params.keys() {
                self.param_spec(library, &ParamPath::new(node.id.clone(), name.clone()))?;
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
    /// than its deepest input. Empty for cyclic graphs.
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

    fn lib() -> NodeLibrary {
        NodeLibrary::builtin()
    }

    #[test]
    fn param_path_roundtrip() {
        let path: ParamPath = "warp.amount".parse().unwrap();
        assert_eq!(path.node, NodeId::new("warp"));
        assert_eq!(path.param, "amount");
        assert_eq!(serde_json::to_string(&path).unwrap(), "\"warp.amount\"");
        assert!("noparam".parse::<ParamPath>().is_err());
    }

    #[test]
    fn kind_serializes_as_type_field() {
        let spec = NodeSpec::shader("w", "warp").with_inputs(["i"]);
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(json, r#"{"id":"w","type":"warp","inputs":["i"]}"#);
        let img = NodeSpec::new(
            "i",
            NodeKind::Image {
                path: "a.png".into(),
            },
        );
        assert_eq!(
            serde_json::to_string(&img).unwrap(),
            r#"{"id":"i","type":"image","path":"a.png"}"#
        );
        let back: NodeSpec = serde_json::from_str(
            r#"{"id":"k","type":"nodes/kaleido","params":{"mode":"screen","on":true}}"#,
        )
        .unwrap();
        assert_eq!(back.kind, NodeKind::shader("nodes/kaleido"));
        assert_eq!(back.params["mode"], ParamValue::Choice("screen".into()));
        assert_eq!(back.params["on"], ParamValue::Bool(true));
    }

    #[test]
    fn first_pass_is_valid_and_ordered() {
        let graph = Graph::first_pass();
        graph.validate(&lib()).unwrap();
        assert_eq!(
            graph.topo_order().unwrap(),
            vec![
                NodeId::new("image"),
                NodeId::new("warp"),
                NodeId::new("feedback")
            ]
        );
    }

    #[test]
    fn showcase_is_valid_and_describes_typed_params() {
        let graph = Graph::showcase();
        graph.validate(&lib()).unwrap();
        let params = graph.describe_params(&lib());
        let mode = params
            .iter()
            .find(|p| p.path == ParamPath::new("blend", "mode"))
            .unwrap();
        assert_eq!(mode.value, ParamValue::Choice("screen".into()));
        assert!(matches!(mode.ty, crate::params::ParamType::Choice { .. }));
        let depths = graph.depths();
        assert_eq!(depths[&NodeId::new("grade")], 4);
    }

    #[test]
    fn validation_errors() {
        let mut graph = Graph::first_pass();
        graph.node_mut(&NodeId::new("warp")).unwrap().inputs = vec![NodeId::new("feedback")];
        assert!(matches!(graph.validate(&lib()), Err(GraphError::Cycle(_))));

        let mut graph = Graph::first_pass();
        graph.node_mut(&NodeId::new("warp")).unwrap().inputs = vec![NodeId::new("nope")];
        assert!(matches!(
            graph.validate(&lib()),
            Err(GraphError::DanglingInput { .. })
        ));

        let mut graph = Graph::first_pass();
        graph.node_mut(&NodeId::new("warp")).unwrap().kind = NodeKind::shader("bogus");
        assert_eq!(
            graph.validate(&lib()),
            Err(GraphError::UnknownKind(NodeId::new("warp"), "bogus".into()))
        );

        let mut graph = Graph::first_pass();
        graph.node_mut(&NodeId::new("feedback")).unwrap().inputs =
            vec![NodeId::new("warp"), NodeId::new("image")];
        assert!(matches!(
            graph.validate(&lib()),
            Err(GraphError::TooManyInputs { .. })
        ));

        let mut graph = Graph::first_pass();
        graph
            .node_mut(&NodeId::new("warp"))
            .unwrap()
            .params
            .insert("bogus".into(), ParamValue::Float(1.0));
        assert!(matches!(
            graph.validate(&lib()),
            Err(GraphError::UnknownParam(..))
        ));
    }

    #[test]
    fn set_param_conforms() {
        let mut graph = Graph::first_pass();
        let path = ParamPath::new("warp", "amount");
        assert_eq!(
            graph
                .set_param(&lib(), &path, ParamValue::Float(5.0))
                .unwrap(),
            ParamValue::Float(1.0)
        );
        assert_eq!(
            graph.param_value(&lib(), &path).unwrap(),
            ParamValue::Float(1.0)
        );
    }

    #[test]
    fn structure_previews_image_sources_only_with_assets() {
        let graph = Graph::showcase();
        let plain = graph.structure(&lib());
        assert!(plain.nodes.iter().all(|n| n.preview.is_none()));
        let dir = std::path::Path::new("/nonexistent/assets");
        let s = graph.structure_with_assets(&lib(), Some(dir));
        let image = s
            .nodes
            .iter()
            .find(|n| n.kind.starts_with("image"))
            .expect("showcase has an image source");
        assert_eq!(
            image.preview.as_deref(),
            Some("/nonexistent/assets/images/sample.png")
        );
        assert!(
            s.nodes
                .iter()
                .filter(|n| !n.kind.starts_with("image"))
                .all(|n| n.preview.is_none())
        );
        // The field is optional on the wire: old structures still parse.
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("preview"));
        let back: GraphStructure = serde_json::from_str(&json).unwrap();
        assert_eq!(back, plain);
    }

    #[test]
    fn structure_names_slots() {
        let s = Graph::showcase().structure(&lib());
        let warp = s
            .nodes
            .iter()
            .find(|n| n.id == NodeId::new("warp"))
            .unwrap();
        assert_eq!(warp.inputs[0].name, "source");
        assert_eq!(warp.inputs[1].from, Some(NodeId::new("noise")));
        assert!(warp.inputs[1].optional);
        assert!(
            s.nodes
                .iter()
                .find(|n| n.id == NodeId::new("feedback"))
                .unwrap()
                .feedback
        );
    }

    #[test]
    fn json_roundtrip() {
        let graph = Graph::showcase();
        let back = Graph::from_json(&graph.to_json().unwrap()).unwrap();
        assert_eq!(back, graph);
    }
}
