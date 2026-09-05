//! node: test_pattern
//! doc: Color bars, grid and a ring
//! param scale: float = 8 in 1..32 "Grid density"

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let aspect = frame.aspect;
    let bar = floor(uv.x * 8.0);
    let bars = vec3<f32>(
        f32((i32(bar) & 4) != 0),
        f32((i32(bar) & 2) != 0),
        f32((i32(bar) & 1) != 0),
    );
    let cell = fract(uv * vec2<f32>(params.scale * aspect, params.scale));
    let line = step(0.94, max(cell.x, cell.y));
    let grid = mix(vec3<f32>(0.08), vec3<f32>(0.85), line);
    let p = (uv - 0.5) * vec2<f32>(aspect, 1.0);
    let ring = smoothstep(0.02, 0.0, abs(length(p) - 0.35));
    var out = select(grid, bars, uv.y < 0.2);
    out = mix(out, vec3<f32>(1.0, 0.3, 0.1), ring);
    return vec4<f32>(out, 1.0);
}
