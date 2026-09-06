//! node: pixelate
//! doc: Snap UV to a coarse grid before sampling; a cheap, obvious modulation target
//! input source
//! param cells: float = 64 in 4..400 "Cells across the frame"
//! param square: bool = true "Square cells (otherwise cells follow the frame's aspect)"

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var grid = vec2<f32>(params.cells, params.cells);
    if params.square != 0u {
        grid.y = params.cells / frame.aspect;
    }
    let uv = (floor(in.uv * grid) + 0.5) / grid;
    return source(uv);
}
