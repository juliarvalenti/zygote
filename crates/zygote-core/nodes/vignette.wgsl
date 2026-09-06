//! node: vignette
//! doc: Darken or tint toward the edges; spatial finishing, separate from color_grade's tonal work
//! input source
//! param amount: float = 0.6 in 0..1 "How strong the edges get"
//! param radius: float = 0.55 in 0..1.5 "Where the falloff starts (fraction of frame height)"
//! param softness: float = 0.5 in 0.001..2 "Width of the falloff"
//! param roundness: float = 1 in 0..1 "1 = circular, 0 = follows the frame's rectangle"
//! param tint: color = #000000 "Colour the edges fade toward"

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = source(in.uv).rgb;
    let d = abs(in.uv - 0.5) * vec2<f32>(frame.aspect, 1.0) * 2.0;
    let circular = length(d) * 0.5;
    let boxy = max(d.x / frame.aspect, d.y) * 0.5;
    let dist = mix(boxy, circular, params.roundness);
    let t = smoothstep(params.radius, params.radius + params.softness, dist) * params.amount;
    return vec4<f32>(mix(c, params.tint.rgb, t), 1.0);
}
