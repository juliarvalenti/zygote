//! node: edge_detect
//! doc: Sobel edges; as white lines, drawn over the source, or in the source's own colors
//! input source
//! param radius: float = 0.0015 in 0.0002..0.01 "Sample spacing (fraction of frame height)"
//! param gain: float = 2 in 0..8 "Edge strength"
//! param threshold: float = 0.05 in 0..1 "Ignore edges weaker than this"
//! param mode: choice = lines [lines, overlay, color] "Output"
#import zygote::common::{luma}

fn l(uv: vec2<f32>) -> f32 {
    return luma(source(uv).rgb);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let dx = vec2<f32>(params.radius / frame.aspect, 0.0);
    let dy = vec2<f32>(0.0, params.radius);
    let uv = in.uv;
    let tl = l(uv - dx - dy); let t = l(uv - dy); let tr = l(uv + dx - dy);
    let ml = l(uv - dx);                          let mr = l(uv + dx);
    let bl = l(uv - dx + dy); let b = l(uv + dy); let br = l(uv + dx + dy);
    let gx = (tr + 2.0 * mr + br) - (tl + 2.0 * ml + bl);
    let gy = (bl + 2.0 * b + br) - (tl + 2.0 * t + tr);
    var e = length(vec2<f32>(gx, gy)) * params.gain;
    e = clamp((e - params.threshold) / max(1.0 - params.threshold, 1e-3), 0.0, 1.0);
    let s = source(uv);
    let src = s.rgb;
    var out: vec3<f32>;
    switch params.mode {
        case 1u: { out = src + vec3<f32>(e); }
        case 2u: { out = src * e / max(luma(src), 1e-3); }
        default: { out = vec3<f32>(e); }
    }
    return vec4<f32>(clamp(out, vec3<f32>(0.0), vec3<f32>(1.0)), s.a);
}
