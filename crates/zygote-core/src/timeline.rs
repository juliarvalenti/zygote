//! Cue timeline.
//!
//! A [`Timeline`] is a list of [`Cue`]s along a time axis. Each cue holds a
//! snapshot of parameter values. Evaluating the timeline at a time `t` yields
//! the parameter values in effect: either a hard cut to the last cue at or
//! before `t`, or an interpolation towards the next cue.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::graph::ParamPath;
use crate::params::ParamValue;

/// How the timeline moves *into* a cue from the previous one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Transition {
    /// Values jump to this cue exactly at its time.
    Cut,
    /// Values are linearly interpolated from the previous cue to this one.
    #[default]
    Interpolate,
}

impl Transition {
    pub fn toggled(self) -> Self {
        match self {
            Transition::Cut => Transition::Interpolate,
            Transition::Interpolate => Transition::Cut,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    pub id: u32,
    /// Position on the time axis in seconds.
    pub time: f32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub transition: Transition,
    pub values: BTreeMap<ParamPath, ParamValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    /// Cues, kept sorted by time.
    pub cues: Vec<Cue>,
    /// Length of the time axis in seconds.
    pub duration: f32,
    #[serde(default = "default_true")]
    pub looping: bool,
    #[serde(default)]
    next_id: u32,
}

fn default_true() -> bool {
    true
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            cues: Vec::new(),
            duration: 8.0,
            looping: true,
            next_id: 1,
        }
    }
}

impl Timeline {
    pub fn new(duration: f32) -> Self {
        Self {
            duration,
            ..Default::default()
        }
    }

    /// Add a cue, returning its id. Cues stay sorted by time.
    pub fn add_cue(
        &mut self,
        time: f32,
        transition: Transition,
        values: BTreeMap<ParamPath, ParamValue>,
    ) -> u32 {
        // Guard against files written before `next_id` existed.
        let max_existing = self.cues.iter().map(|c| c.id).max().unwrap_or(0);
        self.next_id = self.next_id.max(max_existing + 1);
        let id = self.next_id;
        self.next_id += 1;
        self.cues.push(Cue {
            id,
            time: time.clamp(0.0, self.duration),
            label: format!("Cue {id}"),
            transition,
            values,
        });
        self.sort();
        id
    }

    pub fn remove_cue(&mut self, id: u32) -> Option<Cue> {
        let idx = self.cues.iter().position(|c| c.id == id)?;
        Some(self.cues.remove(idx))
    }

    pub fn cue(&self, id: u32) -> Option<&Cue> {
        self.cues.iter().find(|c| c.id == id)
    }

    pub fn cue_mut(&mut self, id: u32) -> Option<&mut Cue> {
        self.cues.iter_mut().find(|c| c.id == id)
    }

    pub fn move_cue(&mut self, id: u32, time: f32) {
        let duration = self.duration;
        if let Some(cue) = self.cue_mut(id) {
            cue.time = time.clamp(0.0, duration);
        }
        self.sort();
    }

    pub fn sort(&mut self) {
        self.cues
            .sort_by(|a, b| a.time.total_cmp(&b.time).then(a.id.cmp(&b.id)));
    }

    /// Wrap or clamp a transport time onto the axis.
    pub fn wrap_time(&self, t: f32) -> f32 {
        if self.duration <= 0.0 {
            return 0.0;
        }
        if self.looping {
            t.rem_euclid(self.duration)
        } else {
            t.clamp(0.0, self.duration)
        }
    }

    /// Index of the last cue with `time <= t`, if any.
    fn cue_index_at(&self, t: f32) -> Option<usize> {
        self.cues.iter().rposition(|c| c.time <= t)
    }

    /// The cue whose values are (or are being approached) at time `t`.
    pub fn active_cue(&self, t: f32) -> Option<&Cue> {
        self.cue_index_at(t)
            .map(|i| &self.cues[i])
            .or_else(|| self.cues.first())
    }

