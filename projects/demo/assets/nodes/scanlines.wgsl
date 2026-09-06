//! node: scanlines
//! doc: CRT-style scanlines and a rolling brightness band
//! input source
//! param density: float = 180 in 20..600 "Lines across the frame"
//! param strength: float = 0.35 in 0..1 "How dark the gaps get"
//! param roll: float = 0.15 in 0..2 "Speed of the rolling band"
//! param tint: color = #b8ffd0 "Phosphor tint"

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let src = source(in.uv);
    let c = src.rgb;
    let line = 0.5 + 0.5 * sin(in.uv.y * params.density * 3.14159);
    let dark = 1.0 - params.strength * (1.0 - line);
    let band = 0.85 + 0.15 * smoothstep(0.0, 1.0, sin((in.uv.y + frame.time * params.roll) * 6.2831));
    return vec4<f32>(c * dark * band * params.tint.rgb, src.a);
}
