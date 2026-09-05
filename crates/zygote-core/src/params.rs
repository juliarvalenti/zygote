//! Typed parameters.
//!
//! Every controllable value in Zygote is a [`ParamValue`] described by a
//! [`ParamSpec`]. The type decides how the timeline interpolates it, how the UI
//! renders it and how it is packed into a shader uniform.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A parameter's current value.
///
/// Serialises untagged so graph files and cue files stay readable:
/// `0.5`, `true`, `"screen"`, `"#ff8844"`, `[0.1, 0.2]`, `[1, 0.5, 0, 1]`.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Bool(bool),
    /// One of a fixed set of named options (a blend mode, a palette…).
    Choice(String),
    /// Linear RGBA in `0..1`.
    Color([f32; 4]),
    Vec2([f32; 2]),
}

impl ParamValue {
    pub const fn kind(&self) -> ParamKind {
        match self {
            ParamValue::Float(_) => ParamKind::Float,
            ParamValue::Bool(_) => ParamKind::Bool,
            ParamValue::Choice(_) => ParamKind::Choice,
            ParamValue::Color(_) => ParamKind::Color,
            ParamValue::Vec2(_) => ParamKind::Vec2,
        }
    }

    pub fn as_float(&self) -> Option<f32> {
        match self {
            ParamValue::Float(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParamValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_choice(&self) -> Option<&str> {
        match self {
            ParamValue::Choice(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_color(&self) -> Option<[f32; 4]> {
        match self {
            ParamValue::Color(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_vec2(&self) -> Option<[f32; 2]> {
        match self {
            ParamValue::Vec2(v) => Some(*v),
            _ => None,
        }
    }

    /// Interpolate from `self` towards `to` by `t` in `0..=1`.
    ///
    /// * floats, vec2 and colors (linear RGB) lerp,
    /// * bools and choices hold `self` and switch to `to` when `t` reaches 1,
    /// * mismatched kinds hold `self` until `t` reaches 1.
    pub fn interpolate(&self, to: &ParamValue, t: f32) -> ParamValue {
        let t = t.clamp(0.0, 1.0);
        match (self, to) {
            (ParamValue::Float(a), ParamValue::Float(b)) => ParamValue::Float(a + (b - a) * t),
            (ParamValue::Vec2(a), ParamValue::Vec2(b)) => {
                ParamValue::Vec2([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t])
            }
            (ParamValue::Color(a), ParamValue::Color(b)) => {
                let mut out = [0.0; 4];
                for i in 0..4 {
                    out[i] = a[i] + (b[i] - a[i]) * t;
                }
                ParamValue::Color(out)
            }
            _ => {
                if t >= 1.0 {
                    to.clone()
                } else {
                    self.clone()
                }
            }
        }
    }

    /// Parse a `#rrggbb` or `#rrggbbaa` hex color into linear-ish RGBA.
    /// (Treated as already linear; palettes here are artistic, not colorimetric.)
    pub fn parse_hex_color(s: &str) -> Option<[f32; 4]> {
        let hex = s.strip_prefix('#')?;
        let channel = |i: usize| -> Option<f32> {
            let byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
            Some(byte as f32 / 255.0)
        };
        match hex.len() {
            6 => Some([channel(0)?, channel(1)?, channel(2)?, 1.0]),
            8 => Some([channel(0)?, channel(1)?, channel(2)?, channel(3)?]),
            _ => None,
        }
    }

    pub fn to_hex_color(rgba: [f32; 4]) -> String {
        let byte = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round()) as u8;
        if (rgba[3] - 1.0).abs() < 1e-3 {
            format!(
                "#{:02x}{:02x}{:02x}",
                byte(rgba[0]),
                byte(rgba[1]),
                byte(rgba[2])
            )
        } else {
            format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                byte(rgba[0]),
                byte(rgba[1]),
                byte(rgba[2]),
                byte(rgba[3])
            )
        }
    }
}

impl fmt::Display for ParamValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamValue::Float(v) => write!(f, "{v:.3}"),
            ParamValue::Bool(v) => write!(f, "{v}"),
            ParamValue::Choice(v) => f.write_str(v),
            ParamValue::Color(c) => f.write_str(&ParamValue::to_hex_color(*c)),
            ParamValue::Vec2(v) => write!(f, "{:.3}, {:.3}", v[0], v[1]),
        }
    }
}

impl From<f32> for ParamValue {
    fn from(v: f32) -> Self {
        ParamValue::Float(v)
    }
}

impl From<bool> for ParamValue {
    fn from(v: bool) -> Self {
        ParamValue::Bool(v)
    }
}

impl From<&str> for ParamValue {
    fn from(v: &str) -> Self {
        match ParamValue::parse_hex_color(v) {
            Some(c) => ParamValue::Color(c),
            None => ParamValue::Choice(v.to_owned()),
        }
    }
}

impl From<[f32; 4]> for ParamValue {
    fn from(v: [f32; 4]) -> Self {
        ParamValue::Color(v)
    }
}

impl From<[f32; 2]> for ParamValue {
    fn from(v: [f32; 2]) -> Self {
        ParamValue::Vec2(v)
    }
}

/// Untagged wire/file representation.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum ValueRepr {
    Bool(bool),
    Float(f32),
    Text(String),
    Vec2([f32; 2]),
    Color([f32; 4]),
}

impl Serialize for ParamValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let repr = match self {
            ParamValue::Float(v) => ValueRepr::Float(*v),
            ParamValue::Bool(v) => ValueRepr::Bool(*v),
            ParamValue::Choice(v) => ValueRepr::Text(v.clone()),
            ParamValue::Color(c) => ValueRepr::Text(ParamValue::to_hex_color(*c)),
            ParamValue::Vec2(v) => ValueRepr::Vec2(*v),
        };
        repr.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ParamValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match ValueRepr::deserialize(deserializer)? {
            ValueRepr::Bool(v) => ParamValue::Bool(v),
            ValueRepr::Float(v) => ParamValue::Float(v),
            ValueRepr::Text(s) => ParamValue::from(s.as_str()),
            ValueRepr::Vec2(v) => ParamValue::Vec2(v),
            ValueRepr::Color(c) => ParamValue::Color(c),
        })
    }
}

