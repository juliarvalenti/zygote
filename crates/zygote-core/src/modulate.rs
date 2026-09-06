//! Modulation: LFOs, ADSR envelopes and the audio hook, routed to parameters.
//!
//! A show's [`Modulation`] is a list of shared [`ModSource`]s and a list of
//! [`Assignment`]s. Each assignment adds `depth * source_value` on top of a
//! parameter's resolved value (base → cue → live), so the slider always means
//! "the center" and depth 0 is exactly the slider. Every source is a pure
//! function of transport time (plus the gate log for envelopes), so pausing
//! freezes it, scrubbing rewinds it, and the UI and renderer agree exactly.

use serde::{Deserialize, Serialize};

use crate::graph::ParamPath;

/// Number of FFT bands exposed to modulators. Fixed so the shape of the hook
/// is stable before real audio analysis exists.
pub const AUDIO_BAND_COUNT: usize = 8;

/// Normalized (0..1) energy per frequency band, low to high.
///
/// Nothing in this repository fills this yet. An audio-analysis stage only has
/// to write into this struct each frame for audio sources to work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioBands(pub [f32; AUDIO_BAND_COUNT]);

impl AudioBands {
    pub fn band(&self, index: usize) -> f32 {
        self.0.get(index).copied().unwrap_or(0.0)
    }

    /// Mean energy across all bands.
    pub fn level(&self) -> f32 {
        self.0.iter().sum::<f32>() / AUDIO_BAND_COUNT as f32
    }
}

/// Per-frame context modulators sample from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModContext {
    /// Transport seconds.
    pub time: f32,
    /// Seconds since the previous frame (0 while paused).
    pub dt: f32,
    pub audio: AudioBands,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
}

impl LfoShape {
    pub const ALL: [LfoShape; 4] = [
        LfoShape::Sine,
        LfoShape::Triangle,
        LfoShape::Saw,
        LfoShape::Square,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LfoShape::Sine => "sine",
            LfoShape::Triangle => "tri",
            LfoShape::Saw => "saw",
            LfoShape::Square => "square",
        }
    }

    /// Bipolar waveform at normalized phase `t` in `0..1`.
    pub fn sample(self, t: f32) -> f32 {
        let t = t.rem_euclid(1.0);
        match self {
            LfoShape::Sine => (t * std::f32::consts::TAU).sin(),
            LfoShape::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
            LfoShape::Saw => t * 2.0 - 1.0,
            LfoShape::Square => {
                if t < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }
}

/// Attack / decay / sustain / release, all in seconds except `sustain` (level).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Adsr {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Default for Adsr {
    fn default() -> Self {
        Self {
            attack: 0.05,
            decay: 0.4,
            sustain: 0.6,
            release: 0.8,
        }
    }
}

impl Adsr {
    /// Level `held` seconds after gate-on while the gate is still held.
    pub fn held_level(&self, held: f32) -> f32 {
        let held = held.max(0.0);
        let a = self.attack.max(1e-4);
        if held < a {
            return held / a;
        }
        let d = self.decay.max(1e-4);
        let into_decay = held - a;
        if into_decay < d {
            let k = into_decay / d;
            return 1.0 + (self.sustain - 1.0) * k;
        }
        self.sustain
    }

    /// Level `since_off` seconds after the gate was released from `level_at_off`.
    pub fn released_level(&self, level_at_off: f32, since_off: f32) -> f32 {
        let r = self.release.max(1e-4);
        let k = (since_off / r).clamp(0.0, 1.0);
        level_at_off * (1.0 - k)
    }
}

/// What a source computes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceKind {
    /// Bipolar oscillator in `-1..1`.
    Lfo {
        rate_hz: f32,
        phase: f32,
        shape: LfoShape,
    },
    /// Unipolar envelope in `0..1`, driven by the named trigger's gate.
    Envelope { adsr: Adsr, trigger: String },
    /// Energy of one FFT band in `0..1`. Reads zeros until audio analysis is wired.
    AudioBand { band: usize },
    /// Mean energy over all bands in `0..1`.
    AudioLevel,
}

/// A shared modulation source with a stable id (`lfo1`, `env_a`, …).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModSource {
    pub id: String,
    #[serde(flatten)]
    pub kind: SourceKind,
}

impl ModSource {
    pub fn lfo(id: &str, rate_hz: f32, shape: LfoShape) -> Self {
        Self {
            id: id.to_owned(),
            kind: SourceKind::Lfo {
                rate_hz,
                phase: 0.0,
                shape,
            },
        }
    }

    pub fn envelope(id: &str, trigger: &str) -> Self {
        Self {
            id: id.to_owned(),
            kind: SourceKind::Envelope {
                adsr: Adsr::default(),
                trigger: trigger.to_owned(),
            },
        }
    }

    /// Short human label.
    pub fn label(&self) -> String {
        match &self.kind {
            SourceKind::Lfo { rate_hz, shape, .. } => {
                format!("{} · {} {rate_hz:.2} Hz", self.id, shape.label())
            }
            SourceKind::Envelope { trigger, .. } => format!("{} · env ← {trigger}", self.id),
            SourceKind::AudioBand { band } => format!("{} · audio band {band}", self.id),
            SourceKind::AudioLevel => format!("{} · audio level", self.id),
        }
    }

