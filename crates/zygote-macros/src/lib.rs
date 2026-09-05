//! `#[derive(NodeParams)]`: one Rust struct is the whole parameter
//! declaration of a node.
//!
//! ```ignore
//! #[derive(NodeParams, Clone, Debug)]
//! pub struct Kaleido {
//!     /// Number of mirror wedges
//!     #[param(default = 6.0, min = 1.0, max = 24.0)]
//!     pub segments: f32,
//!     #[param(default = "screen", options = ["multiply", "screen", "add", "alpha"])]
//!     pub mode: String,
//!     #[param(default = true)]
//!     pub invert: bool,
//!     #[param(default = "#ff8040")]
//!     pub tint: [f32; 4],
//!     #[param(default = [0.0, 0.5], min = -1.0, max = 1.0)]
//!     pub offset: [f32; 2],
//! }
//! ```
//!
//! Field types decide the parameter kind: `f32` → float, `i32` → int, `bool` → bool,
//! `String` → choice (needs `options`), `[f32; 4]` → color, `[f32; 2]` → vec2.
//! Doc comments become the parameter's description. Ranges default to `0..1`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Data, DeriveInput, Expr, ExprArray, ExprLit, Fields, Lit, Meta, Type, parse_macro_input,
};

enum Kind {
    Float,
    Int,
    Bool,
    Choice,
    Color,
    Vec2,
}

struct Field {
    ident: syn::Ident,
    name: String,
    kind: Kind,
    default: Option<Expr>,
    min: Option<Expr>,
    max: Option<Expr>,
    options: Vec<String>,
    doc: String,
}

#[proc_macro_derive(NodeParams, attributes(param))]
pub fn derive_node_params(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Path to `zygote_core` from the calling crate: the crate itself, a direct
/// dependency, or re-exported through `zygote_render::core`.
fn core_path() -> proc_macro2::TokenStream {
    use proc_macro_crate::{FoundCrate, crate_name};
    match crate_name("zygote-core") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
            quote!(::#ident)
        }
        Err(_) => match crate_name("zygote-render") {
            Ok(FoundCrate::Name(name)) => {
                let ident = syn::Ident::new(&name, proc_macro2::Span::call_site());
                quote!(::#ident::core)
            }
            _ => quote!(::zygote_core),
        },
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ident = &input.ident;
    let core = core_path();
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            ident,
            "NodeParams only supports structs",
        ));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new_spanned(
            ident,
            "NodeParams needs named fields",
        ));
    };

    let fields: Vec<Field> = named
        .named
        .iter()
        .map(parse_field)
        .collect::<syn::Result<_>>()?;

    let specs = fields
        .iter()
        .map(|f| spec_tokens(f, &core))
        .collect::<syn::Result<Vec<_>>>()?;
    let from_values = fields
        .iter()
        .map(from_value_tokens)
        .collect::<syn::Result<Vec<_>>>()?;
    let to_values = fields.iter().map(|f| to_value_tokens(f, &core));
    let defaults = fields
        .iter()
        .map(default_field_tokens)
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        impl #core::NodeParams for #ident {
            fn specs() -> ::std::vec::Vec<#core::ParamSpec> {
                ::std::vec![#(#specs),*]
            }

            fn from_values(
                values: &::std::collections::BTreeMap<::std::string::String, #core::ParamValue>,
            ) -> Self {
                let defaults = <Self as ::std::default::Default>::default();
                Self { #(#from_values),* }
            }

            fn to_values(&self) -> ::std::collections::BTreeMap<::std::string::String, #core::ParamValue> {
                let mut out = ::std::collections::BTreeMap::new();
                #(#to_values)*
                out
            }
        }

        impl ::std::default::Default for #ident {
            fn default() -> Self {
                Self { #(#defaults),* }
            }
        }
    })
}

