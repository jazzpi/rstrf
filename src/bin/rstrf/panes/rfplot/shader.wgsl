struct Uniforms {
    x_bounds: vec2f,
    y_bounds: vec2f,
    power_bounds: vec2f,
    nslices: u32,
    nchan: u32,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> color_map: array<vec4f>;
@group(1) @binding(0) var<storage, read> spec_data: array<f32>;

struct VertexIn {
    @builtin(vertex_index) vertex_index: u32,
}

struct VertexOut {
    @builtin(position) position: vec4f,
    @location(0) texcoord: vec2f,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var xy: vec2f;
    switch in.vertex_index {
        case 0u: { xy = vec2f(0.0, 0.0); }
        case 1u: { xy = vec2f(1.0, 0.0); }
        case 2u: { xy = vec2f(0.0, 1.0); }
        case 3u: { xy = vec2f(0.0, 1.0); }
        case 4u: { xy = vec2f(1.0, 0.0); }
        default: { xy = vec2f(1.0, 1.0); }
    }
    let uv = vec2f(
        mix(uniforms.x_bounds.x, uniforms.x_bounds.y, xy.x),
        mix(uniforms.y_bounds.x, uniforms.y_bounds.y, xy.y),
    );
    return VertexOut(vec4f(xy * 2.0 - 1.0, 0.0, 1.0), uv);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    // TODO: Handle out-of-bounds
    let time_idx = clamp(u32(in.texcoord.x * f32(uniforms.nslices)), 0u, uniforms.nslices - 1u);
    let freq_idx = clamp(u32(in.texcoord.y * f32(uniforms.nchan)), 0u, uniforms.nchan - 1u);
    let idx = time_idx * uniforms.nchan + freq_idx;

    let value = spec_data[idx];

    let normalized = clamp((value - uniforms.power_bounds.x) / (uniforms.power_bounds.y - uniforms.power_bounds.x), 0.0, 1.0);

    let color_index = normalized * 255.0;
    let lower_idx = u32(floor(color_index));
    let upper_idx = min(lower_idx + 1u, 255u);
    let frac = fract(color_index);

    let color_lower = color_map[lower_idx];
    let color_upper = color_map[upper_idx];

    return mix(color_lower, color_upper, frac);
}
