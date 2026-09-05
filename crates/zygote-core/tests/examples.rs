//! The JSON files under `examples/` must stay loadable by the core model.

use zygote_core::{Graph, NodeId, NodeLibrary, ParamValue, Timeline};

const GRAPHS: &[(&str, &str)] = &[
    (
        "first-pass",
        include_str!("../../../examples/graphs/first-pass.json"),
    ),
    (
        "showcase",
        include_str!("../../../examples/graphs/showcase.json"),
    ),
    (
        "generators",
        include_str!("../../../examples/graphs/generators.json"),
    ),
];

#[test]
fn example_graphs_load_and_validate() {
    let lib = NodeLibrary::builtin();
    for (name, json) in GRAPHS {
        let graph = Graph::from_json(json).unwrap_or_else(|e| panic!("{name}: {e}"));
        graph
            .validate(&lib)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            !graph.describe_params(&lib).is_empty(),
            "{name} exposes no parameters"
        );
    }
}

#[test]
fn showcase_example_matches_builtin() {
    let file = Graph::from_json(GRAPHS[1].1).unwrap();
    assert_eq!(file.output, NodeId::new("grade"));
    assert_eq!(file.nodes.len(), Graph::showcase().nodes.len());
    assert_eq!(file.modulations.len(), 2);
}

#[test]
fn generators_example_uses_typed_params() {
    let graph = Graph::from_json(GRAPHS[2].1).unwrap();
    let tint = graph.node(&NodeId::new("tint")).unwrap();
    assert!(matches!(tint.params["color"], ParamValue::Color(_)));
    assert_eq!(
        graph.node(&NodeId::new("mul")).unwrap().params["mode"],
        ParamValue::Choice("multiply".into())
    );
}

#[test]
fn example_timeline_loads() {
    let timeline =
        Timeline::from_json(include_str!("../../../examples/timelines/two-cues.json")).unwrap();
    assert_eq!(timeline.cues.len(), 2);
    let mid = timeline.evaluate(2.0);
    let amount = mid[&"warp.amount".parse().unwrap()].as_float().unwrap();
    assert!(
        (amount - 0.17).abs() < 1e-6,
        "expected midpoint interpolation, got {amount}"
    );
}
