//! Node definitions: the closed vocabulary of what a node may declare.
//!
//! A [`NodeDef`] is one fullscreen pass: a WGSL fragment body, named texture
//! inputs, an optional feedback tap (last frame's own output) and a list of
//! typed [`ParamSpec`]s. Everything else — render targets, cameras, wiring,
//! parameter resolution, UI controls, the uniform layout — is derived from it
//! by Zygote. Node code never touches the engine underneath.
//!
//! Definitions come from two places that produce the same data:
//!
//! * a WGSL file with a `//!` header ([`NodeDef::parse_wgsl`]),
//! * a Rust type deriving `NodeParams` (see `zygote_macros`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::params::{ParamKind, ParamSpec, ParamValue};

/// Most texture inputs one node may declare. Fixed so the GPU bind group
/// layout is static; unconnected slots are bound to a black fallback.
pub const MAX_INPUTS: usize = 4;
/// Size of the per-node parameter uniform buffer in bytes.
pub const UNIFORM_BYTES: usize = 256;
/// Name of the implicit feedback input.
pub const PREVIOUS_INPUT: &str = "previous";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDef {
    pub name: String,
    /// Optional inputs may stay unconnected; the shader sees black.
    #[serde(default)]
    pub optional: bool,
}

/// Where a definition came from; informational.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeOrigin {
    Builtin,
    File(PathBuf),
    Rust(String),
}

/// One node kind: a single fullscreen pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeDef {
    pub name: String,
    #[serde(default)]
    pub doc: String,
    #[serde(default)]
    pub inputs: Vec<InputDef>,
    /// Bind last frame's output of this node as `previous`.
    #[serde(default)]
    pub feedback: bool,
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    /// WGSL fragment body (imports allowed, no bindings; those are generated).
    pub source: String,
    pub origin: NodeOrigin,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HeaderError {
    #[error("line {line}: {message}")]
    Syntax { line: usize, message: String },
    #[error("node declares {0} inputs, at most {MAX_INPUTS} are allowed")]
    TooManyInputs(usize),
    #[error("parameters need {0} bytes of uniform space, {UNIFORM_BYTES} available")]
    UniformTooLarge(usize),
    #[error("duplicate parameter `{0}`")]
    DuplicateParam(String),
    #[error("duplicate input `{0}`")]
    DuplicateInput(String),
    #[error("`{0}` is reserved")]
    Reserved(String),
    #[error(
        "`{0}` is both a parameter/input name and an imported item; the shader preprocessor would rename one of them"
    )]
    ImportCollision(String),
    #[error("could not read shader: {0}")]
    Io(String),
}

