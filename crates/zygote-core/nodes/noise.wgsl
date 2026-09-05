//! node: noise
//! doc: Animated fractal gradient noise; two independent channels so it can drive warps
//! param scale: float = 3 in 0.25..16 "Spatial frequency"
//! param speed: float = 0.3 in 0..4 "Evolution speed"
//! param octaves: float = 4 in 1..6 "fBm octaves"
//! param contrast: float = 1 in 0.1..4 "Output contrast"
#import zygote::common::{fbm3}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let octaves = i32(round(params.octaves));
    let p = vec3<f32>(in.uv * vec2<f32>(frame.aspect, 1.0) * params.scale, frame.time * params.speed);
    var n = fbm3(p, octaves);
    n = clamp(0.5 + 0.5 * n * params.contrast, 0.0, 1.0);
    var m = fbm3(p + vec3<f32>(31.7, 17.3, 5.0), octaves);
    m = clamp(0.5 + 0.5 * m * params.contrast, 0.0, 1.0);
    return vec4<f32>(n, m, (n + m) * 0.5, 1.0);
}
