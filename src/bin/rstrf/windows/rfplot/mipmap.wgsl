// Builds one frequency-axis mipmap level from the level below it, in place in the spectrogram
// storage buffer. One dispatch per level; see the module docs in shader.rs.

struct MipParams {
    src_offset: u32,
    dst_offset: u32,
    nchan_in: u32,
    nslices: u32,
    average: u32,
    // WGSL rounds uniform struct sizes up to a multiple of 16. Kept explicit so the layout matches
    // `MipParams` in shader.rs field for field.
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> params: MipParams;
@group(0) @binding(1) var<storage, read_write> spec_data: array<f32>;

const STRIDE: u32 = 4u;

@compute @workgroup_size(64)
fn mipmap_main(@builtin(global_invocation_id) gid: vec3u) {
    let nchan_out = params.nchan_in / STRIDE;
    // Dispatches are whole workgroups, so the tail of each row is out of range. These writes would
    // land inside the buffer, in the *next* level's region, so WGSL's bounds guarantee would not
    // catch them.
    if gid.x >= nchan_out || gid.y >= params.nslices {
        return;
    }

    let src = params.src_offset + gid.y * params.nchan_in + gid.x * STRIDE;
    var agg: f32;
    if params.average != 0u {
        agg = 0.0;
        for (var i = 0u; i < STRIDE; i++) {
            agg += spec_data[src + i];
        }
        agg /= f32(STRIDE);
    } else {
        // Seeded from the first sample so no sentinel constant has to stay in sync with the CPU
        // path in shader.rs.
        agg = spec_data[src];
        for (var i = 1u; i < STRIDE; i++) {
            agg = max(agg, spec_data[src + i]);
        }
    }

    spec_data[params.dst_offset + gid.y * nchan_out + gid.x] = agg;
}
