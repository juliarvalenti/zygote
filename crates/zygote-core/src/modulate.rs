//! Modulators: time-based sources (elapsed time, LFOs) and the audio-band hook.
//!
//! A [`Modulation`] binds a [`Modulator`] to a parameter path and adds
//! `depth * sample` on top of whatever base value the parameter currently has
//! (spec default, timeline cue or manual override).

use serde::{Deserialize, Serialize};

use crate::graph::ParamPath;

/// Number of FFT bands exposed to modulators. Fixed so the shape of the hook
/// is stable before real audio analysis exists.
pub const AUDIO_BAND_COUNT: usize = 8;

/// Normalised (0..1) energy per frequency band, low to high.
///
/// Nothing in this repository fills this yet. An audio-analysis stage only has
/// to write into this struct each frame for [`Modulator::AudioBand`] to work.
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
    /// Seconds since the renderer started.
    pub time: f32,
    /// Seconds since the previous frame.
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum Modulator {
    /// Unbounded elapsed time scaled by `rate` (seconds → units).
    Time { rate: f32 },
    /// Periodic oscillator in `-1..1`.
    Lfo {
        rate_hz: f32,
        phase: f32,
        shape: LfoShape,
    },
    /// Energy of one FFT band in `0..1`. Reserved: reads zeros until audio analysis is wired.
    AudioBand { band: usize },
    /// Mean energy over all bands in `0..1`.
    AudioLevel,
}

impl Modulator {
    pub fn sample(&self, ctx: &ModContext) -> f32 {
        match *self {
            Modulator::Time { rate } => ctx.time * rate,
            Modulator::Lfo {
                rate_hz,
                phase,
                shape,
            } => {
                let t = (ctx.time * rate_hz + phase).rem_euclid(1.0);
                match shape {
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
            Modulator::AudioBand { band } => ctx.audio.band(band),
            Modulator::AudioLevel => ctx.audio.level(),
        }
    }
}

/// A modulator routed to a parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Modulation {
    pub target: ParamPath,
    #[serde(flatten)]
    pub source: Modulator,
    /// Multiplier applied to the modulator sample before adding to the base value.
    pub depth: f32,
}

impl Modulation {
    pub fn offset(&self, ctx: &ModContext) -> f32 {
        self.source.sample(ctx) * self.depth
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
        for shape in [
            LfoShape::Sine,
            LfoShape::Triangle,
            LfoShape::Saw,
            LfoShape::Square,
        ] {
            let lfo = Modulator::Lfo {
                rate_hz: 1.0,
                phase: 0.0,
                shape,
            };
            for i in 0..200 {
                let t = i as f32 * 0.037;
                let v = lfo.sample(&ctx(t));
                assert!(
                    (-1.0..=1.0).contains(&v),
                    "{shape:?} out of range at {t}: {v}"
                );
                let w = lfo.sample(&ctx(t + 3.0));
                assert!((v - w).abs() < 1e-3, "{shape:?} not periodic");
            }
        }
    }

    #[test]
    fn audio_hook_reads_zero_until_wired() {
        let m = Modulator::AudioBand { band: 3 };
        assert_eq!(m.sample(&ctx(1.0)), 0.0);
        let mut c = ctx(1.0);
        c.audio.0[3] = 0.75;
        assert_eq!(m.sample(&c), 0.75);
        assert!((Modulator::AudioLevel.sample(&c) - 0.75 / 8.0).abs() < 1e-6);
    }

    #[test]
    fn modulation_serializes_flat() {
        let m = Modulation {
            target: ParamPath::new("warp", "amount"),
            source: Modulator::Lfo {
                rate_hz: 0.5,
                phase: 0.25,
                shape: LfoShape::Triangle,
            },
            depth: 0.1,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"source\":\"lfo\""), "{json}");
        let back: Modulation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }
}
