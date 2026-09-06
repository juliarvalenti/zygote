//! node: kaleido
//! doc: Kaleidoscope: mirrors the source into wedges around a centre
//! input source
//! param segments: float = 6 in 1..24 "Number of mirror wedges"
//! param spin: float = 0 in -1..1 "Rotation of the wedge pattern (turns)"
//! param centre: vec2 = 0.5, 0.5 in 0..1 "Centre of the mirror (UV)"
//! param tint: color = #ffffff "Multiply the result by this colour"
//! param mirror: bool = true "Fold every other wedge so edges meet seamlessly"
#import zygote::common::{TAU, rotate2}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = frame.aspect;
    let p = (in.uv - params.centre) * vec2<f32>(aspect, 1.0);
    let radius = length(p);
    var angle = atan2(p.y, p.x) + params.spin * TAU;

    let wedge = TAU / max(params.segments, 1.0);
    var a = angle - wedge * floor(angle / wedge);
    if params.mirror != 0u {
        // Fold every other wedge back so edges meet seamlessly.
        let index = floor(angle / wedge);
        if (i32(index) & 1) == 1 {
            a = wedge - a;
        }
    }
    let q = vec2<f32>(cos(a), sin(a)) * radius;
    let uv = q / vec2<f32>(aspect, 1.0) + params.centre;
    return source(uv) * params.tint;
}
