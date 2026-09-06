//! node: feedback
//! doc: Ping-pong feedback; last frame is zoomed, rotated, hue-shifted, decayed and lightened under the source
//! input source
//! feedback
//! param decay: float = 0.92 in 0..1 "Retention per 1/60 s, so trails look the same at any frame rate"
//! param zoom: float = 1.01 in 0.9..1.1 "Per-pass zoom of the previous frame"
//! param rotate: float = 0.004 in -0.2..0.2 "Per-pass rotation (radians)"
//! param hue_shift: float = 0.01 in -0.5..0.5 "Per-pass hue rotation (turns)"
//! param mix: float = 1 in 0..1 "How much feedback shows over the source"
#import zygote::common::{rotate2, hue_rotate}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = frame.aspect;
    // Decay, zoom, rotation and hue shift are specified per 1/60 s and scaled
    // by the real frame time, so the look does not depend on the refresh rate.
    // A zero-length frame (transport paused) leaves the trail exactly as it was.
    let steps = clamp(frame.dt * 60.0, 0.0, 10.0);
    let zoom = pow(max(params.zoom, 1e-3), steps);
    let decay = pow(max(params.decay, 1e-4), steps);
    let centred = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);
    let prev_uv = rotate2(centred / zoom, -params.rotate * steps) / vec2<f32>(aspect, 1.0) + 0.5;

    let prev4 = previous(prev_uv);
    var prev = prev4.rgb;
    prev = clamp(hue_rotate(prev, params.hue_shift * steps) * decay, vec3<f32>(0.0), vec3<f32>(1.0));

    let src4 = source(in.uv);
    let src = src4.rgb;
    // Lighten composite: trails persist where brighter than the source, and a
    // static source settles back to itself instead of blowing out.
    let out = mix(src, max(src, prev), params.mix);
    let alpha = mix(src4.a, max(src4.a, prev4.a * decay), params.mix);
    return vec4<f32>(out, alpha);
}