impl NodeDef {
    /// Parse a WGSL node file. The header is every line starting with `//!`:
    ///
    /// ```text
    /// //! node: warp            optional; defaults to `name`
    /// //! doc: UV remapping     optional
    /// //! input source
    /// //! input displacement optional
    /// //! feedback
    /// //! param amount: float = 0.15 in 0..1 "Displacement strength"
    /// //! param mode: choice = screen [multiply, screen, add, alpha] "Operator"
    /// //! param invert: bool = false
    /// //! param tint: color = #ff8844
    /// //! param offset: vec2 = 0, 0 in -1..1
    /// ```
    pub fn parse_wgsl(name: &str, source: &str, origin: NodeOrigin) -> Result<Self, HeaderError> {
        let mut def = NodeDef {
            name: name.to_owned(),
            doc: String::new(),
            inputs: Vec::new(),
            feedback: false,
            params: Vec::new(),
            source: source.to_owned(),
            origin,
        };
        for (idx, raw) in source.lines().enumerate() {
            let line = idx + 1;
            let Some(rest) = raw.trim_start().strip_prefix("//!") else {
                continue;
            };
            let rest = rest.trim();
            if rest.is_empty() {
                continue;
            }
            let (keyword, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let keyword = keyword.trim_end_matches(':');
            let args = args.trim();
            match keyword {
                "node" => def.name = args.to_owned(),
                "doc" => def.doc = args.to_owned(),
                "feedback" => def.feedback = true,
                "input" => {
                    let mut words = args.split_whitespace();
                    let Some(input_name) = words.next() else {
                        return Err(syntax(line, "input needs a name"));
                    };
                    if input_name == PREVIOUS_INPUT {
                        return Err(HeaderError::Reserved(input_name.to_owned()));
                    }
                    if def.inputs.iter().any(|i| i.name == input_name) {
                        return Err(HeaderError::DuplicateInput(input_name.to_owned()));
                    }
                    let optional = match words.next() {
                        None => false,
                        Some("optional") => true,
                        Some(other) => {
                            return Err(syntax(
                                line,
                                format!("unexpected `{other}` after input name"),
                            ));
                        }
                    };
                    def.inputs.push(InputDef {
                        name: input_name.to_owned(),
                        optional,
                    });
                }
                "param" => {
                    let spec = parse_param(args).map_err(|m| syntax(line, m))?;
                    if def.params.iter().any(|p| p.name == spec.name) {
                        return Err(HeaderError::DuplicateParam(spec.name));
                    }
                    def.params.push(spec);
                }
                other => {
                    return Err(syntax(line, format!("unknown header keyword `{other}`")));
                }
            }
        }
        def.validate()?;
        Ok(def)
    }

    /// Load a `.wgsl` node file; the node name is the file stem.
    pub fn load_file(path: impl AsRef<Path>) -> Result<Self, HeaderError> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path)
            .map_err(|e| HeaderError::Io(format!("{}: {e}", path.display())))?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("node")
            .to_owned();
        Self::parse_wgsl(&name, &source, NodeOrigin::File(path.to_path_buf()))
    }

    pub fn validate(&self) -> Result<(), HeaderError> {
        if self.inputs.len() > MAX_INPUTS {
            return Err(HeaderError::TooManyInputs(self.inputs.len()));
        }
        // naga_oil rewrites every occurrence of an imported identifier, so a
        // parameter or input sharing a name with an import breaks `params.x`.
        let imported = imported_items(&self.source);
        for name in self
            .params
            .iter()
            .map(|p| p.name.as_str())
            .chain(self.inputs.iter().map(|i| i.name.as_str()))
        {
            if imported.iter().any(|item| item == name) {
                return Err(HeaderError::ImportCollision(name.to_owned()));
            }
        }
        let size = self.uniform_layout().size;
        if size > UNIFORM_BYTES {
            return Err(HeaderError::UniformTooLarge(size));
        }
        Ok(())
    }

    pub fn param(&self, name: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.name == name)
    }

    pub fn input_names(&self) -> Vec<&str> {
        self.inputs.iter().map(|i| i.name.as_str()).collect()
    }

    pub fn required_inputs(&self) -> usize {
        self.inputs.iter().take_while(|i| !i.optional).count()
    }

    /// Default value of every parameter.
    pub fn defaults(&self) -> BTreeMap<String, ParamValue> {
        self.params
            .iter()
            .map(|p| (p.name.clone(), p.default.clone()))
            .collect()
    }

    /// std140-compatible layout of the parameter uniform, in declaration order.
    pub fn uniform_layout(&self) -> UniformLayout {
        UniformLayout::for_params(&self.params)
    }

    /// Write parameter values into a uniform buffer according to
    /// [`NodeDef::uniform_layout`]. Missing values fall back to defaults.
    pub fn write_uniform(&self, values: &BTreeMap<String, ParamValue>, out: &mut [u8]) {
        let layout = self.uniform_layout();
        for (spec, field) in self.params.iter().zip(&layout.fields) {
            let value = values.get(&spec.name).unwrap_or(&spec.default);
            let value = spec.conform(value);
            let mut words: [f32; 4] = [0.0; 4];
            let count = match (&value, &spec.ty) {
                (ParamValue::Float(v), _) => {
                    words[0] = *v;
                    1
                }
                (ParamValue::Bool(b), _) => {
                    words[0] = f32::from_bits(u32::from(*b));
                    1
                }
                (ParamValue::Choice(_), ty) => {
                    words[0] = f32::from_bits(ty.choice_index(&value).unwrap_or(0));
                    1
                }
                (ParamValue::Vec2(v), _) => {
                    words[..2].copy_from_slice(v);
                    2
                }
                (ParamValue::Color(c), _) => {
                    words.copy_from_slice(c);
                    4
                }
            };
            for (i, w) in words.iter().take(count).enumerate() {
                let at = field.offset + i * 4;
                if at + 4 <= out.len() {
                    out[at..at + 4].copy_from_slice(&w.to_le_bytes());
                }
            }
        }
    }

    /// The complete WGSL the GPU compiles: hoisted imports, generated
    /// parameter struct, frame uniform, input bindings and sampling helpers,
    /// then the author's body.
    pub fn wgsl_source(&self) -> String {
        let mut imports = Vec::new();
        let mut body = Vec::new();
        for line in self.source.lines() {
            let t = line.trim_start();
            if t.starts_with("#import") || t.starts_with("#define_import_path") {
                imports.push(line.to_owned());
            } else {
                body.push(line.to_owned());
            }
        }
        let mut out = String::new();
        if !imports.iter().any(|l| l.contains("bevy_pbr::forward_io")) {
            out.push_str("#import bevy_pbr::forward_io::VertexOutput\n");
        }
        for import in &imports {
            out.push_str(import);
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&self.uniform_layout().wgsl_struct("ZygoteParams"));
        out.push_str(
            "struct ZygoteFrame {\n    time: f32,\n    dt: f32,\n    aspect: f32,\n    index: f32,\n    connected: u32,\n    _r0: u32,\n    _r1: u32,\n    _r2: u32,\n}\n\n",
        );
        out.push_str(
            "@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ZygoteParams;\n",
        );
        out.push_str(
            "@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> frame: ZygoteFrame;\n",
        );
        for (i, input) in self.inputs.iter().enumerate() {
            let (tex, smp) = input_bindings(i);
            out.push_str(&format!(
                "@group(#{{MATERIAL_BIND_GROUP}}) @binding({tex}) var zygote_in{i}_tex: texture_2d<f32>;\n\
                 @group(#{{MATERIAL_BIND_GROUP}}) @binding({smp}) var zygote_in{i}_smp: sampler;\n\
                 fn {name}(uv: vec2<f32>) -> vec4<f32> {{\n    return textureSample(zygote_in{i}_tex, zygote_in{i}_smp, uv);\n}}\n\
                 fn has_{name}() -> bool {{\n    return (frame.connected & (1u << {i}u)) != 0u;\n}}\n",
                name = input.name
            ));
        }
        if self.feedback {
            let (tex, smp) = previous_bindings();
            out.push_str(&format!(
                "@group(#{{MATERIAL_BIND_GROUP}}) @binding({tex}) var zygote_prev_tex: texture_2d<f32>;\n\
                 @group(#{{MATERIAL_BIND_GROUP}}) @binding({smp}) var zygote_prev_smp: sampler;\n\
                 fn {PREVIOUS_INPUT}(uv: vec2<f32>) -> vec4<f32> {{\n    return textureSample(zygote_prev_tex, zygote_prev_smp, uv);\n}}\n"
            ));
        }
        out.push('\n');
        out.push_str(&body.join("\n"));
        out.push('\n');
        out
    }
}