fn parse_field(field: &syn::Field) -> syn::Result<Field> {
    let ident = field.ident.clone().expect("named field");
    let kind = kind_of(&field.ty)?;
    let mut out = Field {
        name: ident.to_string(),
        ident,
        kind,
        default: None,
        min: None,
        max: None,
        options: Vec::new(),
        doc: String::new(),
    };
    for attr in &field.attrs {
        if attr.path().is_ident("doc") {
            if let Meta::NameValue(nv) = &attr.meta
                && let Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) = &nv.value
            {
                if !out.doc.is_empty() {
                    out.doc.push(' ');
                }
                out.doc.push_str(s.value().trim());
            }
            continue;
        }
        if !attr.path().is_ident("param") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            let key = meta
                .path
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            let value: Expr = meta.value()?.parse()?;
            match key.as_str() {
                "default" => out.default = Some(value),
                "min" => out.min = Some(value),
                "max" => out.max = Some(value),
                "name" => {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Str(s), ..
                    }) = &value
                    {
                        out.name = s.value();
                    }
                }
                "doc" => {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Str(s), ..
                    }) = &value
                    {
                        out.doc = s.value();
                    }
                }
                "options" => {
                    let Expr::Array(ExprArray { elems, .. }) = &value else {
                        return Err(meta.error("options must be an array of string literals"));
                    };
                    for elem in elems {
                        if let Expr::Lit(ExprLit {
                            lit: Lit::Str(s), ..
                        }) = elem
                        {
                            out.options.push(s.value());
                        } else {
                            return Err(meta.error("options must be string literals"));
                        }
                    }
                }
                other => return Err(meta.error(format!("unknown param attribute `{other}`"))),
            }
            Ok(())
        })?;
    }
    if matches!(out.kind, Kind::Choice) && out.options.is_empty() {
        return Err(syn::Error::new_spanned(
            &field.ty,
            "String parameters are choices and need #[param(options = [...])]",
        ));
    }
    Ok(out)
}

fn kind_of(ty: &Type) -> syn::Result<Kind> {
    match ty {
        Type::Path(p) if p.path.is_ident("f32") => Ok(Kind::Float),
        Type::Path(p) if p.path.is_ident("i32") => Ok(Kind::Int),
        Type::Path(p) if p.path.is_ident("bool") => Ok(Kind::Bool),
        Type::Path(p) if p.path.is_ident("String") => Ok(Kind::Choice),
        Type::Array(arr) => {
            let Type::Path(elem) = &*arr.elem else {
                return Err(syn::Error::new_spanned(ty, "unsupported array element"));
            };
            if !elem.path.is_ident("f32") {
                return Err(syn::Error::new_spanned(
                    ty,
                    "arrays must be [f32; 2] or [f32; 4]",
                ));
            }
            match &arr.len {
                Expr::Lit(ExprLit {
                    lit: Lit::Int(n), ..
                }) if n.base10_parse::<usize>()? == 4 => Ok(Kind::Color),
                Expr::Lit(ExprLit {
                    lit: Lit::Int(n), ..
                }) if n.base10_parse::<usize>()? == 2 => Ok(Kind::Vec2),
                _ => Err(syn::Error::new_spanned(
                    ty,
                    "arrays must be [f32; 2] (vec2) or [f32; 4] (color)",
                )),
            }
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "supported parameter types: f32, bool, String (choice), [f32; 4] (color), [f32; 2] (vec2)",
        )),
    }
}

