//! node: warp
//! doc: UV remap driven by a displacement texture or an internal animated noise field
//! input source
//! input displacement optional
//! param amount: float = 0.15 in 0..1 "Displacement strength (UV units)"
//! param scale: float = 2 in 0.25..16 "Frequency of the internal displacement field"
//! param speed: float = 0.25 in 0..4 "Internal field evolution speed"
//! param twist: float = 0 in -3..3 "Radial twist around the center"
#import zygote::common::{fbm3, rotate2}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = frame.aspect;
    var uv = in.uv;

    // Radial twist, strongest at the edges.
    let centered = (uv - 0.5) * vec2<f32>(aspect, 1.0);
    let radius = length(centered);
    uv = rotate2(centered, params.twist * radius) / vec2<f32>(aspect, 1.0) + 0.5;

    var d: vec2<f32>;
    if has_displacement() {
        d = displacement(uv).rg * 2.0 - 1.0;
    } else {
        let p = vec3<f32>(uv * vec2<f32>(aspect, 1.0) * params.scale, frame.time * params.speed);
        d = vec2<f32>(fbm3(p, 4), fbm3(p + vec3<f32>(17.3, 9.1, 5.1), 4));
    }
    return source(uv + d * params.amount);
}