/// Identifiers brought into scope by `#import` lines:
/// `#import a::b::{x, y}` → `x, y`; `#import a::b::x` → `x`; `#import a::b` → `b`.
fn imported_items(source: &str) -> Vec<String> {
    let mut items = Vec::new();
    for line in source.lines() {
        let Some(rest) = line.trim_start().strip_prefix("#import") else {
            continue;
        };
        let rest = rest.trim();
        if let Some(open) = rest.find('{') {
            let close = rest.rfind('}').unwrap_or(rest.len());
            for item in rest[open + 1..close].split(',') {
                let item = item.trim();
                // `name as alias` binds the alias.
                let bound = item.rsplit(" as ").next().unwrap_or(item).trim();
                if !bound.is_empty() {
                    items.push(bound.to_owned());
                }
            }
        } else if let Some(last) = rest.rsplit("::").next() {
            let bound = last.rsplit(" as ").next().unwrap_or(last).trim();
            if !bound.is_empty() {
                items.push(bound.to_owned());
            }
        }
    }
    items
}

/// Bind group slots of input `i`: `(texture, sampler)`.
pub const fn input_bindings(i: usize) -> (u32, u32) {
    (2 + 2 * i as u32, 3 + 2 * i as u32)
}

/// Bind group slots of the feedback tap.
pub const fn previous_bindings() -> (u32, u32) {
    let base = 2 + 2 * MAX_INPUTS as u32;
    (base, base + 1)
}

