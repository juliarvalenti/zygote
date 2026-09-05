// Two-input compositing: multiply, screen, add, alpha.
#import bevy_pbr::forward_io::VertexOutput

struct BlendParams {
    // mode, mix, unused, unused
    p0: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: BlendParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var a_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var a_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var b_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var b_smp: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let mode = i32(round(params.p0.x));
    let amount = params.p0.y;
    let a = textureSample(a_tex, a_smp, in.uv);
    let b = textureSample(b_tex, b_smp, in.uv);

    var blended: vec3<f32>;
    if mode == 0 {
        blended = a.rgb * b.rgb;
    } else if mode == 1 {
        blended = 1.0 - (1.0 - a.rgb) * (1.0 - b.rgb);
    } else if mode == 2 {
        blended = a.rgb + b.rgb;
    } else {
        blended = mix(a.rgb, b.rgb, b.a);
    }
    return vec4<f32>(mix(a.rgb, blended, amount), 1.0);
}
