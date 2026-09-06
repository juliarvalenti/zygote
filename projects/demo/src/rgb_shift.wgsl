// Body of the `rgb_shift` node. Parameters come from the Rust struct in
// main.rs: params.amount, params.angle, params.radial, params.mix.
#import zygote::common::{TAU}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let centred = (in.uv - 0.5) * vec2<f32>(frame.aspect, 1.0);
    let dir = vec2<f32>(cos(params.angle * TAU), sin(params.angle * TAU));
    // Blend a fixed direction with a radial one (offset grows toward the edges).
    let offset = mix(dir, centred * 2.0, params.radial) * params.amount / vec2<f32>(frame.aspect, 1.0);
    let src = source(in.uv);
    let r = source(in.uv + offset).r;
    let b = source(in.uv - offset).b;
    let shifted = vec3<f32>(r, src.g, b);
    return vec4<f32>(mix(src.rgb, shifted, params.mix), src.a);
}