fn syntax(line: usize, message: impl Into<String>) -> HeaderError {
    HeaderError::Syntax {
        line,
        message: message.into(),
    }
}

/// `name: type = default [in a..b] [options] ["doc"]`
fn parse_param(args: &str) -> Result<ParamSpec, String> {
    let (mut rest, doc) = split_doc(args);
    let (name, after_name) = rest
        .split_once(':')
        .ok_or("expected `name: type = default`")?;
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("`{name}` is not a valid parameter name"));
    }
    if name == "time" || name == PREVIOUS_INPUT {
        return Err(format!("`{name}` is reserved"));
    }
    let (ty_name, after_ty) = after_name.split_once('=').ok_or("expected `= default`")?;
    let ty_name = ty_name.trim();
    rest = after_ty.trim();

    // Optional `[a, b, c]` options list (choices).
    let mut options: Vec<String> = Vec::new();
    if let Some(open) = rest.find('[') {
        let close = rest.rfind(']').ok_or("unclosed `[`")?;
        options = rest[open + 1..close]
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        rest = format!("{} {}", &rest[..open], &rest[close + 1..])
            .trim()
            .to_owned()
            .leak();
    }
    // Optional `in a..b` range.
    let mut range: Option<(f32, f32)> = None;
    if let Some((before, after)) = rest.split_once(" in ") {
        let (a, b) = after
            .trim()
            .split_once("..")
            .ok_or("range must look like `in min..max`")?;
        range = Some((
            a.trim()
                .parse()
                .map_err(|_| format!("bad range start `{a}`"))?,
            b.trim()
                .parse()
                .map_err(|_| format!("bad range end `{b}`"))?,
        ));
        rest = before.trim();
    }
    let default_text = rest.trim();

    let spec = match ty_name {
        "float" | "f32" => {
            let (min, max) = range.unwrap_or((0.0, 1.0));
            let default: f32 = default_text
                .parse()
                .map_err(|_| format!("bad float default `{default_text}`"))?;
            ParamSpec::float(name, min, max, default, &doc)
        }
        "bool" => {
            let default = match default_text {
                "true" | "on" | "1" => true,
                "false" | "off" | "0" => false,
                other => return Err(format!("bad bool default `{other}`")),
            };
            ParamSpec::bool(name, default, &doc)
        }
        "choice" => {
            if options.is_empty() {
                return Err("choice needs `[option, option, ...]`".into());
            }
            if !options.iter().any(|o| o == default_text) {
                return Err(format!(
                    "default `{default_text}` is not one of the options"
                ));
            }
            let opts: Vec<&str> = options.iter().map(String::as_str).collect();
            ParamSpec::choice(name, &opts, default_text, &doc)
        }
        "color" => {
            let rgba = ParamValue::parse_hex_color(default_text)
                .ok_or_else(|| format!("bad color default `{default_text}` (use #rrggbb)"))?;
            ParamSpec::color(name, rgba, &doc)
        }
        "vec2" => {
            let (min, max) = range.unwrap_or((0.0, 1.0));
            let parts: Vec<&str> = default_text.split(',').map(str::trim).collect();
            if parts.len() != 2 {
                return Err(format!("bad vec2 default `{default_text}` (use `x, y`)"));
            }
            let x: f32 = parts[0]
                .parse()
                .map_err(|_| format!("bad number `{}`", parts[0]))?;
            let y: f32 = parts[1]
                .parse()
                .map_err(|_| format!("bad number `{}`", parts[1]))?;
            ParamSpec::vec2(name, min, max, [x, y], &doc)
        }
        other => return Err(format!("unknown parameter type `{other}`")),
    };
    Ok(spec)
}

