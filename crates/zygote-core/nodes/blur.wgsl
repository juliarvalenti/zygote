//! node: blur
//! doc: Gaussian blur along one axis or both. Chain horizontal then vertical for a cheap wide blur; add the result back over the source with blend for bloom
//! input source
//! param radius: float = 0.01 in 0..0.1 "Blur radius (fraction of frame height)"
//! param direction: choice = both [horizontal, vertical, both] "Axis"
//! param threshold: float = 0 in 0..1 "Only what is brighter than this gets blurred (bloom prepass)"
//! param gain: float = 1 in 0..4 "Output multiplier"

const TAPS: i32 = 4; // taps run -TAPS..=TAPS per axis

// Colour above the threshold; alpha is blurred as-is.
fn bright(uv: vec2<f32>) -> vec4<f32> {
    let c = source(uv);
    let rgb = max(c.rgb - vec3<f32>(params.threshold), vec3<f32>(0.0)) / max(1.0 - params.threshold, 1e-3);
    return vec4<f32>(rgb, c.a);
}

fn weight(i: f32, sigma: f32) -> f32 {
    return exp(-(i * i) / (2.0 * sigma * sigma));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // The kernel spans the radius with TAPS steps each side; sigma is set so
    // the outermost tap still carries a little weight.
    let step_uv = params.radius / f32(TAPS);
    let sigma = f32(TAPS) * 0.5;
    let dx = vec2<f32>(step_uv / frame.aspect, 0.0);
    let dy = vec2<f32>(0.0, step_uv);
    var acc = vec4<f32>(0.0);
    var total = 0.0;
    switch params.direction {
        case 0u: {
            for (var i = -TAPS; i <= TAPS; i++) {
                let w = weight(f32(i), sigma);
                acc += bright(in.uv + dx * f32(i)) * w;
                total += w;
            }
        }
        case 1u: {
            for (var i = -TAPS; i <= TAPS; i++) {
                let w = weight(f32(i), sigma);
                acc += bright(in.uv + dy * f32(i)) * w;
                total += w;
            }
        }
        default: {
            for (var j = -TAPS; j <= TAPS; j++) {
                for (var i = -TAPS; i <= TAPS; i++) {
                    let w = weight(f32(i), sigma) * weight(f32(j), sigma);
                    acc += bright(in.uv + dx * f32(i) + dy * f32(j)) * w;
                    total += w;
                }
            }
        }
    }
    let c = acc / total;
    return vec4<f32>(clamp(c.rgb * params.gain, vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}
