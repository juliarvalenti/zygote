//! node: voronoi
//! doc: Animated cellular (Worley) noise; a different character from fBm, with cell colours, edges or distance
//! param scale: float = 6 in 1..40 "Cells across the frame"
//! param speed: float = 0.3 in 0..3 "How fast the cell points wander"
//! param jitter: float = 1 in 0..1 "How far points stray from a regular grid"
//! param mode: choice = edges [cells, edges, distance, bubbles] "What to draw"
//! param width: float = 0.04 in 0.005..0.3 "Edge thickness (edges mode)"
//! param contrast: float = 1 in 0.2..4 "Output contrast"
#import zygote::common::{hash3}

const TAU: f32 = 6.28318530718;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.uv * vec2<f32>(frame.aspect, 1.0) * params.scale;
    let base = floor(p);
    var f1 = 8.0;
    var f2 = 8.0;
    var nearest = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let cell = base + vec2<f32>(f32(x), f32(y));
            let h = hash3(vec3<f32>(cell, 7.0));
            let wobble = vec2<f32>(sin(frame.time * params.speed + h.x * TAU), cos(frame.time * params.speed + h.y * TAU));
            let point = cell + 0.5 + params.jitter * 0.45 * wobble;
            let d = length(point - p);
            if d < f1 {
                f2 = f1;
                f1 = d;
                nearest = h;
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    var out: vec3<f32>;
    switch params.mode {
        case 0u: { out = nearest; }
        case 2u: { out = vec3<f32>(clamp(f1, 0.0, 1.0)); }
        case 3u: { out = vec3<f32>(1.0 - smoothstep(0.0, 0.7, f1)); }
        default: { out = vec3<f32>(1.0 - smoothstep(0.0, params.width, f2 - f1)); }
    }
    out = clamp((out - 0.5) * params.contrast + 0.5, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(out, 1.0);
}
