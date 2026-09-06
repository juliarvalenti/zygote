//! node: lines
//! doc: Pulsing concentric polygonal rings and radial spokes from the center
//! param rings: int = 14 in 1..60 "Rings across the half-height"
//! param sides: int = 6 in 3..12 "Polygon sides (12 is nearly a circle)"
//! param thickness: float = 0.06 in 0.005..0.5 "Line width relative to ring spacing"
//! param accent_every: int = 4 in 0..12 "Every n-th ring is heavier (0 = none)"
//! param spokes: int = 12 in 0..64 "Radial spokes"
//! param spoke_width: float = 0.03 in 0.002..0.3 "Spoke width relative to spoke spacing"
//! param pulse_rate: float = 0.5 in 0..4 "Pulses per second"
//! param pulse_depth: float = 0.6 in 0..1 "How much a pulse brightens the rings it passes"
//! param expand: float = 0.12 in -1..1 "Outward drift in ring spacings per second"
//! param rotate: float = 0.01 in -0.5..0.5 "Rotation in turns per second"
//! param falloff: float = 0.9 in 0..4 "Darkening towards the edges"
#import zygote::common::{PI, TAU, rotate2}

// Distance from the center measured so that the iso-lines are regular polygons.
fn polygon_radius(p: vec2<f32>, sides: f32) -> f32 {
    let a = atan2(p.y, p.x);
    let sector = TAU / sides;
    let local = a - sector * floor((a + sector * 0.5) / sector);
    return length(p) * cos(local) / cos(sector * 0.5);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = frame.time;
    var p = (in.uv - 0.5) * vec2<f32>(frame.aspect, 1.0) * 2.0;
    p = rotate2(p, params.rotate * TAU * t);

    let r = polygon_radius(p, f32(params.sides));
    let ring_coord = r * f32(params.rings) - params.expand * t;
    let ring_index = floor(ring_coord);
    let ring_frac = abs(fract(ring_coord) - 0.5);

    // Base rings, with every n-th heavier.
    var width = params.thickness;
    if params.accent_every >= 1 {
        let idx = i32(round(ring_index));
        if idx % params.accent_every == 0 { width *= 2.2; }
    }
    // A pulse traveling outward brightens the rings it crosses.
    let phase = fract(params.pulse_rate * t) * 1.4;
    let radial = r * 0.5;
    let wave = exp(-40.0 * (radial - phase) * (radial - phase));
    width *= 1.0 + params.pulse_depth * wave;
    let ring = 1.0 - smoothstep(width * 0.35, width * 0.5, ring_frac);
    let brightness = 0.55 + 0.45 * params.pulse_depth * wave + 0.45 * (1.0 - params.pulse_depth);

    // Spokes, thinned towards the center so they do not clump.
    var spoke = 0.0;
    if params.spokes >= 1 {
        let a = atan2(p.y, p.x) / TAU;
        let s = abs(fract(a * f32(params.spokes)) - 0.5);
        let sw = params.spoke_width * max(length(p), 0.05);
        spoke = (1.0 - smoothstep(sw * 0.5, sw, s * max(length(p), 0.05))) * smoothstep(0.02, 0.25, length(p));
    }

    let fade = exp(-length(p) * params.falloff * 0.8);
    let v = max(ring * brightness, spoke * 0.7) * fade;
    return vec4<f32>(v, v, v, 1.0);
}
