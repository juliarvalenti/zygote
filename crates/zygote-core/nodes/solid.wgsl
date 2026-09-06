//! node: solid
//! doc: Solid color source
//! param color: color = #ff8019 "Fill color"

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return params.color;
}
