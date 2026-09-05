// Ping-pong feedback. `prev_tex` is last frame's output of this node; it is
// zoomed, rotated, hue shifted and decayed, then screened under the source.
#import bevy_pbr::forward_io::VertexOutput
#import zygote::common::{rotate2, hue_rotate}

struct FeedbackParams {
    // decay, zoom, rotate, hue_shift
    p0: vec4<f32>,
    // mix, time, aspect, unused
    p1: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: FeedbackParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var source_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var source_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var prev_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var prev_smp: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let decay = params.p0.x;
    let zoom = max(params.p0.y, 1e-3);
    let angle = params.p0.z;
    let hue_shift = params.p0.w;
    let amount = params.p1.x;
    let aspect = params.p1.z;

    // Transform the lookup into last frame: zoom > 1 grows the image each pass.
    let centred = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);
    let prev_uv = rotate2(centred / zoom, -angle) / vec2<f32>(aspect, 1.0) + 0.5;

    var prev = textureSample(prev_tex, prev_smp, prev_uv).rgb;
    prev = clamp(hue_rotate(prev, hue_shift) * decay, vec3<f32>(0.0), vec3<f32>(1.0));

    let src = textureSample(source_tex, source_smp, in.uv).rgb;
    // Lighten composite: trails persist where they are brighter than the
    // source, but a static source settles back to itself instead of blowing
    // out to white the way an additive or screen composite would.
    let trails = max(src, prev);
    let out = mix(src, trails, amount);
    return vec4<f32>(out, 1.0);
}
