// Shared helpers for every Zygote node shader.
#define_import_path zygote::common

const PI: f32 = 3.141592653589793;
const TAU: f32 = 6.283185307179586;

fn hash3(p: vec3<f32>) -> vec3<f32> {
    var q = vec3<f32>(
        dot(p, vec3<f32>(127.1, 311.7, 74.7)),
        dot(p, vec3<f32>(269.5, 183.3, 246.1)),
        dot(p, vec3<f32>(113.5, 271.9, 124.6)),
    );
    return -1.0 + 2.0 * fract(sin(q) * 43758.5453123);
}

// 3D gradient (Perlin-style) noise in roughly -1..1.
fn gnoise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    // quintic interpolant
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);

    let a = dot(hash3(i + vec3<f32>(0.0, 0.0, 0.0)), f - vec3<f32>(0.0, 0.0, 0.0));
    let b = dot(hash3(i + vec3<f32>(1.0, 0.0, 0.0)), f - vec3<f32>(1.0, 0.0, 0.0));
    let c = dot(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), f - vec3<f32>(0.0, 1.0, 0.0));
    let d = dot(hash3(i + vec3<f32>(1.0, 1.0, 0.0)), f - vec3<f32>(1.0, 1.0, 0.0));
    let e = dot(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), f - vec3<f32>(0.0, 0.0, 1.0));
    let g = dot(hash3(i + vec3<f32>(1.0, 0.0, 1.0)), f - vec3<f32>(1.0, 0.0, 1.0));
    let h = dot(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), f - vec3<f32>(0.0, 1.0, 1.0));
    let k = dot(hash3(i + vec3<f32>(1.0, 1.0, 1.0)), f - vec3<f32>(1.0, 1.0, 1.0));

    let x0 = mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
    let x1 = mix(mix(e, g, u.x), mix(h, k, u.x), u.y);
    return mix(x0, x1, u.z) * 1.6;
}

// Fractal Brownian motion, `octaves` clamped to 1..6. Output roughly -1..1.
fn fbm3(p_in: vec3<f32>, octaves: i32) -> f32 {
    var p = p_in;
    var amp = 0.5;
    var sum = 0.0;
    var norm = 0.0;
    let n = clamp(octaves, 1, 6);
    for (var i = 0; i < 6; i++) {
        if i >= n { break; }
        sum += amp * gnoise3(p);
        norm += amp;
        p = p * 2.03 + vec3<f32>(11.3, 7.1, 3.7);
        amp *= 0.5;
    }
    return sum / max(norm, 1e-5);
}

fn rotate2(v: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Rotate hue by `turns` (1.0 = full circle) using the YIQ chroma plane.
fn hue_rotate(c: vec3<f32>, turns: f32) -> vec3<f32> {
    let y = dot(c, vec3<f32>(0.299, 0.587, 0.114));
    let i = dot(c, vec3<f32>(0.596, -0.274, -0.322));
    let q = dot(c, vec3<f32>(0.211, -0.523, 0.312));
    let a = turns * TAU;
    let i2 = i * cos(a) - q * sin(a);
    let q2 = i * sin(a) + q * cos(a);
    return vec3<f32>(
        y + 0.956 * i2 + 0.621 * q2,
        y - 0.272 * i2 - 0.647 * q2,
        y - 1.106 * i2 + 1.703 * q2,
    );
}

// Cosine gradient palettes (after Inigo Quilez). `index` selects a preset 0..4.
fn palette(t: f32, index: f32) -> vec3<f32> {
    let i = i32(round(index));
    var a = vec3<f32>(0.5, 0.5, 0.5);
    var b = vec3<f32>(0.5, 0.5, 0.5);
    var c = vec3<f32>(1.0, 1.0, 1.0);
    var d = vec3<f32>(0.0, 0.33, 0.67);
    if i == 1 {
        c = vec3<f32>(1.0, 1.0, 1.0); d = vec3<f32>(0.0, 0.10, 0.20);
    } else if i == 2 {
        c = vec3<f32>(1.0, 1.0, 0.5); d = vec3<f32>(0.8, 0.90, 0.30);
    } else if i == 3 {
        c = vec3<f32>(1.0, 0.7, 0.4); d = vec3<f32>(0.0, 0.15, 0.20);
    } else if i >= 4 {
        c = vec3<f32>(2.0, 1.0, 0.0); d = vec3<f32>(0.5, 0.20, 0.25);
    }
    return a + b * cos(TAU * (c * t + d));
}
