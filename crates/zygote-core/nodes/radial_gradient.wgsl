//! node: radial_gradient
//! doc: Soft gradient from a centre point; a mask source, a vignette, or a spotlight
//! param center: vec2 = 0.5, 0.5 in 0..1 "Centre (UV)"
//! param radius: float = 0.5 in 0..1.5 "Where the falloff starts (fraction of frame height)"
//! param softness: float = 0.5 in 0.001..2 "Width of the falloff"
//! param shape: choice = circle [circle, square, diamond] "Distance metric"
//! param inner: color = #ffffff "Colour at the centre"
//! param outer: color = #000000 "Colour outside"

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let d = (in.uv - params.center) * vec2<f32>(frame.aspect, 1.0);
    var dist: f32;
    switch params.shape {
        case 1u: { dist = max(abs(d.x), abs(d.y)); }
        case 2u: { dist = abs(d.x) + abs(d.y); }
        default: { dist = length(d); }
    }
    let t = smoothstep(params.radius, params.radius + params.softness, dist);
    return vec4<f32>(mix(params.inner.rgb, params.outer.rgb, t), 1.0);
}
