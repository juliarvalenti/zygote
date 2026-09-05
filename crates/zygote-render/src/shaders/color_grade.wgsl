// Hue / saturation / posterize plus palette or LUT remapping.
#import bevy_pbr::forward_io::VertexOutput
#import zygote::common::{hue_rotate, luma, palette}

struct ColorGradeParams {
    // hue, saturation, posterize, palette
    p0: vec4<f32>,
    // palette_mix, lut_mix, use_lut, unused
    p1: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ColorGradeParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var source_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var source_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var lut_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var lut_smp: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let hue = params.p0.x;
    let saturation = params.p0.y;
    let posterize = params.p0.z;
    let palette_index = params.p0.w;
    let palette_mix = params.p1.x;
    let lut_mix = params.p1.y * params.p1.z;

    var c = textureSample(source_tex, source_smp, in.uv).rgb;
    c = hue_rotate(c, hue);
    let l = luma(c);
    c = mix(vec3<f32>(l), c, saturation);
    if posterize >= 1.0 {
        c = floor(c * posterize + 0.5) / posterize;
    }
    let l2 = clamp(luma(c), 0.0, 1.0);
    c = mix(c, palette(l2, palette_index), palette_mix);
    let lut = textureSample(lut_tex, lut_smp, vec2<f32>(l2, 0.5)).rgb;
    c = mix(c, lut, lut_mix);
    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
