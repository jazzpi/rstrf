struct Uniforms {
    power_bounds: vec2f,
    time_bounds: vec2f,
    freq_bounds: vec2f,
    pixel_height: f32,
    viewport_width: f32,
    nslices: u32,
    nchan: u32,
    average: u32,
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
    // Each vertex covers one slice, so `u` is just the slice index and it
    // shouldn't be interpolated.
    @location(0) @interpolate(flat) u: u32,
    @location(1) v: f32,
}

struct FragOut {
    @builtin(frag_depth) depth: f32,
    @location(0) color: vec4f,
}

fn unmix(range: vec2f, value: f32) -> f32 {
    return (value - range.x) / (range.y - range.x);
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let x_range = x_ranges[in.time_idx];
    let x = mix(x_range.x, x_range.y, in.corner.x);
    let x_normalized = unmix(uniforms.time_bounds, x);
    // Avoid gaps by snapping left edges down, right edges up
    let px = x_normalized * uniforms.viewport_width;
    let px_snapped = select(floor(px), ceil(px), in.corner.x > 0.5);

    let x_snapped = px_snapped / uniforms.viewport_width;

    let y = unmix(uniforms.freq_bounds, in.corner.y);
    // Gaps can only happen in time, not frequency, so no need to snap y
    let pos = vec2f(x_snapped, y) * 2.0 - 1.0;
    return VertexOut(vec4f(pos, 0.0, 1.0), in.time_idx, in.corner.y);
}

@fragment
fn fs_main(in: VertexOut) -> FragOut {
    let value = get_value(in.u, in.v);

    let normalized = clamp(unmix(uniforms.power_bounds, value), 0.0, 1.0);

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
    let n_y = u32(ceil(uniforms.pixel_height));
    var value = select(uniforms.power_bounds.x, 0.0, uniforms.average != 0u);
    for (var f = 0u; f < n_y; f++) {
        let freq_idx = clamp(u32(freq_idx) + f, 0u, uniforms.nchan - 1u);
        let idx = time_idx * uniforms.nchan + freq_idx;
        if uniforms.average != 0u {
            value += spec_data[idx];
        } else {

            value = max(value, spec_data[idx]);
        }
    }
    if uniforms.average != 0u {
        value /= f32(n_y);
    }
    return value;
}