/// The five parameter kinds, without their constraints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    Float,
    Bool,
    Choice,
    Color,
    Vec2,
}

/// Type and constraints of a parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParamType {
    Float { min: f32, max: f32 },
    Bool,
    Choice { options: Vec<String> },
    Color,
    Vec2 { min: f32, max: f32 },
}

impl ParamType {
    pub const fn kind(&self) -> ParamKind {
        match self {
            ParamType::Float { .. } => ParamKind::Float,
            ParamType::Bool => ParamKind::Bool,
            ParamType::Choice { .. } => ParamKind::Choice,
            ParamType::Color => ParamKind::Color,
            ParamType::Vec2 { .. } => ParamKind::Vec2,
        }
    }

    pub fn unit_float() -> Self {
        ParamType::Float { min: 0.0, max: 1.0 }
    }

    /// Bring a value into this type: clamp ranges, snap unknown choices to the
    /// first option, coerce mismatched kinds where it is unambiguous.
    pub fn conform(&self, value: &ParamValue) -> ParamValue {
        match (self, value) {
            (ParamType::Float { min, max }, ParamValue::Float(v)) => {
                ParamValue::Float(v.clamp(min.min(*max), max.max(*min)))
            }
            (ParamType::Float { min, max }, ParamValue::Bool(b)) => {
                ParamValue::Float(if *b { *max } else { *min })
            }
            (ParamType::Bool, ParamValue::Bool(b)) => ParamValue::Bool(*b),
            (ParamType::Bool, ParamValue::Float(v)) => ParamValue::Bool(*v >= 0.5),
            (ParamType::Choice { options }, ParamValue::Choice(c)) => {
                if options.iter().any(|o| o == c) {
                    ParamValue::Choice(c.clone())
                } else {
                    ParamValue::Choice(options.first().cloned().unwrap_or_default())
                }
            }
            (ParamType::Choice { options }, ParamValue::Float(v)) => {
                let idx = (v.round().max(0.0) as usize).min(options.len().saturating_sub(1));
                ParamValue::Choice(options.get(idx).cloned().unwrap_or_default())
            }
            (ParamType::Color, ParamValue::Color(c)) => {
                ParamValue::Color(c.map(|x| x.clamp(0.0, 1.0)))
            }
            (ParamType::Vec2 { min, max }, ParamValue::Vec2(v)) => {
                ParamValue::Vec2(v.map(|x| x.clamp(min.min(*max), max.max(*min))))
            }
            _ => self.default_value(),
        }
    }

    /// A neutral value of this type.
    pub fn default_value(&self) -> ParamValue {
        match self {
            ParamType::Float { min, max } => ParamValue::Float(min.min(*max)),
            ParamType::Bool => ParamValue::Bool(false),
            ParamType::Choice { options } => {
                ParamValue::Choice(options.first().cloned().unwrap_or_default())
            }
            ParamType::Color => ParamValue::Color([1.0; 4]),
            ParamType::Vec2 { .. } => ParamValue::Vec2([0.0; 2]),
        }
    }

    /// Index of a choice value in this type's options.
    pub fn choice_index(&self, value: &ParamValue) -> Option<u32> {
        match (self, value) {
            (ParamType::Choice { options }, ParamValue::Choice(c)) => {
                options.iter().position(|o| o == c).map(|i| i as u32)
            }
            _ => None,
        }
    }
}