/// Split a trailing `"doc string"` off a header line.
fn split_doc(args: &str) -> (&str, String) {
    let trimmed = args.trim_end();
    if trimmed.ends_with('"') {
        if let Some(open) = trimmed[..trimmed.len() - 1].rfind('"') {
            let doc = trimmed[open + 1..trimmed.len() - 1].to_owned();
            return (trimmed[..open].trim_end(), doc);
        }
    }
    (trimmed, String::new())
}

/// One field of the generated uniform struct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldLayout {
    pub name: String,
    pub offset: usize,
    pub size: usize,
    pub wgsl_type: &'static str,
}

/// std140-compatible layout: scalars 4/4, vec2 8/8, vec4 16/16.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UniformLayout {
    pub fields: Vec<FieldLayout>,
    /// Bytes used, rounded up to 16.
    pub size: usize,
}

impl UniformLayout {
    pub fn for_params(params: &[ParamSpec]) -> Self {
        let mut fields = Vec::with_capacity(params.len());
        let mut cursor = 0usize;
        for spec in params {
            let (size, align, wgsl_type) = match spec.ty.kind() {
                ParamKind::Float => (4, 4, "f32"),
                ParamKind::Bool | ParamKind::Choice => (4, 4, "u32"),
                ParamKind::Vec2 => (8, 8, "vec2<f32>"),
                ParamKind::Color => (16, 16, "vec4<f32>"),
            };
            cursor = cursor.div_ceil(align) * align;
            fields.push(FieldLayout {
                name: spec.name.clone(),
                offset: cursor,
                size,
                wgsl_type,
            });
            cursor += size;
        }
        Self {
            fields,
            size: cursor.div_ceil(16) * 16,
        }
    }

    /// WGSL declaration with explicit padding so offsets match exactly.
    pub fn wgsl_struct(&self, name: &str) -> String {
        let mut out = format!("struct {name} {{\n");
        let mut cursor = 0usize;
        let mut pad = 0usize;
        for field in &self.fields {
            while cursor < field.offset {
                out.push_str(&format!("    _pad{pad}: f32,\n"));
                pad += 1;
                cursor += 4;
            }
            out.push_str(&format!("    {}: {},\n", field.name, field.wgsl_type));
            cursor = field.offset + field.size;
        }
        if self.fields.is_empty() {
            out.push_str("    _unused: vec4<f32>,\n");
        }
        out.push_str("}\n\n");
        out
    }
}

/// All node kinds a renderer knows: builtins plus project files plus Rust types.
#[derive(Clone, Debug, Default)]
pub struct NodeLibrary {
    defs: BTreeMap<String, NodeDef>,
}

/// Builtin nodes shipped with Zygote, as `(name, source)`.
pub const BUILTIN_NODES: &[(&str, &str)] = &[
    ("solid", include_str!("../nodes/solid.wgsl")),
    ("test_pattern", include_str!("../nodes/test_pattern.wgsl")),
    ("noise", include_str!("../nodes/noise.wgsl")),
    ("warp", include_str!("../nodes/warp.wgsl")),
    ("blend", include_str!("../nodes/blend.wgsl")),
    ("feedback", include_str!("../nodes/feedback.wgsl")),
    ("color_grade", include_str!("../nodes/color_grade.wgsl")),
];

impl NodeLibrary {
    pub fn empty() -> Self {
        Self::default()
    }

    /// The builtin node set.
    pub fn builtin() -> Self {
        let mut lib = Self::default();
        for (name, source) in BUILTIN_NODES {
            let def = NodeDef::parse_wgsl(name, source, NodeOrigin::Builtin)
                .unwrap_or_else(|e| panic!("builtin node `{name}` has an invalid header: {e}"));
            lib.insert(def);
        }
        lib
    }

