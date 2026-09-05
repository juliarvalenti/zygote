// Body of the `kaleido` node. Parameters come from the Rust struct in main.rs:
// params.segments, params.spin, params.centre, params.tint, params.mirror.
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
    return source(uv) * vec4<f32>(params.tint.rgb, 1.0);
}
