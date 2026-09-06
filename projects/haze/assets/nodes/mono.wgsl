//! node: mono
//! doc: Black-and-white finish: contrast, gamma, grain, vignette and glitch bands
//! input source
//! param contrast: float = 1.5 in 0.2..4 "Contrast around mid gray"
//! param gamma: float = 1.1 in 0.2..3 "Tone curve"
//! param grain: float = 0.10 in 0..1 "Film grain"
//! param vignette: float = 0.7 in 0..2 "Edge darkening"
//! param glitch: float = 0.25 in 0..1 "Horizontal band displacement"
//! param glitch_rate: float = 5 in 0..30 "Glitch updates per second"
//! param bands: int = 28 in 4..200 "Glitch band count"
//! param invert: bool = false "White background"
#import zygote::common::{luma}

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var uv = in.uv;

    // Glitch: a few horizontal bands jump sideways, re-rolled `glitch_rate` times a second.
    let band = floor(uv.y * f32(params.bands));
    let seed = floor(frame.time * params.glitch_rate);
    let roll = hash21(vec2<f32>(band, seed));
    if roll > 1.0 - params.glitch * 0.25 {
        let shift = (hash21(vec2<f32>(seed, band)) - 0.5) * params.glitch * 0.3;
        uv.x += shift;
    }

    var v = luma(source(uv).rgb);
    v = (v - 0.5) * params.contrast + 0.5;
    v = pow(clamp(v, 0.0, 1.0), params.gamma);

    let g = hash21(uv * vec2<f32>(1920.0, 1080.0) + vec2<f32>(frame.index * 0.37, frame.index * 1.13));
    v += (g - 0.5) * params.grain;

    let p = (uv - 0.5) * vec2<f32>(frame.aspect, 1.0);
    v *= 1.0 - params.vignette * dot(p, p);

    v = clamp(v, 0.0, 1.0);
    if params.invert != 0u { v = 1.0 - v; }
    return vec4<f32>(v, v, v, 1.0);
}
