//! node: feedback
//! doc: Ping-pong feedback; last frame is zoomed, rotated, hue-shifted, decayed and lightened under the source
//! input source
//! feedback
//! param decay: float = 0.92 in 0..1 "Previous frame retention"
//! param zoom: float = 1.01 in 0.9..1.1 "Per-pass zoom of the previous frame"
//! param rotate: float = 0.004 in -0.2..0.2 "Per-pass rotation (radians)"
//! param hue_shift: float = 0.01 in -0.5..0.5 "Per-pass hue rotation (turns)"
//! param mix: float = 1 in 0..1 "How much feedback shows over the source"
#import zygote::common::{rotate2, hue_rotate}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = frame.aspect;
    let zoom = max(params.zoom, 1e-3);
    let centred = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);
    let prev_uv = rotate2(centred / zoom, -params.rotate) / vec2<f32>(aspect, 1.0) + 0.5;

    var prev = previous(prev_uv).rgb;
    prev = clamp(hue_rotate(prev, params.hue_shift) * params.decay, vec3<f32>(0.0), vec3<f32>(1.0));

    let src = source(in.uv).rgb;
    // Lighten composite: trails persist where brighter than the source, and a
    // static source settles back to itself instead of blowing out.
    let out = mix(src, max(src, prev), params.mix);
    return vec4<f32>(out, 1.0);
}
