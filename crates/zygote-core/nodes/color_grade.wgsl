//! node: color_grade
//! doc: Hue, saturation, posterize, plus palette or LUT remapping by luminance
//! input source
//! input lut optional
//! param hue: float = 0 in -0.5..0.5 "Hue rotation (turns)"
//! param saturation: float = 1 in 0..3 "Saturation multiplier"
//! param posterize: float = 0 in 0..32 "Levels per channel, 0 = off"
//! param preset: choice = spectrum [spectrum, cool, warm, ember, neon] "Built-in cosine palette"
//! param palette_mix: float = 0 in 0..1 "Palette remap amount"
//! param lut_mix: float = 0 in 0..1 "LUT remap amount (lut input: horizontal strip indexed by luminance)"
#import zygote::common::{hue_rotate, luma, palette}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var c = source(in.uv).rgb;
    c = hue_rotate(c, params.hue);
    let l = luma(c);
    c = mix(vec3<f32>(l), c, params.saturation);
    if params.posterize >= 1.0 {
        c = floor(c * params.posterize + 0.5) / params.posterize;
    }
    let l2 = clamp(luma(c), 0.0, 1.0);
    c = mix(c, palette(l2, f32(params.preset)), params.palette_mix);
    if has_lut() {
        c = mix(c, lut(vec2<f32>(l2, 0.5)).rgb, params.lut_mix);
    }
    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
