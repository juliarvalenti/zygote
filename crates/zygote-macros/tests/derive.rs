use std::collections::BTreeMap;

use zygote_core::{NodeParams, ParamKind, ParamSpec, ParamType, ParamValue};
use zygote_macros::NodeParams;

#[derive(NodeParams, Clone, Debug, PartialEq)]
struct Kaleido {
    /// Number of mirror wedges
    #[param(default = 6.0, min = 1.0, max = 24.0)]
    segments: f32,
    #[param(default = "screen", options = ["multiply", "screen", "add", "alpha"])]
    mode: String,
    #[param(default = true, doc = "Flip the image")]
    invert: bool,
    #[param(default = "#ff8040")]
    tint: [f32; 4],
    #[param(default = [0.0, 0.5], min = -1.0, max = 1.0)]
    offset: [f32; 2],
    plain: f32,
}

#[test]
fn specs_follow_field_order_and_attributes() {
    let specs = Kaleido::specs();
    assert_eq!(specs.len(), 6);
    assert_eq!(
        specs[0],
        ParamSpec::float("segments", 1.0, 24.0, 6.0, "Number of mirror wedges")
    );
    assert_eq!(specs[1].ty.kind(), ParamKind::Choice);
    assert_eq!(specs[1].default, ParamValue::Choice("screen".into()));
    assert_eq!(specs[2], ParamSpec::bool("invert", true, "Flip the image"));
    assert_eq!(specs[3].ty, ParamType::Color);
    assert!((specs[3].default.as_color().unwrap()[1] - 128.0 / 255.0).abs() < 1e-6);
    assert_eq!(
        specs[4].ty,
        ParamType::Vec2 {
            min: -1.0,
            max: 1.0
        }
    );
    assert_eq!(specs[5], ParamSpec::float("plain", 0.0, 1.0, 0.0, ""));
}

#[test]
fn default_and_values_roundtrip() {
    let d = Kaleido::default();
    assert_eq!(d.segments, 6.0);
    assert_eq!(d.mode, "screen");
    assert!(d.invert);
    let mut values = d.to_values();
    values.insert("segments".into(), ParamValue::Float(12.0));
    values.insert("mode".into(), ParamValue::Choice("add".into()));
    values.insert("plain".into(), ParamValue::Bool(true)); // wrong kind → default
    let k = Kaleido::from_values(&values);
    assert_eq!(k.segments, 12.0);
    assert_eq!(k.mode, "add");
    assert_eq!(k.plain, 0.0);
    assert_eq!(Kaleido::from_values(&BTreeMap::new()), Kaleido::default());
}

#[test]
fn specs_build_a_node_def_layout() {
    let layout = zygote_core::UniformLayout::for_params(&Kaleido::specs());
    let offsets: Vec<usize> = layout.fields.iter().map(|f| f.offset).collect();
    assert_eq!(offsets, vec![0, 4, 8, 16, 32, 40]);
}
