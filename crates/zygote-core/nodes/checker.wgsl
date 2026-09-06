//! node: checker
//! doc: Scrolling, rotating checkerboard, optionally beaten against a second rotated grid for moiré
//! param cells: float = 8 in 1..64 "Squares across the frame height"
//! param rotate: float = 0 in 0..1 "Rotation (turns)"
//! param speed: float = 0.1 in -2..2 "Scroll speed (cells per second)"
//! param moire: float = 0 in 0..0.25 "Angle of a second grid multiplied in (0 = off)"
//! param color_a: color = #101010 "Dark squares"
//! param color_b: color = #f0f0f0 "Light squares"
#import zygote::common::{rotate2}

const TAU: f32 = 6.28318530718;

fn board(p: vec2<f32>) -> f32 {
    let c = floor(p);
    return f32((i32(c.x) + i32(c.y)) & 1);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let centred = (in.uv - 0.5) * vec2<f32>(frame.aspect, 1.0);
    let scroll = vec2<f32>(frame.time * params.speed, 0.0);
    let p = rotate2(centred, params.rotate * TAU) * params.cells + scroll;
    var v = board(p);
    if params.moire > 0.0 {
        let q = rotate2(centred, (params.rotate + params.moire) * TAU) * params.cells + scroll;
        v = abs(v - board(q));
    }
    return vec4<f32>(mix(params.color_a.rgb, params.color_b.rgb, v), 1.0);
}
