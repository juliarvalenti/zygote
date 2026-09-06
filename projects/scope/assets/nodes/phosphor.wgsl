//! node: phosphor
//! doc: Phosphor persistence; the last frame blooms outward and decays under the fresh beam
//! input beam
//! feedback
//! param persistence: float = 0.975 in 0..1 "Retention per 1/60 s"
//! param bloom: float = 0.7 in 0..3 "How far the glow spreads per pass (pixels)"
//! param ceiling: float = 3 in 1..8 "Brightest the phosphor can burn"

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let steps = clamp(frame.dt * 60.0, 0.0, 10.0);
    let decay = pow(max(params.persistence, 1e-4), steps);
    // Bloom by averaging a small cross of taps; scaled per pass so the
    // spread does not depend on the frame rate.
    let px = params.bloom * steps / vec2<f32>(1280.0 * frame.aspect / (16.0 / 9.0), 720.0);
    var prev = previous(in.uv).r * 0.4;
    prev += previous(in.uv + vec2<f32>(px.x, 0.0)).r * 0.15;
    prev += previous(in.uv - vec2<f32>(px.x, 0.0)).r * 0.15;
    prev += previous(in.uv + vec2<f32>(0.0, px.y)).r * 0.15;
    prev += previous(in.uv - vec2<f32>(0.0, px.y)).r * 0.15;
    let v = min(prev * decay + beam(in.uv).r, params.ceiling);
    return vec4<f32>(v, v, v, 1.0);
}
