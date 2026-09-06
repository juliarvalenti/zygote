//! node: blend
//! doc: Two-input compositing
//! input a
//! input b
//! param mode: choice = screen [multiply, screen, add, alpha] "Blend operator"
//! param mix: float = 1 in 0..1 "Opacity of input b"

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let a = a(in.uv);
    let b = b(in.uv);
    var blended: vec3<f32>;
    switch params.mode {
        case 0u: { blended = a.rgb * b.rgb; }
        case 1u: { blended = 1.0 - (1.0 - a.rgb) * (1.0 - b.rgb); }
        case 2u: { blended = a.rgb + b.rgb; }
        default: { blended = mix(a.rgb, b.rgb, b.a); }
    }
    // Alpha composites as "over": either input being opaque makes the result opaque.
    return vec4<f32>(mix(a.rgb, blended, params.mix), a.a + b.a * (1.0 - a.a));
}