    /// Add or replace a definition.
    pub fn insert(&mut self, def: NodeDef) -> Option<NodeDef> {
        self.defs.insert(def.name.clone(), def)
    }

    pub fn get(&self, name: &str) -> Option<&NodeDef> {
        self.defs.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.defs.contains_key(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.defs.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = &NodeDef> {
        self.defs.values()
    }

    /// Load every `*.wgsl` under `dir` (non-recursive). Returns the names
    /// loaded; files with invalid headers are reported, not fatal.
    pub fn load_dir(
        &mut self,
        dir: impl AsRef<Path>,
    ) -> (Vec<String>, Vec<(PathBuf, HeaderError)>) {
        let mut loaded = Vec::new();
        let mut errors = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir.as_ref()) else {
            return (loaded, errors);
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "wgsl"))
            .collect();
        paths.sort();
        for path in paths {
            match NodeDef::load_file(&path) {
                Ok(def) => {
                    loaded.push(def.name.clone());
                    self.insert(def);
                }
                Err(e) => errors.push((path, e)),
            }
        }
        (loaded, errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::ParamType;

    const SAMPLE: &str = r#"
//! node: kaleido
//! doc: Mirror the input into wedges
//! input source
//! input mask optional
//! feedback
//! param segments: float = 6 in 1..24 "Number of wedges"
//! param mode: choice = screen [multiply, screen, add, alpha] "Operator"
//! param invert: bool = false
//! param tint: color = #ff8040
//! param offset: vec2 = 0, 0.5 in -1..1
#import zygote::common::{rotate2}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return source(in.uv) * params.tint;
}
"#;

    #[test]
    fn parses_header() {
        let def = NodeDef::parse_wgsl("file_stem", SAMPLE, NodeOrigin::Builtin).unwrap();
        assert_eq!(def.name, "kaleido");
        assert_eq!(def.doc, "Mirror the input into wedges");
        assert_eq!(def.input_names(), vec!["source", "mask"]);
        assert_eq!(def.required_inputs(), 1);
        assert!(def.feedback);
        assert_eq!(def.params.len(), 5);
        assert_eq!(
            def.params[0],
            ParamSpec::float("segments", 1.0, 24.0, 6.0, "Number of wedges")
        );
        assert_eq!(
            def.params[1].ty,
            ParamType::Choice {
                options: vec![
                    "multiply".into(),
                    "screen".into(),
                    "add".into(),
                    "alpha".into()
                ]
            }
        );
        assert_eq!(def.params[1].default, ParamValue::Choice("screen".into()));
        assert_eq!(def.params[2].default, ParamValue::Bool(false));
        assert_eq!(def.params[3].ty, ParamType::Color);
        assert_eq!(def.params[4].default, ParamValue::Vec2([0.0, 0.5]));
        assert_eq!(
            def.params[4].ty,
            ParamType::Vec2 {
                min: -1.0,
                max: 1.0
            }
        );
    }

    #[test]
    fn layout_follows_std140_rules() {
        let def = NodeDef::parse_wgsl("k", SAMPLE, NodeOrigin::Builtin).unwrap();
        let layout = def.uniform_layout();
        let offsets: Vec<usize> = layout.fields.iter().map(|f| f.offset).collect();
        // f32 @0, u32 @4, u32 @8, vec4 @16 (aligned), vec2 @32
        assert_eq!(offsets, vec![0, 4, 8, 16, 32]);
        assert_eq!(layout.size, 48);
        let wgsl = layout.wgsl_struct("P");
        assert!(wgsl.contains("_pad0: f32"), "{wgsl}");
        assert!(wgsl.contains("tint: vec4<f32>"), "{wgsl}");
    }

    #[test]
    fn writes_uniform_bytes() {
        let def = NodeDef::parse_wgsl("k", SAMPLE, NodeOrigin::Builtin).unwrap();
        let mut values = def.defaults();
        values.insert("segments".into(), ParamValue::Float(100.0)); // clamps to 24
        values.insert("mode".into(), ParamValue::Choice("add".into()));
        values.insert("invert".into(), ParamValue::Bool(true));
        let mut buf = [0u8; UNIFORM_BYTES];
        def.write_uniform(&values, &mut buf);
        let f = |at: usize| f32::from_le_bytes(buf[at..at + 4].try_into().unwrap());
        let u = |at: usize| u32::from_le_bytes(buf[at..at + 4].try_into().unwrap());
        assert_eq!(f(0), 24.0);
        assert_eq!(u(4), 2);
        assert_eq!(u(8), 1);
        assert!((f(16) - 1.0).abs() < 1e-6, "tint.r");
        assert_eq!(f(36), 0.5, "offset.y");
    }

    #[test]
    fn generated_wgsl_declares_bindings_and_helpers() {
        let def = NodeDef::parse_wgsl("k", SAMPLE, NodeOrigin::Builtin).unwrap();
        let wgsl = def.wgsl_source();
        let first_code_line = wgsl.lines().find(|l| !l.trim().is_empty()).unwrap();
        assert!(
            first_code_line.starts_with("#import"),
            "imports hoisted: {first_code_line}"
        );
        assert!(wgsl.contains("#import zygote::common::{rotate2}"));
        assert!(wgsl.contains("struct ZygoteParams"));
        assert!(wgsl.contains("@binding(0) var<uniform> params: ZygoteParams"));
        assert!(wgsl.contains("@binding(2) var zygote_in0_tex"));
        assert!(wgsl.contains("fn source(uv: vec2<f32>) -> vec4<f32>"));
        assert!(wgsl.contains("fn mask(uv: vec2<f32>)"));
        assert!(wgsl.contains("fn has_mask() -> bool"));
        assert!(wgsl.contains("connected: u32"));
        assert!(wgsl.contains("fn previous(uv: vec2<f32>)"));
        assert!(wgsl.contains("@binding(10) var zygote_prev_tex"));
    }

    #[test]
    fn header_errors_are_specific() {
        let bad = "//! param x: float = abc\n";
        assert!(matches!(
            NodeDef::parse_wgsl("b", bad, NodeOrigin::Builtin),
            Err(HeaderError::Syntax { line: 1, .. })
        ));
        let dup = "//! param x: float = 0\n//! param x: float = 1\n";
        assert_eq!(
            NodeDef::parse_wgsl("b", dup, NodeOrigin::Builtin),
            Err(HeaderError::DuplicateParam("x".into()))
        );
        let reserved = "//! input previous\n";
        assert_eq!(
            NodeDef::parse_wgsl("b", reserved, NodeOrigin::Builtin),
            Err(HeaderError::Reserved("previous".into()))
        );
        let many = "//! input a\n//! input b\n//! input c\n//! input d\n//! input e\n";
        assert_eq!(
            NodeDef::parse_wgsl("b", many, NodeOrigin::Builtin),
            Err(HeaderError::TooManyInputs(5))
        );
    }

    #[test]
    fn param_colliding_with_an_import_is_rejected() {
        let src = "//! param palette: float = 0\n#import zygote::common::{luma, palette}\n";
        assert_eq!(
            NodeDef::parse_wgsl("b", src, NodeOrigin::Builtin),
            Err(HeaderError::ImportCollision("palette".into()))
        );
        assert_eq!(
            imported_items("#import zygote::common::{a, b as c}\n#import x::y::z\n#import p::q"),
            vec!["a", "c", "z", "q"]
        );
    }

    #[test]
    fn builtin_library_loads() {
        let lib = NodeLibrary::builtin();
        let names: Vec<&str> = lib.names().collect();
        assert_eq!(
            names,
            vec![
                "blend",
                "color_grade",
                "feedback",
                "noise",
                "solid",
                "test_pattern",
                "warp"
            ]
        );
        let warp = lib.get("warp").unwrap();
        assert_eq!(warp.input_names(), vec!["source", "displacement"]);
        assert_eq!(warp.required_inputs(), 1);
        assert!(lib.get("feedback").unwrap().feedback);
        assert_eq!(
            lib.get("blend").unwrap().param("mode").unwrap().ty.kind(),
            ParamKind::Choice
        );
    }
}
