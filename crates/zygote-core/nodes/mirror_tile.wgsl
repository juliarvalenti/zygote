//! node: mirror_tile
//! doc: Tile UV space into a grid; each axis can repeat or mirror, so one source becomes a kaleidoscopic wallpaper
//! input source
//! param tiles_x: int = 2 in 1..12 "Tiles across"
//! param tiles_y: int = 2 in 1..12 "Tiles down"
//! param mirror_x: bool = true "Flip every other column"
//! param mirror_y: bool = true "Flip every other row"
//! param offset: vec2 = 0, 0 in -1..1 "Scroll the tiling"
//! param zoom: float = 1 in 0.25..4 "Zoom into each tile"

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let tiles = vec2<f32>(f32(params.tiles_x), f32(params.tiles_y));
    let scaled = in.uv * tiles + params.offset;
    let index = floor(scaled);
    var cell = fract(scaled);
    if params.mirror_x != 0u && (i32(index.x) & 1) != 0 {
        cell.x = 1.0 - cell.x;
    }
    if params.mirror_y != 0u && (i32(index.y) & 1) != 0 {
        cell.y = 1.0 - cell.y;
    }
    cell = (cell - 0.5) / params.zoom + 0.5;
    return source(cell);
}
