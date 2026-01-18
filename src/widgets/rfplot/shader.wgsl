struct Uniforms {
    x_bounds: vec2f,
    y_bounds: vec2f,
    power_bounds: vec2f,
    nslices: u32,
    nchan: u32,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> spec_data: array<f32>;

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

fn viridis_color(t: f32) -> vec3f {
    // TODO: Use a texture instead?
    let r = 0.267004 + t * (0.004874 + t * (2.295841 + t * (-5.139501 + t * (3.687970 - t * 1.205134))));
    let g = 0.004874 + t * (0.424485 + t * (1.439978 + t * (-1.768869 + t * (0.664066 - t * 0.023530))));
    let b = 0.329415 + t * (1.480254 + t * (-2.141231 + t * (0.714629 + t * 0.617008)));
    return clamp(vec3f(r, g, b), vec3f(0.0), vec3f(1.0));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    // TODO: Handle out-of-bounds
    let time_idx = clamp(u32(in.texcoord.x * f32(uniforms.nslices)), 0u, uniforms.nslices - 1u);
    let freq_idx = clamp(u32(in.texcoord.y * f32(uniforms.nchan)), 0u, uniforms.nchan - 1u);
    let idx = time_idx * uniforms.nchan + freq_idx;

    let value = spec_data[idx];

    let power = log2(value + 1e-12);
    let normalized = clamp((power - uniforms.power_bounds.x) / (uniforms.power_bounds.y - uniforms.power_bounds.x), 0.0, 1.0);

    let color = viridis_color(normalized);
    return vec4f(color, 1.0);
}
