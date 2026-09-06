//! node: luma_mask
//! doc: Composite b over a through a mask; the mask is a third input, or b's own brightness when none is connected
//! input a
//! input b
//! input mask optional
//! param channel: choice = luma [luma, red, green, blue, alpha] "Which channel of the mask to key on"
//! param threshold: float = 0 in 0..1 "Mask level where b starts to show"
//! param softness: float = 1 in 0.001..1 "Width of the ramp above the threshold"
//! param invert: bool = false "Show b where the mask is dark instead"
#import zygote::common::{luma}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let a = a(in.uv);
    let b = b(in.uv);
    var m4 = b;
    if has_mask() {
        m4 = mask(in.uv);
    }
    var m: f32;
    switch params.channel {
        case 1u: { m = m4.r; }
        case 2u: { m = m4.g; }
        case 3u: { m = m4.b; }
        case 4u: { m = m4.a; }
        default: { m = luma(m4.rgb); }
    }
    m = smoothstep(params.threshold, params.threshold + params.softness, m);
    if params.invert != 0u {
        m = 1.0 - m;
    }
    return mix(a, b, m);
}
