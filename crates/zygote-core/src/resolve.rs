//! Parameter resolution: base values → timeline → manual overrides → modulation.

use std::collections::BTreeMap;

use crate::graph::{Graph, ParamPath};
use crate::node_def::NodeLibrary;
use crate::params::ParamValue;

/// Fully resolved parameter values for one frame.
pub type ResolvedParams = BTreeMap<ParamPath, ParamValue>;

/// Combine the graph's base values with timeline values, manual overrides and
/// modulation offsets. Priority (highest first):
///
/// 1. `offsets` are *added* on top of everything below (float and int params),
/// 2. `overrides` (live manual control),
/// 3. `timeline` (programmed cues),
/// 4. the graph's own base values.
///
/// Results are conformed to each parameter's type (clamped, snapped).
/// Unknown paths are ignored.
pub fn resolve_params(
    graph: &Graph,
    library: &NodeLibrary,
    timeline: &BTreeMap<ParamPath, ParamValue>,
    overrides: &BTreeMap<ParamPath, ParamValue>,
    offsets: &[(ParamPath, f32)],
) -> ResolvedParams {
    let mut values = graph.base_values(library);
    for (path, value) in timeline.iter().chain(overrides.iter()) {
        if let Some(slot) = values.get_mut(path) {
            *slot = value.clone();
        }
    }
    for (path, offset) in offsets {
        if let Some(slot) = values.get_mut(path) {
            *slot = apply_offset(slot, *offset);
        }
    }
    for (path, value) in values.iter_mut() {
        if let Ok(spec) = graph.param_spec(library, path) {
            *value = spec.conform(value);
        }
    }
    values
}

/// Add a modulation offset to a value; only numeric kinds respond.
pub fn apply_offset(value: &ParamValue, offset: f32) -> ParamValue {
    match value {
        ParamValue::Float(v) => ParamValue::Float(v + offset),
        ParamValue::Int(v) => ParamValue::Int((*v as f32 + offset).round() as i32),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modulate::{GateLog, LfoShape, ModContext, ModSource, Modulation};

    #[test]
    fn override_beats_timeline_beats_base() {
        let graph = Graph::first_pass();
        let lib = NodeLibrary::builtin();
        let amount = ParamPath::new("warp", "amount");

        let base = resolve_params(&graph, &lib, &BTreeMap::new(), &BTreeMap::new(), &[]);
        assert_eq!(base[&amount], ParamValue::Float(0.12));

        let timeline = BTreeMap::from([(amount.clone(), ParamValue::Float(0.5))]);
        let with_timeline = resolve_params(&graph, &lib, &timeline, &BTreeMap::new(), &[]);
        assert_eq!(with_timeline[&amount], ParamValue::Float(0.5));

        let overrides = BTreeMap::from([(amount.clone(), ParamValue::Float(0.9))]);
        let with_override = resolve_params(&graph, &lib, &timeline, &overrides, &[]);
        assert_eq!(with_override[&amount], ParamValue::Float(0.9));
    }

    #[test]
    fn modulation_adds_and_clamps() {
        let graph = Graph::first_pass();
        let lib = NodeLibrary::builtin();
        let amount = ParamPath::new("warp", "amount");
        let mut m = Modulation::default();
        m.sources.push(ModSource::lfo("lfo1", 1.0, LfoShape::Sine));
        m.assign(&amount, Some("lfo1"), 10.0);
        let ctx = ModContext {
            time: 0.25,
            ..Default::default()
        };
        let offsets = m.offsets(&ctx, &GateLog::default());
        let values = resolve_params(&graph, &lib, &BTreeMap::new(), &BTreeMap::new(), &offsets);
        assert_eq!(
            values[&amount],
            ParamValue::Float(1.0),
            "clamped to spec max"
        );
    }

    #[test]
    fn ints_step_and_choices_ignore_offsets() {
        let graph = Graph::showcase();
        let lib = NodeLibrary::builtin();
        let octaves = ParamPath::new("noise", "octaves");
        let mode = ParamPath::new("blend", "mode");
        let offsets = vec![(octaves.clone(), 1.4), (mode.clone(), 3.0)];
        let values = resolve_params(&graph, &lib, &BTreeMap::new(), &BTreeMap::new(), &offsets);
        assert_eq!(values[&octaves], ParamValue::Int(5));
        assert_eq!(values[&mode], ParamValue::Choice("screen".into()));
    }

    #[test]
    fn wrong_kind_from_a_client_is_conformed() {
        let graph = Graph::showcase();
        let lib = NodeLibrary::builtin();
        let mode = ParamPath::new("blend", "mode");
        let overrides = BTreeMap::from([(mode.clone(), ParamValue::Float(2.0))]);
        let values = resolve_params(&graph, &lib, &BTreeMap::new(), &overrides, &[]);
        assert_eq!(values[&mode], ParamValue::Choice("add".into()));
    }
}
