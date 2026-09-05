//! node: haze
//! doc: 3D noise haze emitted from the centre and drifting outward
//! param density: float = 0.7 in 0..2 "Overall haze brightness"
//! param scale: float = 2.5 in 0.5..12 "Noise scale"
//! param speed: float = 0.2 in 0..2 "Outward drift"
//! param turbulence: float = 0.35 in 0..2 "How fast the noise itself evolves"
//! param octaves: float = 5 in 1..6 "Noise detail"
//! param core: float = 0.12 in 0.01..1 "Radius of the bright core"
//! param falloff: float = 1.8 in 0.2..6 "How quickly the haze thins with distance"
//! param threshold: float = 0.35 in 0..1 "Noise level below which haze is invisible"
#import zygote::common::{fbm3}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = frame.time;
    let p = (in.uv - 0.5) * vec2<f32>(frame.aspect, 1.0) * 2.0;
    let r = length(p);
    let dir = p / max(r, 1e-4);

    // Translate the field along the radial direction so structure moves outward,
    // and stretch it radially so it reads as streaks emitted from the centre.
    let radial = vec2<f32>(r * params.scale - t * params.speed * params.scale, atan2(dir.y, dir.x) * 2.5);
    let q = vec3<f32>(radial, t * params.turbulence);
    var n = fbm3(q, i32(round(params.octaves)));
    n = 0.5 + 0.5 * n;
    n = smoothstep(params.threshold, 1.0, n);

    let body = n * exp(-r * params.falloff) * params.density;
    let glow = exp(-r * r / (params.core * params.core)) * 0.9;
    let v = clamp(body + glow, 0.0, 1.0);
    return vec4<f32>(v, v, v, 1.0);
}