/// Declaration of one parameter: the one source of truth the UI, the
/// timeline and the shader uniform are all derived from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamSpec {
    pub name: String,
    #[serde(flatten)]
    pub ty: ParamType,
    pub default: ParamValue,
    #[serde(default)]
    pub doc: String,
}

impl ParamSpec {
    pub fn float(name: &str, min: f32, max: f32, default: f32, doc: &str) -> Self {
        Self {
            name: name.to_owned(),
            ty: ParamType::Float { min, max },
            default: ParamValue::Float(default),
            doc: doc.to_owned(),
        }
    }

    pub fn bool(name: &str, default: bool, doc: &str) -> Self {
        Self {
            name: name.to_owned(),
            ty: ParamType::Bool,
            default: ParamValue::Bool(default),
            doc: doc.to_owned(),
        }
    }

    pub fn choice(name: &str, options: &[&str], default: &str, doc: &str) -> Self {
        Self {
            name: name.to_owned(),
            ty: ParamType::Choice {
                options: options.iter().map(|s| (*s).to_owned()).collect(),
            },
            default: ParamValue::Choice(default.to_owned()),
            doc: doc.to_owned(),
        }
    }

    pub fn color(name: &str, default: [f32; 4], doc: &str) -> Self {
        Self {
            name: name.to_owned(),
            ty: ParamType::Color,
            default: ParamValue::Color(default),
            doc: doc.to_owned(),
        }
    }

    pub fn vec2(name: &str, min: f32, max: f32, default: [f32; 2], doc: &str) -> Self {
        Self {
            name: name.to_owned(),
            ty: ParamType::Vec2 { min, max },
            default: ParamValue::Vec2(default),
            doc: doc.to_owned(),
        }
    }

    pub fn conform(&self, value: &ParamValue) -> ParamValue {
        self.ty.conform(value)
    }
}

/// A parameter together with its full address and current base value.
/// This is what the renderer sends to the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamDescriptor {
    pub path: crate::graph::ParamPath,
    pub node_kind: String,
    #[serde(flatten)]
    pub ty: ParamType,
    pub default: ParamValue,
    pub value: ParamValue,
    #[serde(default)]
    pub doc: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_roundtrip_untagged() {
        for (json, value) in [
            ("0.5", ParamValue::Float(0.5)),
            ("true", ParamValue::Bool(true)),
            ("\"screen\"", ParamValue::Choice("screen".into())),
            (
                "\"#ff8040\"",
                ParamValue::Color([1.0, 128.0 / 255.0, 64.0 / 255.0, 1.0]),
            ),
            ("[0.25,-0.5]", ParamValue::Vec2([0.25, -0.5])),
        ] {
            let parsed: ParamValue = serde_json::from_str(json).unwrap();
            assert_eq!(parsed, value, "{json}");
            let back: ParamValue =
                serde_json::from_str(&serde_json::to_string(&value).unwrap()).unwrap();
            assert_eq!(back, value, "{json}");
        }
    }

    #[test]
    fn interpolation_semantics() {
        let a = ParamValue::Float(0.0);
        let b = ParamValue::Float(2.0);
        assert_eq!(a.interpolate(&b, 0.25), ParamValue::Float(0.5));

        let m = ParamValue::Choice("multiply".into());
        let s = ParamValue::Choice("screen".into());
        assert_eq!(m.interpolate(&s, 0.99), m);
        assert_eq!(m.interpolate(&s, 1.0), s);

        let off = ParamValue::Bool(false);
        assert_eq!(off.interpolate(&ParamValue::Bool(true), 0.5), off);

        let black = ParamValue::Color([0.0, 0.0, 0.0, 1.0]);
        let white = ParamValue::Color([1.0, 1.0, 1.0, 1.0]);
        assert_eq!(
            black.interpolate(&white, 0.5),
            ParamValue::Color([0.5, 0.5, 0.5, 1.0])
        );
    }

    #[test]
    fn conform_clamps_and_coerces() {
        let ty = ParamType::Float { min: 0.0, max: 1.0 };
        assert_eq!(ty.conform(&ParamValue::Float(4.0)), ParamValue::Float(1.0));
        let choice = ParamType::Choice {
            options: vec!["a".into(), "b".into()],
        };
        assert_eq!(
            choice.conform(&ParamValue::Choice("zzz".into())),
            ParamValue::Choice("a".into())
        );
        assert_eq!(
            choice.conform(&ParamValue::Float(1.0)),
            ParamValue::Choice("b".into())
        );
        assert_eq!(
            choice.choice_index(&ParamValue::Choice("b".into())),
            Some(1)
        );
    }
}
