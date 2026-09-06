//! node: beam
//! doc: The electron beam of an XY oscilloscope; each frame draws only the arc swept since the last frame, so the trace lives in the phosphor
//! param ratio_x: float = 1 in 1..8 "Horizontal oscillator frequency (cycles per figure)"
//! param ratio_y: float = 1 in 1..8 "Vertical oscillator frequency"
//! param phase: float = 0.25 in 0..1 "Phase between the two axes (turns); sweeping it turns the figure"
//! param speed: float = 0.5 in 0..3 "Figures traced per second"
//! param depth: float = 0.45 in 0..1 "A third oscillator pushes the trace toward and away from the glass"
//! param ratio_z: float = 1 in 0..8 "Frequency of the depth oscillator"
//! param size: float = 0.72 in 0.1..1 "Trace size"
//! param focus: float = 0.007 in 0.002..0.05 "Beam width"
//! param intensity: float = 2.2 in 0..4 "Beam brightness"
//! param waveform: choice = sine [sine, triangle, fold] "Oscillator shape"
//! param hum: float = 0.003 in 0..0.05 "Mains hum wobbling the beam"

const TAU: f32 = 6.28318530718;
const SEGMENTS: i32 = 48;

fn osc(x: f32) -> f32 {
    switch params.waveform {
        case 1u: { return asin(sin(x)) * (2.0 / 3.14159265); }
        case 2u: { return sin(x + 0.7 * sin(2.0 * x)); }
        default: { return sin(x); }
    }
}

// Beam position at oscillator time `t` (radians of the base oscillator).
fn trace(t: f32) -> vec2<f32> {
    let x = osc(params.ratio_x * t + params.phase * TAU);
    let y = osc(params.ratio_y * t);
    let z = sin(params.ratio_z * t);
    let persp = 1.0 / (1.0 + params.depth * 0.6 * z);
    var p = vec2<f32>(x, y) * persp * params.size;
    p += params.hum * vec2<f32>(sin(t * 37.0), cos(t * 53.0));
    return p;
}

fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let h = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-8), 0.0, 1.0);
    return length(p - a - ab * h);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = (in.uv - 0.5) * vec2<f32>(frame.aspect, 1.0) * 2.0;
    // The arc swept this frame. A long frame draws a longer arc; a paused
    // transport (dt = 0) draws only the beam spot.
    let dt = min(frame.dt, 0.5);
    let t1 = frame.time * params.speed * TAU;
    let t0 = t1 - dt * params.speed * TAU;

    // Energy deposited on the phosphor is beam power × time, spread along the
    // arc: brightness per unit length goes as 1/velocity, so the trace glows
    // where the beam lingers (turning points, the far side of the depth
    // swing) and thins where it races. Segments shorter than the spot count
    // as stationary within it, so the sum does not depend on the frame rate
    // or the segment count. A zero-length frame deposits nothing.
    let dt_i = dt / f32(SEGMENTS);
    var glow = 0.0;
    var a = trace(t0);
    for (var i = 1; i <= SEGMENTS; i++) {
        let b = trace(mix(t0, t1, f32(i) / f32(SEGMENTS)));
        let d = seg_dist(p, a, b);
        glow += exp(-d * d / (2.0 * params.focus * params.focus)) * dt_i / max(length(b - a), params.focus);
        a = b;
    }
    let v = min(glow * params.intensity * 2.6, 4.0);
    return vec4<f32>(v, v, v, 1.0);
}
