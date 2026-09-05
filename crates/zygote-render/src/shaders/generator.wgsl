// Zero-input sources: solid color, test pattern, procedural noise.
#import bevy_pbr::forward_io::VertexOutput
#import zygote::common::{fbm3, luma}

struct GeneratorParams {
    // kind, time, scale, speed
    p0: vec4<f32>,
    // r, g, b, octaves
    p1: vec4<f32>,
    // contrast, aspect, unused, unused
    p2: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: GeneratorParams;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let kind = i32(round(params.p0.x));
    let time = params.p0.y;
    let scale = params.p0.z;
    let speed = params.p0.w;
    let aspect = params.p2.y;
    let uv = in.uv;

    if kind == 0 {
        return vec4<f32>(params.p1.rgb, 1.0);
    }

    if kind == 1 {
        // Color bars in the top band, grid below, a circle in the middle.
        let bar = floor(uv.x * 8.0);
        var col = vec3<f32>(
            f32((i32(bar) & 4) != 0),
            f32((i32(bar) & 2) != 0),
            f32((i32(bar) & 1) != 0),
        );
        let cell = fract(uv * vec2<f32>(scale * aspect, scale));
        let line = step(0.94, max(cell.x, cell.y));
        let grid = mix(vec3<f32>(0.08), vec3<f32>(0.85), line);
        let p = (uv - 0.5) * vec2<f32>(aspect, 1.0);
        let ring = smoothstep(0.02, 0.0, abs(length(p) - 0.35));
        var out = select(grid, col, uv.y < 0.2);
        out = mix(out, vec3<f32>(1.0, 0.3, 0.1), ring);
        return vec4<f32>(out, 1.0);
    }

    // kind 2: noise field
    let octaves = i32(round(params.p1.w));
    let contrast = params.p2.x;
    let p = vec3<f32>(uv * vec2<f32>(aspect, 1.0) * scale, time * speed);
    var n = fbm3(p, octaves);
    n = clamp(0.5 + 0.5 * n * contrast, 0.0, 1.0);
    // Second channel with an offset so the field can drive 2D warps.
    var m = fbm3(p + vec3<f32>(31.7, 17.3, 5.0), octaves);
    m = clamp(0.5 + 0.5 * m * contrast, 0.0, 1.0);
    return vec4<f32>(n, m, (n + m) * 0.5, 1.0);
}
