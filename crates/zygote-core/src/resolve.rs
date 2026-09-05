//! Parameter resolution: base values → timeline → manual overrides → modulators.

use std::collections::BTreeMap;

use crate::graph::{Graph, ParamPath};
use crate::modulate::ModContext;

/// Fully resolved parameter values for one frame.
pub type ResolvedParams = BTreeMap<ParamPath, f32>;

/// Combine the graph's base values with timeline values, manual overrides and
/// modulations. Priority (highest first):
///
/// 1. modulations are *added* on top of everything below,
/// 2. `overrides` (live manual control),
/// 3. `timeline` (programmed cues),
/// 4. the graph's own base values.
///
/// Results are clamped to each parameter's spec range. Unknown paths in
/// `timeline`/`overrides` are ignored.
pub fn resolve_params(
    graph: &Graph,
    ctx: &ModContext,
    timeline: &BTreeMap<ParamPath, f32>,
    overrides: &BTreeMap<ParamPath, f32>,
) -> ResolvedParams {
    let mut values = graph.base_values();
    for (path, value) in timeline.iter().chain(overrides.iter()) {
        if let Some(slot) = values.get_mut(path) {
            *slot = *value;
        }
    }
    for modulation in &graph.modulations {
        if let Some(slot) = values.get_mut(&modulation.target) {
            *slot += modulation.offset(ctx);
        }
    }
    for (path, value) in values.iter_mut() {
        if let Some(spec) = graph.param_spec(path) {
            *value = spec.clamp(*value);
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
        let amount = ParamPath::new("warp", "amount");
        let ctx = ModContext::default();

        let base = resolve_params(&graph, &ctx, &BTreeMap::new(), &BTreeMap::new());
        assert_eq!(base[&amount], 0.12);

        let timeline = BTreeMap::from([(amount.clone(), 0.5)]);
        let with_timeline = resolve_params(&graph, &ctx, &timeline, &BTreeMap::new());
        assert_eq!(with_timeline[&amount], 0.5);

        let overrides = BTreeMap::from([(amount.clone(), 0.9)]);
        let with_override = resolve_params(&graph, &ctx, &timeline, &overrides);
        assert_eq!(with_override[&amount], 0.9);
    }

    #[test]
    fn modulation_adds_and_clamps() {
        let mut graph = Graph::first_pass();
        let amount = ParamPath::new("warp", "amount");
        graph.modulations.push(Modulation {
            target: amount.clone(),
            source: Modulator::Lfo {
                rate_hz: 1.0,
                phase: 0.25, // sine peak at t = 0
                shape: LfoShape::Sine,
            },
            depth: 10.0,
        });
        let ctx = ModContext::default();
        let values = resolve_params(&graph, &ctx, &BTreeMap::new(), &BTreeMap::new());
        assert_eq!(values[&amount], 1.0, "clamped to spec max");
    }
}