/// The field's default as a Rust expression of the field type.
fn default_expr(field: &Field) -> syn::Result<proc_macro2::TokenStream> {
    Ok(match (&field.kind, &field.default) {
        (Kind::Float, Some(d)) => quote!((#d) as f32),
        (Kind::Float, None) => quote!(0.0_f32),
        (Kind::Int, Some(d)) => quote!((#d) as i32),
        (Kind::Int, None) => quote!(0_i32),
        (Kind::Bool, Some(d)) => quote!(#d),
        (Kind::Bool, None) => quote!(false),
        (Kind::Choice, Some(d)) => quote!(::std::string::String::from(#d)),
        (Kind::Choice, None) => {
            let first = &field.options[0];
            quote!(::std::string::String::from(#first))
        }
        (
            Kind::Color,
            Some(Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            })),
        ) => {
            let rgba = parse_hex(&s.value()).ok_or_else(|| {
                syn::Error::new_spanned(s, "color default must be #rrggbb or #rrggbbaa")
            })?;
            let [r, g, b, a] = rgba;
            quote!([#r, #g, #b, #a])
        }
        (Kind::Color, Some(d)) => quote!(#d),
        (Kind::Color, None) => quote!([1.0_f32, 1.0, 1.0, 1.0]),
        (Kind::Vec2, Some(d)) => quote!(#d),
        (Kind::Vec2, None) => quote!([0.0_f32, 0.0]),
    })
}

fn parse_hex(s: &str) -> Option<[f32; 4]> {
    let hex = s.strip_prefix('#')?;
    let ch = |i: usize| -> Option<f32> {
        Some(u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()? as f32 / 255.0)
    };
    match hex.len() {
        6 => Some([ch(0)?, ch(1)?, ch(2)?, 1.0]),
        8 => Some([ch(0)?, ch(1)?, ch(2)?, ch(3)?]),
        _ => None,
    }
}

fn spec_tokens(
    field: &Field,
    core: &proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    let name = &field.name;
    let doc = &field.doc;
    let default = default_expr(field)?;
    let min = field
        .min
        .clone()
        .map(|e| quote!((#e) as f32))
        .unwrap_or(quote!(0.0_f32));
    let max = field
        .max
        .clone()
        .map(|e| quote!((#e) as f32))
        .unwrap_or(quote!(1.0_f32));
    Ok(match field.kind {
        Kind::Float => quote!(#core::ParamSpec::float(#name, #min, #max, #default, #doc)),
        Kind::Int => {
            let imin = field
                .min
                .clone()
                .map(|e| quote!((#e) as i32))
                .unwrap_or(quote!(0_i32));
            let imax = field
                .max
                .clone()
                .map(|e| quote!((#e) as i32))
                .unwrap_or(quote!(100_i32));
            quote!(#core::ParamSpec::int(#name, #imin, #imax, #default, #doc))
        }
        Kind::Bool => quote!(#core::ParamSpec::bool(#name, #default, #doc)),
        Kind::Choice => {
            let options = &field.options;
            quote!({
                let default: ::std::string::String = #default;
                #core::ParamSpec::choice(#name, &[#(#options),*], &default, #doc)
            })
        }
        Kind::Color => quote!(#core::ParamSpec::color(#name, #default, #doc)),
        Kind::Vec2 => quote!(#core::ParamSpec::vec2(#name, #min, #max, #default, #doc)),
    })
}

fn from_value_tokens(field: &Field) -> syn::Result<proc_macro2::TokenStream> {
    let ident = &field.ident;
    let name = &field.name;
    let accessor = match field.kind {
        Kind::Float => quote!(as_float()),
        Kind::Int => quote!(as_int()),
        Kind::Bool => quote!(as_bool()),
        Kind::Choice => quote!(as_choice().map(::std::borrow::ToOwned::to_owned)),
        Kind::Color => quote!(as_color()),
        Kind::Vec2 => quote!(as_vec2()),
    };
    Ok(quote! {
        #ident: values
            .get(#name)
            .and_then(|v| v.#accessor)
            .unwrap_or(defaults.#ident)
    })
}

fn to_value_tokens(field: &Field, core: &proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let ident = &field.ident;
    let name = &field.name;
    let value = match field.kind {
        Kind::Float => quote!(#core::ParamValue::Float(self.#ident)),
        Kind::Int => quote!(#core::ParamValue::Int(self.#ident)),
        Kind::Bool => quote!(#core::ParamValue::Bool(self.#ident)),
        Kind::Choice => quote!(#core::ParamValue::Choice(self.#ident.clone())),
        Kind::Color => quote!(#core::ParamValue::Color(self.#ident)),
        Kind::Vec2 => quote!(#core::ParamValue::Vec2(self.#ident)),
    };
    quote!(out.insert(::std::string::String::from(#name), #value);)
}

fn default_field_tokens(field: &Field) -> syn::Result<proc_macro2::TokenStream> {
    let ident = &field.ident;
    let default = default_expr(field)?;
    Ok(quote!(#ident: #default))
}