    /// Parameter values in effect at time `t`.
    ///
    /// * Before the first cue: the first cue's values (held).
    /// * Between cues: previous cue values, blended towards the next cue when
    ///   the next cue's transition is [`Transition::Interpolate`] (see
    ///   [`ParamValue::interpolate`] for per-type semantics). Parameters
    ///   present in only one of the two cues are held from that cue.
    /// * After the last cue: the last cue's values.
    pub fn evaluate(&self, t: f32) -> BTreeMap<ParamPath, ParamValue> {
        if self.cues.is_empty() {
            return BTreeMap::new();
        }
        let Some(prev_idx) = self.cue_index_at(t) else {
            return self.cues[0].values.clone();
        };
        let prev = &self.cues[prev_idx];
        let Some(next) = self.cues.get(prev_idx + 1) else {
            return prev.values.clone();
        };
        match next.transition {
            Transition::Cut => prev.values.clone(),
            Transition::Interpolate => {
                let span = (next.time - prev.time).max(1e-6);
                let alpha = ((t - prev.time) / span).clamp(0.0, 1.0);
                let mut out = prev.values.clone();
                for (path, target) in &next.values {
                    let entry = out.entry(path.clone()).or_insert_with(|| target.clone());
                    *entry = entry.interpolate(target, alpha);
                }
                out
            }
        }
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        let mut timeline: Timeline = serde_json::from_str(json)?;
        timeline.sort();
        Ok(timeline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, f32)]) -> BTreeMap<ParamPath, ParamValue> {
        pairs
            .iter()
            .map(|(p, v)| (p.parse::<ParamPath>().unwrap(), ParamValue::Float(*v)))
            .collect()
    }

    fn f(v: &ParamValue) -> f32 {
        v.as_float().unwrap()
    }

    fn two_cue_timeline() -> Timeline {
        let mut tl = Timeline::new(10.0);
        tl.add_cue(
            2.0,
            Transition::Cut,
            values(&[("warp.amount", 0.0), ("fb.decay", 0.5)]),
        );
        tl.add_cue(
            6.0,
            Transition::Interpolate,
            values(&[("warp.amount", 1.0)]),
        );
        tl
    }

    #[test]
    fn holds_first_cue_before_start_and_last_after_end() {
        let tl = two_cue_timeline();
        assert_eq!(
            tl.evaluate(0.0),
            values(&[("warp.amount", 0.0), ("fb.decay", 0.5)])
        );
        assert_eq!(tl.evaluate(9.0), values(&[("warp.amount", 1.0)]));
    }

    #[test]
    fn interpolates_between_cues_and_holds_missing_keys() {
        let tl = two_cue_timeline();
        let mid = tl.evaluate(4.0);
        assert!((f(&mid[&"warp.amount".parse::<ParamPath>().unwrap()]) - 0.5).abs() < 1e-6);
        assert_eq!(f(&mid[&"fb.decay".parse::<ParamPath>().unwrap()]), 0.5);
    }

    #[test]
    fn cut_transition_holds_previous_until_cue_time() {
        let mut tl = two_cue_timeline();
        tl.cue_mut(2).unwrap().transition = Transition::Cut;
        assert_eq!(
            f(&tl.evaluate(5.99)[&"warp.amount".parse::<ParamPath>().unwrap()]),
            0.0
        );
        assert_eq!(
            f(&tl.evaluate(6.0)[&"warp.amount".parse::<ParamPath>().unwrap()]),
            1.0
        );
    }

    #[test]
    fn cues_stay_sorted_and_ids_unique() {
        let mut tl = Timeline::new(10.0);
        let b = tl.add_cue(5.0, Transition::Cut, BTreeMap::new());
        let a = tl.add_cue(1.0, Transition::Cut, BTreeMap::new());
        assert_ne!(a, b);
        assert_eq!(tl.cues.iter().map(|c| c.id).collect::<Vec<_>>(), vec![a, b]);
        tl.move_cue(a, 7.0);
        assert_eq!(tl.cues.iter().map(|c| c.id).collect::<Vec<_>>(), vec![b, a]);
        assert!(tl.remove_cue(b).is_some());
        assert_eq!(tl.cues.len(), 1);
    }

    #[test]
    fn wrap_time_loops() {
        let tl = Timeline::new(4.0);
        assert!((tl.wrap_time(9.0) - 1.0).abs() < 1e-6);
        let mut clamped = tl.clone();
        clamped.looping = false;
        assert_eq!(clamped.wrap_time(9.0), 4.0);
    }

    #[test]
    fn json_roundtrip() {
        let tl = two_cue_timeline();
        let back = Timeline::from_json(&tl.to_json().unwrap()).unwrap();
        assert_eq!(back, tl);
    }
}
