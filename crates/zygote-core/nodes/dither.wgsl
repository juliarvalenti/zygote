//! node: dither
//! doc: Ordered dithering to a few levels per channel; a retro texture distinct from plain posterizing
//! input source
//! param levels: int = 2 in 2..16 "Levels per channel"
//! param pattern: choice = bayer8 [bayer4, bayer8, noise] "Threshold pattern"
//! param cells: float = 320 in 40..1280 "Dither cells across the frame"
//! param strength: float = 1 in 0..1 "Mix with the undithered source"
#import zygote::common::{hash3}

// Bayer threshold for an n×n matrix with n = 2^bits, in 0..1.
fn bayer(x: u32, y: u32, bits: u32) -> f32 {
    var v = 0u;
    var xx = x;
    var yy = y;
    var scale = 1u;
    for (var i = 0u; i < bits; i++) {
        let digit = (((xx & 1u) ^ (yy & 1u)) << 1u) | (yy & 1u);
        v += digit * scale;
        scale *= 4u;
        xx >>= 1u;
        yy >>= 1u;
    }
    let n = f32(1u << (2u * bits));
    return (f32(v) + 0.5) / n;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let grid = vec2<f32>(params.cells, params.cells / frame.aspect);
    let cell = floor(in.uv * grid);
    let sample = (cell + 0.5) / grid;
    let c = source(sample).rgb;
    var t: f32;
    switch params.pattern {
        case 0u: { t = bayer(u32(cell.x) & 3u, u32(cell.y) & 3u, 2u); }
        case 2u: { t = hash3(vec3<f32>(cell, 3.0)).x; }
        default: { t = bayer(u32(cell.x) & 7u, u32(cell.y) & 7u, 3u); }
    }
    let steps = f32(params.levels - 1);
    let q = floor(c * steps + t) / steps;
    return vec4<f32>(mix(source(in.uv).rgb, q, params.strength), 1.0);
}
