// UV remapping / domain warp. Displaces the source lookup by either an
// external displacement texture (rg → -1..1) or an internal animated fBm field.
#import bevy_pbr::forward_io::VertexOutput
#import zygote::common::{fbm3, rotate2}

struct WarpParams {
    // amount, scale, speed, twist
    p0: vec4<f32>,
    // time, use_displacement, aspect, unused
    p1: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: WarpParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var source_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var source_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var disp_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var disp_smp: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let amount = params.p0.x;
    let scale = params.p0.y;
    let speed = params.p0.z;
    let twist = params.p0.w;
    let time = params.p1.x;
    let use_disp = params.p1.y > 0.5;
    let aspect = params.p1.z;

    var uv = in.uv;

    // Radial twist around the centre, strongest at the edges.
    let centred = (uv - 0.5) * vec2<f32>(aspect, 1.0);
    let radius = length(centred);
    let twisted = rotate2(centred, twist * radius);
    uv = twisted / vec2<f32>(aspect, 1.0) + 0.5;

    var d: vec2<f32>;
    if use_disp {
        d = textureSample(disp_tex, disp_smp, uv).rg * 2.0 - 1.0;
    } else {
        let p = vec3<f32>(uv * vec2<f32>(aspect, 1.0) * scale, time * speed);
        d = vec2<f32>(
            fbm3(p, 4),
            fbm3(p + vec3<f32>(17.3, 9.1, 5.1), 4),
        );
    }
    uv += d * amount;

    return textureSample(source_tex, source_smp, uv);
}