    /// Current value of the source. LFOs are bipolar, everything else unipolar.
    pub fn sample(&self, ctx: &ModContext, gates: &GateLog) -> f32 {
        match &self.kind {
            SourceKind::Lfo {
                rate_hz,
                phase,
                shape,
            } => shape.sample(ctx.time * rate_hz + phase),
            SourceKind::Envelope { adsr, trigger } => gates.envelope_level(trigger, adsr, ctx.time),
            SourceKind::AudioBand { band } => ctx.audio.band(*band),
            SourceKind::AudioLevel => ctx.audio.level(),
        }
    }
}

/// A source routed to a parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub target: ParamPath,
    pub source: String,
    /// Multiplier applied to the source value before adding to the resolved
    /// value, in the parameter's own units.
    pub depth: f32,
}

/// A gate change for a named trigger, at transport time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateEvent {
    pub trigger: String,
    pub on: bool,
    pub time: f32,
}

/// History of gate events, kept sorted by time. Envelopes are evaluated from
/// it as a pure function of time so scrubbing is deterministic.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GateLog {
    events: Vec<GateEvent>,
}

/// Events older than this are dropped when the log is trimmed.
const GATE_LOG_MAX: usize = 4096;

impl GateLog {
    pub fn push(&mut self, event: GateEvent) {
        let at = self.events.partition_point(|e| e.time <= event.time);
        self.events.insert(at, event);
        if self.events.len() > GATE_LOG_MAX {
            let excess = self.events.len() - GATE_LOG_MAX;
            self.events.drain(..excess);
        }
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn events(&self) -> &[GateEvent] {
        &self.events
    }

    /// Is the trigger's gate held at time `t`?
    pub fn is_on(&self, trigger: &str, t: f32) -> bool {
        self.events
            .iter()
            .rev()
            .find(|e| e.trigger == trigger && e.time <= t)
            .is_some_and(|e| e.on)
    }

    /// Envelope level for `trigger` at time `t`.
    pub fn envelope_level(&self, trigger: &str, adsr: &Adsr, t: f32) -> f32 {
        // Most recent gate-on at or before t.
        let Some(on_idx) = self
            .events
            .iter()
            .rposition(|e| e.trigger == trigger && e.on && e.time <= t)
        else {
            return 0.0;
        };
        let on_time = self.events[on_idx].time;
        // First gate-off after that gate-on, at or before t.
        let off = self.events[on_idx + 1..]
            .iter()
            .find(|e| e.trigger == trigger && !e.on && e.time <= t);
        match off {
            None => adsr.held_level(t - on_time),
            Some(off) => {
                let level_at_off = adsr.held_level(off.time - on_time);
                adsr.released_level(level_at_off, t - off.time)
            }
        }
    }
}

/// A show's modulation setup.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Modulation {
    #[serde(default)]
    pub sources: Vec<ModSource>,
    #[serde(default)]
    pub assignments: Vec<Assignment>,
}

impl Modulation {
    pub fn source(&self, id: &str) -> Option<&ModSource> {
        self.sources.iter().find(|s| s.id == id)
    }

    pub fn source_mut(&mut self, id: &str) -> Option<&mut ModSource> {
        self.sources.iter_mut().find(|s| s.id == id)
    }

    /// Next free id with the given prefix (`lfo1`, `lfo2`, …).
    pub fn next_id(&self, prefix: &str) -> String {
        let mut n = 1;
        loop {
            let id = format!("{prefix}{n}");
            if self.source(&id).is_none() {
                return id;
            }
            n += 1;
        }
    }

    pub fn remove_source(&mut self, id: &str) {
        self.sources.retain(|s| s.id != id);
        self.assignments.retain(|a| a.source != id);
    }

    pub fn assignment(&self, target: &ParamPath) -> Option<&Assignment> {
        self.assignments.iter().find(|a| &a.target == target)
    }

    /// Set (or clear with `None`) the single assignment of a parameter.
    pub fn assign(&mut self, target: &ParamPath, source: Option<&str>, depth: f32) {
        self.assignments.retain(|a| &a.target != target);
        if let Some(source) = source {
            self.assignments.push(Assignment {
                target: target.clone(),
                source: source.to_owned(),
                depth,
            });
        }
    }

    /// Current value of every source, keyed by id.
    pub fn source_values(&self, ctx: &ModContext, gates: &GateLog) -> Vec<(String, f32)> {
        self.sources
            .iter()
            .map(|s| (s.id.clone(), s.sample(ctx, gates)))
            .collect()
    }

