//! Parameter resolution: base values → timeline → manual overrides → modulators.

use std::collections::BTreeMap;

use crate::graph::{Graph, ParamPath};
use crate::modulate::ModContext;
use crate::node_def::NodeLibrary;
use crate::params::ParamValue;

/// Fully resolved parameter values for one frame.
pub type ResolvedParams = BTreeMap<ParamPath, ParamValue>;

/// Combine the graph's base values with timeline values, manual overrides and
/// modulations. Priority (highest first):
///
/// 1. modulations are *added* on top of everything below (float params only),
/// 2. `overrides` (live manual control),
/// 3. `timeline` (programmed cues),
/// 4. the graph's own base values.
///
/// Results are conformed to each parameter's type (clamped, snapped).
/// Unknown paths in `timeline`/`overrides` are ignored.
pub fn resolve_params(
    graph: &Graph,
    library: &NodeLibrary,
    ctx: &ModContext,
    timeline: &BTreeMap<ParamPath, ParamValue>,
    overrides: &BTreeMap<ParamPath, ParamValue>,
) -> ResolvedParams {
    let mut values = graph.base_values(library);
    for (path, value) in timeline.iter().chain(overrides.iter()) {
        if let Some(slot) = values.get_mut(path) {
            *slot = value.clone();
        }
    }
    for modulation in &graph.modulations {
        if let Some(ParamValue::Float(slot)) = values.get_mut(&modulation.target) {
            *slot += modulation.offset(ctx);
        }
    }
    for (path, value) in values.iter_mut() {
        if let Ok(spec) = graph.param_spec(library, path) {
            *value = spec.conform(value);
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modulate::{LfoShape, Modulation, Modulator};

    #[test]
    fn override_beats_timeline_beats_base() {
        let graph = Graph::first_pass();
        let lib = NodeLibrary::builtin();
        let amount = ParamPath::new("warp", "amount");
        let ctx = ModContext::default();

        let base = resolve_params(&graph, &lib, &ctx, &BTreeMap::new(), &BTreeMap::new());
        assert_eq!(base[&amount], ParamValue::Float(0.12));

        let timeline = BTreeMap::from([(amount.clone(), ParamValue::Float(0.5))]);
        let with_timeline = resolve_params(&graph, &lib, &ctx, &timeline, &BTreeMap::new());
        assert_eq!(with_timeline[&amount], ParamValue::Float(0.5));

        let overrides = BTreeMap::from([(amount.clone(), ParamValue::Float(0.9))]);
        let with_override = resolve_params(&graph, &lib, &ctx, &timeline, &overrides);
        assert_eq!(with_override[&amount], ParamValue::Float(0.9));
    }

    #[test]
    fn modulation_adds_and_clamps() {
        let mut graph = Graph::first_pass();
        let lib = NodeLibrary::builtin();
        let amount = ParamPath::new("warp", "amount");
        graph.modulations.push(Modulation {
            target: amount.clone(),
            source: Modulator::Lfo {
                rate_hz: 1.0,
                phase: 0.25,
                shape: LfoShape::Sine,
            },
            depth: 10.0,
        });
        let values = resolve_params(
            &graph,
            &lib,
            &ModContext::default(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        assert_eq!(
            values[&amount],
            ParamValue::Float(1.0),
            "clamped to spec max"
        );
    }

    #[test]
    fn wrong_kind_from_a_client_is_conformed() {
        let graph = Graph::showcase();
        let lib = NodeLibrary::builtin();
        let mode = ParamPath::new("blend", "mode");
        let overrides = BTreeMap::from([(mode.clone(), ParamValue::Float(2.0))]);
        let values = resolve_params(
            &graph,
            &lib,
            &ModContext::default(),
            &BTreeMap::new(),
            &overrides,
        );
        assert_eq!(values[&mode], ParamValue::Choice("add".into()));
    }
}
