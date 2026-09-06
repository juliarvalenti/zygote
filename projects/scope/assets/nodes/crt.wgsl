//! node: crt
//! doc: The tube: phosphor colour, graticule, curved glass, vignette and a little flutter
//! input phosphor
//! param tint: color = #46ff78 "Phosphor colour"
//! param hot: color = #f0fff2 "Colour where the phosphor saturates"
//! param graticule: float = 0.07 in 0..1 "Grid brightness"
//! param divisions: int = 8 in 2..16 "Grid divisions across the screen"
//! param curvature: float = 0.10 in 0..0.4 "Barrel distortion of the glass"
//! param vignette: float = 0.55 in 0..1 "Edge darkening"
//! param flicker: float = 0.03 in 0..0.3 "Brightness flutter"
//! param gamma: float = 0.9 in 0.4..2 "Response curve of the phosphor"
#import zygote::common::{hash3}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = (in.uv - 0.5) * vec2<f32>(frame.aspect, 1.0);
    let r2 = dot(c, c);
    let warped = 0.5 + c * (1.0 + params.curvature * r2) / vec2<f32>(frame.aspect, 1.0);
    if any(warped < vec2<f32>(0.0)) || any(warped > vec2<f32>(1.0)) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    // Graticule: a grid in screen height units, with heavier centre axes.
    let g = (warped - 0.5) * vec2<f32>(frame.aspect, 1.0) * f32(params.divisions);
    let cell = abs(fract(g + 0.5) - 0.5);
    let line = 1.0 - smoothstep(0.0, 0.03 * f32(params.divisions) / 8.0, min(cell.x, cell.y));
    let axis = 1.0 - smoothstep(0.0, 0.02, min(abs(g.x), abs(g.y)));
    let grid = params.graticule * (line * 0.5 + axis);

    var v = phosphor(warped).r;
    v = pow(max(v, 0.0), params.gamma);
    let flutter = 1.0 + params.flicker * (hash3(vec3<f32>(floor(frame.time * 24.0), 1.0, 7.0)).x - 0.5);
    v *= flutter;

    let tint = params.tint.rgb;
    let hot = params.hot.rgb;
    var col = tint * min(v, 1.0) + hot * max(v - 1.0, 0.0) * 0.5;
    col = mix(col, hot, smoothstep(1.0, 3.0, v) * 0.6);
    col += tint * grid * 0.35;

    let vig = 1.0 - params.vignette * smoothstep(0.2, 0.95, r2 * 2.2);
    return vec4<f32>(col * vig, 1.0);
}
