//! node: streak
//! doc: Feedback that slides the previous frame in one direction instead of zooming; motion-blur streaks rather than spirals
//! input source
//! feedback
//! param angle: float = 0 in 0..1 "Direction the trail travels (turns)"
//! param distance: float = 0.004 in 0..0.05 "Slide per 1/60 s (fraction of frame height)"
//! param decay: float = 0.9 in 0..1 "Retention per 1/60 s"
//! param spread: float = 0 in 0..0.02 "Blur across the direction per pass"
//! param mix: float = 1 in 0..1 "How much trail shows over the source"

const TAU: f32 = 6.28318530718;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let steps = clamp(frame.dt * 60.0, 0.0, 10.0);
    let decay = pow(max(params.decay, 1e-4), steps);
    let dir = vec2<f32>(cos(params.angle * TAU), -sin(params.angle * TAU));
    let to_uv = vec2<f32>(1.0 / frame.aspect, 1.0);
    let back = in.uv - dir * params.distance * steps * to_uv;
    let side = vec2<f32>(-dir.y, dir.x) * params.spread * steps * to_uv;
    var prev = previous(back).rgb * 0.5;
    prev += previous(back + side).rgb * 0.25;
    prev += previous(back - side).rgb * 0.25;
    prev = clamp(prev * decay, vec3<f32>(0.0), vec3<f32>(1.0));
    let src = source(in.uv).rgb;
    let out = mix(src, max(src, prev), params.mix);
    return vec4<f32>(out, 1.0);
}