    /// Offset to add to each assigned parameter.
    pub fn offsets(&self, ctx: &ModContext, gates: &GateLog) -> Vec<(ParamPath, f32)> {
        let values = self.source_values(ctx, gates);
        self.assignments
            .iter()
            .filter_map(|a| {
                let value = values.iter().find(|(id, _)| id == &a.source)?.1;
                Some((a.target.clone(), value * a.depth))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(time: f32) -> ModContext {
        ModContext {
            time,
            dt: 1.0 / 60.0,
            audio: AudioBands::default(),
        }
    }

    #[test]
    fn lfo_shapes_are_bounded_and_periodic() {
        for shape in LfoShape::ALL {
            for i in 0..200 {
                let t = i as f32 * 0.037;
                let v = shape.sample(t);
                assert!(
                    (-1.0..=1.0).contains(&v),
                    "{shape:?} out of range at {t}: {v}"
                );
                assert!(
                    (v - shape.sample(t + 3.0)).abs() < 1e-3,
                    "{shape:?} not periodic"
                );
            }
        }
    }

    #[test]
    fn envelope_follows_gate_log() {
        let adsr = Adsr {
            attack: 1.0,
            decay: 1.0,
            sustain: 0.5,
            release: 1.0,
        };
        let mut gates = GateLog::default();
        assert_eq!(
            gates.envelope_level("hit", &adsr, 5.0),
            0.0,
            "never triggered"
        );
        gates.push(GateEvent {
            trigger: "hit".into(),
            on: true,
            time: 10.0,
        });
        assert_eq!(
            gates.envelope_level("hit", &adsr, 9.0),
            0.0,
            "before the gate"
        );
        assert!(
            (gates.envelope_level("hit", &adsr, 10.5) - 0.5).abs() < 1e-6,
            "mid attack"
        );
        assert!(
            (gates.envelope_level("hit", &adsr, 11.0) - 1.0).abs() < 1e-6,
            "peak"
        );
        assert!(
            (gates.envelope_level("hit", &adsr, 11.5) - 0.75).abs() < 1e-6,
            "mid decay"
        );
        assert!(
            (gates.envelope_level("hit", &adsr, 14.0) - 0.5).abs() < 1e-6,
            "sustain"
        );
        gates.push(GateEvent {
            trigger: "hit".into(),
            on: false,
            time: 14.0,
        });
        assert!(
            (gates.envelope_level("hit", &adsr, 14.5) - 0.25).abs() < 1e-6,
            "mid release"
        );
        assert_eq!(gates.envelope_level("hit", &adsr, 16.0), 0.0, "released");
        // Scrubbing back before the release still sees sustain: pure function of time.
        assert!((gates.envelope_level("hit", &adsr, 13.0) - 0.5).abs() < 1e-6);
        // Releasing during the attack releases from the reached level.
        let mut short = GateLog::default();
        short.push(GateEvent {
            trigger: "hit".into(),
            on: true,
            time: 0.0,
        });
        short.push(GateEvent {
            trigger: "hit".into(),
            on: false,
            time: 0.5,
        });
        assert!((short.envelope_level("hit", &adsr, 1.0) - 0.25).abs() < 1e-6);
        assert!(short.is_on("hit", 0.25) && !short.is_on("hit", 0.75));
    }

    #[test]
    fn offsets_scale_by_depth_and_ignore_unknown_sources() {
        let mut m = Modulation::default();
        m.sources.push(ModSource::lfo("lfo1", 1.0, LfoShape::Sine));
        m.assign(&ParamPath::new("warp", "amount"), Some("lfo1"), 0.5);
        m.assign(&ParamPath::new("warp", "twist"), Some("ghost"), 9.0);
        // Sine peak at a quarter period.
        let offsets = m.offsets(&ctx(0.25), &GateLog::default());
        assert_eq!(offsets.len(), 1);
        assert_eq!(offsets[0].0, ParamPath::new("warp", "amount"));
        assert!((offsets[0].1 - 0.5).abs() < 1e-5);
        m.assign(&ParamPath::new("warp", "amount"), None, 0.0);
        assert!(
            m.assignments
                .iter()
                .all(|a| a.target != ParamPath::new("warp", "amount"))
        );
        assert_eq!(m.next_id("lfo"), "lfo2");
        m.remove_source("lfo1");
        assert_eq!(m.next_id("lfo"), "lfo1");
    }

    #[test]
    fn audio_hook_reads_zero_until_wired() {
        let s = ModSource {
            id: "a".into(),
            kind: SourceKind::AudioBand { band: 3 },
        };
        assert_eq!(s.sample(&ctx(1.0), &GateLog::default()), 0.0);
        let mut c = ctx(1.0);
        c.audio.0[3] = 0.75;
        assert_eq!(s.sample(&c, &GateLog::default()), 0.75);
    }

    #[test]
    fn serializes_flat() {
        let mut m = Modulation::default();
        m.sources
            .push(ModSource::lfo("lfo1", 0.5, LfoShape::Triangle));
        m.sources.push(ModSource::envelope("env1", "hit"));
        m.assign(&ParamPath::new("warp", "amount"), Some("lfo1"), 0.1);
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"kind\":\"lfo\""), "{json}");
        assert!(json.contains("\"trigger\":\"hit\""), "{json}");
        let back: Modulation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }
}
