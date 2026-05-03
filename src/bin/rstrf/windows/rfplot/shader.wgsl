struct Uniforms {
    power_bounds: vec2f,
    pixel_size: vec2f,
    nslices: u32,
    nchan: u32,
}

@group(0) @binding(0) var<storage, read> color_map: array<vec4f>;
@group(1) @binding(0) var<storage, read> spec_data: array<f32>;
@group(1) @binding(1) var<uniform> uniforms: Uniforms;

struct VertexIn {
    @location(0) xy: vec2f,
    @location(1) uv: vec2f,
}

struct VertexOut {
    @builtin(position) position: vec4f,
    @location(0) texcoord: vec2f,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    return VertexOut(vec4f(in.xy * 2.0 - 1.0, 0.0, 1.0), in.uv);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    let value = get_value(in.texcoord);

    let normalized = clamp((value - uniforms.power_bounds.x) / (uniforms.power_bounds.y - uniforms.power_bounds.x), 0.0, 1.0);

    let color_index = normalized * 255.0;
    let lower_idx = u32(floor(color_index));
    let upper_idx = min(lower_idx + 1u, 255u);
    let frac = fract(color_index);

    let color_lower = color_map[lower_idx];
    let color_upper = color_map[upper_idx];

    return mix(color_lower, color_upper, frac);
}

fn get_value(coord: vec2f) -> f32 {
    let time_idx = coord.x * f32(uniforms.nslices);
    let freq_idx = coord.y * f32(uniforms.nchan);
    var value = uniforms.power_bounds.x;
    let n_x = u32(ceil(uniforms.pixel_size.x));
    let n_y = u32(ceil(uniforms.pixel_size.y));
    for (var t = 0u; t < n_x; t++) {
        for (var f = 0u; f < n_y; f++) {
            let time_idx = clamp(u32(time_idx) + t, 0u, uniforms.nslices - 1u);
            let freq_idx = clamp(u32(freq_idx) + f, 0u, uniforms.nchan - 1u);
            let idx = time_idx * uniforms.nchan + freq_idx;
            value = max(value, spec_data[idx]);
        }
    }
    return value;
}
