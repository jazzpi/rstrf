struct Uniforms {
    power_bounds: vec2f,
    time_bounds: vec2f,
    freq_bounds: vec2f,
    pixel_height: f32,
    nslices: u32,
    nchan: u32,
}

@group(0) @binding(0) var<storage, read> color_map: array<vec4f>;
@group(1) @binding(0) var<uniform> uniforms: Uniforms;
@group(1) @binding(1) var<storage, read> spec_data: array<f32>;
@group(1) @binding(2) var<storage, read> x_ranges: array<vec2f>;

struct VertexIn {
    @location(0) corner: vec2f, // Vertex buffer
    @location(1) time_idx: u32, // Instance buffer
}

struct VertexOut {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) u: u32,
    @location(1) v: f32,
}

struct FragOut {
    @builtin(frag_depth) depth: f32,
    @location(0) color: vec4f,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let x_range = x_ranges[in.time_idx];
    let x = mix(x_range.x, x_range.y, in.corner.x);
    let x_normalized = (x - uniforms.time_bounds.x) / (uniforms.time_bounds.y - uniforms.time_bounds.x);
    let pos = vec2f(x_normalized, in.corner.y) * 2.0 - 1.0;
    let v = mix(uniforms.freq_bounds.x, uniforms.freq_bounds.y, in.corner.y);
    return VertexOut(vec4f(pos, 0.0, 1.0), in.time_idx, v);
}

@fragment
fn fs_main(in: VertexOut) -> FragOut {
    let value = get_value(in.u, in.v);

    let normalized = clamp((value - uniforms.power_bounds.x) / (uniforms.power_bounds.y - uniforms.power_bounds.x), 0.0, 1.0);

    let color_index = normalized * 255.0;
    let lower_idx = u32(floor(color_index));
    let upper_idx = min(lower_idx + 1u, 255u);
    let frac = fract(color_index);

    let color_lower = color_map[lower_idx];
    let color_upper = color_map[upper_idx];
    let color = mix(color_lower, color_upper, frac);
    let depth = 1.0 - normalized; // lower depth is rendered above higher depth

    return FragOut(depth, color);
}

fn get_value(u: u32, v: f32) -> f32 {
    let time_idx = clamp(u, 0u, uniforms.nslices - 1u);
    let freq_idx = v * f32(uniforms.nchan);
    var value = uniforms.power_bounds.x;
    let n_y = u32(ceil(uniforms.pixel_height));
    for (var f = 0u; f < n_y; f++) {
        let freq_idx = clamp(u32(freq_idx) + f, 0u, uniforms.nchan - 1u);
        let idx = time_idx * uniforms.nchan + freq_idx;
        value = max(value, spec_data[idx]);
    }
    return value;
}
