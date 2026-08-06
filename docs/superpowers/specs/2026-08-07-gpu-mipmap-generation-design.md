# GPU mipmap generation for the spectrogram frequency-axis pyramid

**Date:** 2026-08-07
**Status:** design approved, not implemented
**Scope:** `src/bin/rstrf/windows/rfplot/shader.rs`, `shader.wgsl`, new `mipmap.wgsl`

## Problem

`Pipeline::create_spectrogram_buffer` builds the frequency-axis mipmap on the CPU, single-threaded,
on the render thread inside `Primitive::prepare`. Two consequences:

1. It is a significant share of load time for large workspaces.
2. It blocks the outstanding TODO to regenerate the pyramid when the max-hold/average mode changes.
   On the CPU that means recomputing *and* re-uploading the pyramid on every toggle, which is too
   slow to hang a UI control off.

## Measurements

One chunk of 314 slices x 80000 channels (100.5 MB), optimized build:

| Step | Time |
| --- | --- |
| `copy_from_slice` of level 0 into the mapped staging buffer | 53.5 ms |
| Full pyramid, all 8 levels | 44.0 ms |
| — of which level 1 alone | 19.5 ms |

The pyramid is ~45% of the CPU work in `create_spectrogram_buffer`. Moving it to the GPU therefore
caps out at roughly **1.8x** on buffer-creation time. The larger win is the mode toggle: on the GPU
it is a re-dispatch with no upload at all.

## Decisions

| Decision | Choice | Rationale |
| --- | --- | --- |
| Where the pyramid is built | Compute shader, one dispatch per level | Level *k* reads level *k-1*, ~167 MB of VRAM traffic per chunk total |
| Alternative rejected | All levels straight from level 0 | 8x the memory traffic; no barrier saving, since the tracker cannot tell the dispatches are independent |
| Alternative rejected | Parallelize the CPU pyramid with rayon | ~7x faster, but a mode toggle still costs a ~33 MB/chunk upload; the data has to stay on the GPU for the toggle to be cheap |
| Adapters without compute | Keep the CPU path as a fallback | The code and its tests already exist; costs one branch |
| Per-level parameters | One uniform buffer, `has_dynamic_offset: true`, 256-byte stride | Idiomatic; the alignment constraint is trivially satisfiable because we choose the stride |
| Validation of the GPU path | Visual check only | A shader bug shows up as a visibly broken waterfall, not a silent number |
| Shader file | Separate `mipmap.wgsl` | `shader.wgsl` currently reads top-to-bottom as one story |

## Architecture

### Compute pipeline

Created once in `Pipeline::new`, which already receives `&wgpu::Device`:

```rust
pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    mipmap: Option<wgpu::ComputePipeline>,   // None => CPU fallback
    instances: HashMap<Uuid, PrimitiveData>,
}
```

The `Option` is the capability check. See "Risks" for how it is decided.

### Bind groups

`spec_data` is already bound in group 1, binding 1, as `var<storage, read>` for rendering. The
compute shader needs `var<storage, read_write>`, which is a different `BufferBindingType`, so a
different layout and a different bind group — pointing at the same `wgpu::Buffer`. No usage flags
change: `BufferUsages::STORAGE` already permits both.

Binding the same buffer read-write and read-only *within one dispatch* would be a validation error
(`STORAGE_READ_WRITE` is an exclusive usage). Binding it read-write in a compute pass and read-only
in a later render pass is a state transition, and wgpu inserts the barrier.

Per-level parameters:

```wgsl
struct MipParams {
    src_offset: u32,   // in f32 elements, into spec_data
    dst_offset: u32,
    nchan_in: u32,     // input channels per slice; nchan_out = nchan_in / 4
    nslices: u32,
    average: u32,
}
```

Stored as an array with a 256-byte stride (`min_uniform_buffer_offset_alignment`) and selected per
dispatch with `set_bind_group(0, &bg, &[level * 256])`.

Note this is *not* applicable to `spec_data` itself: a dynamic offset there needs
`min_storage_buffer_offset_alignment` = 64 floats, and level offsets are
`nslices * sum(floor(nchan / 4^i))`, with `nslices` varying per chunk. The `buf_offset` uniform
field stays.

### Dispatch

```wgsl
@compute @workgroup_size(64)
fn mipmap_main(@builtin(global_invocation_id) gid: vec3u) {
    let nchan_out = params.nchan_in / 4u;
    if gid.x >= nchan_out || gid.y >= params.nslices { return; }

    let src = params.src_offset + gid.y * params.nchan_in + gid.x * 4u;
    var agg: f32;
    if params.average != 0u {
        agg = 0.0;
        for (var i = 0u; i < 4u; i++) { agg += spec_data[src + i]; }
        agg /= 4.0;
    } else {
        agg = spec_data[src];
        for (var i = 1u; i < 4u; i++) { agg = max(agg, spec_data[src + i]); }
    }

    spec_data[params.dst_offset + gid.y * nchan_out + gid.x] = agg;
}
```

- Workgroup size 64: a multiple of every common SIMD width, well under the guaranteed
  `max_compute_invocations_per_workgroup` of 256. No workgroup memory or `workgroupBarrier` is
  needed — each output bin is independent.
- Max-hold seeds from sample 0 rather than a sentinel, so no magic constant has to be kept in sync
  with the CPU path. `compute_mipmap` should be changed to match.
- Branching on `params.average` is uniform across the dispatch, so there is no warp divergence.
- The early return is mandatory. Dispatches are whole workgroups, so 20000 bins at 64-wide is 20032
  invocations; the extra 32 per row would write into the *next* level's region. WGSL's bounds
  guarantee does not help — those writes are inside the buffer.

Dispatch is 2D, `x` over output bins and `y` over slices, because
`max_compute_workgroups_per_dimension` is 65535 and level 1 flattened to 1D would need 98125
workgroups. To keep the `y` dimension in range by construction rather than by argument:

```rust
let chunk_len = spectrogram.nslices
    .min(max_chunk_len)
    .min(limits.max_compute_workgroups_per_dimension as usize);
```

### Synchronization

Data dependencies do **not** serialize dispatches on a GPU: workgroups from dispatch *N+1* can start
while *N* is in flight, and *N*'s writes may still be in L2 when *N+1* reads. WebGPU has no explicit
barrier API by design — automatic hazard tracking is the entire synchronization model, and
`storageBarrier()` is workgroup-scoped, so it does not apply.

The guarantee relied on: `dispatch` calls `flush_states` before every dispatch
(wgpu-core `command/compute.rs:855`), and the tracker skips a barrier only when the state is
unchanged **and** all usages are ordered (`track/mod.rs:342`). `BufferUses::STORAGE_READ_WRITE` is
in `EXCLUSIVE` and not in `ORDERED` (wgpu-types `lib.rs:5370`), so the barrier is never skipped.
This is an API-level contract, not a wgpu quirk: WebGPU specifies that queue commands behave as
though executed in order with each command's writes visible to the next.

Upload ordering: `unmap()` queues a staging copy into pending writes, which flush ahead of the
command buffers in the same submit. So level 0 is present before the first dispatch reads it. The
only rule for us is **unmap before encoding**.

### Buffer creation

The split is *after* level 0 is written — nothing before that point differs between the two paths:

```rust
{
    let mut view = buffer.slice(..).get_mapped_range_mut();
    let floats: &mut [f32] = bytemuck::cast_slice_mut(&mut view[..]);
    floats[..data.len()].copy_from_slice(data);

    if !gpu_mipmap {
        let levels = Self::cpu_mipmap_levels(data, nslices, nchan, average);
        floats[data.len()..][..levels.len()].copy_from_slice(&levels);
    }
}
buffer.unmap();
// GPU path: caller encodes the compute pass here, after unmap
```

The GPU dispatch is encoded by a separate `encode_mipmap_pass(&mut encoder, chunk)`, so the compute
machinery does not thread itself through buffer allocation. The existing per-chunk
`queue.submit(std::iter::empty())` + blocking poll (which caps transient staging memory at one
chunk) gains a real command buffer.

### Mode toggle

In `update_buffers`, alongside the existing `colormap` and `spectrogram_id` invalidation checks:

```rust
let average = primitive.controls.average_plotting();
if primitive_data.average != average {
    match &self.mipmap {
        Some(compute) => Self::redispatch_mipmaps(
            device, queue, compute, &primitive_data.buffers.spectrogram, average,
        ),
        None => Self::repatch_mipmaps_cpu(
            queue, spectrogram, &primitive_data.buffers.spectrogram, average,
        ),
    }
    primitive_data.average = average;
}
```

- GPU: rewrite each chunk's `params` (9 x 256 B), encode one compute pass per chunk, one submit.
  `write_buffer` flushes ahead of the command buffers, so every dispatch sees the new flag.
- CPU: `cpu_mipmap_levels` per chunk, one `write_buffer` of ~33 MB at offset `data_size`. The buffer
  object is unchanged, so bind groups stay valid.

`prepare` runs before `render` for the same frame and we submit during `prepare`, so the dispatch is
queued ahead of the frame's render pass. The compute pass leaves `spec_data` in
`STORAGE_READ_WRITE` and the render pass wants `STORAGE_READ`; a different submit, so the tracker
inserts the transition. The toggle takes effect on the next frame with no explicit sync from us.

### Supporting changes

- Extract `fn chunk_len(limits, spectrogram) -> usize` from `create_spectrogram_buffers`. It is
  already used twice there (chunking and the "too large to render" error) and is needed by the CPU
  toggle path.
- Retain `spectrogram: wgpu::Buffer` on `SpectrogramChunk`; it is currently moved into the bind
  group and dropped, leaving nothing to `write_buffer` into.
- Add `average: bool` to `PrimitiveData`, next to `colormap` and `spectrogram_id`.
- New `struct ChunkMipmap { params: wgpu::Buffer, bind_group: wgpu::BindGroup, nchan_out: Vec<u32> }`
  on `SpectrogramChunk`, `Option`al on the GPU path. Per chunk because the bind group names that
  chunk's `spec_data` and the offsets scale with its `nslices`. ~2.3 KB per chunk.

`cpu_mipmap_levels` returns levels 1.. concatenated in exactly the layout they occupy in
`spec_data`, walking the chain with `split_at_mut` so level *k* reads level *k-1* from the buffer it
is filling. Peak memory is unchanged from today (~33 MB vs ~31 MB for a 100 MB chunk).
`compute_mipmap` stays as the single-level primitive underneath, so its eight tests are untouched.

## Phasing

Each phase is independently verifiable and independently shippable.

### Phase 1 — CPU refactor, no behaviour change

`cpu_mipmap_levels`, `chunk_len` extraction, retained buffer handle, max-hold seeding from sample 0.

**Verification:** the 8 existing `compute_mipmap` tests, plus a new test asserting that
`cpu_mipmap_levels`' layout matches the `buf_offset_chan` formula in `update_buffers`. That
writer/reader contract is currently checked by nothing in the repo, and it is the same contract the
compute shader must satisfy in phase 3. Visual check: the waterfall is unchanged at every zoom
level.

### Phase 2 — recompute on mode toggle, CPU only

`average` on `PrimitiveData`, the invalidation branch, `repatch_mipmaps_cpu`. Closes the
recompute-on-mode-change TODO.

**Verification:** toggle max-hold/average while zoomed out; the picture must change. Slow
(~400 ms for a 1 GB spectrogram) but correct.

### Phase 3 — compute shader

`mipmap.wgsl`, the compute pipeline, `ChunkMipmap`, `encode_mipmap_pass`, `redispatch_mipmaps`, the
capability branch. Closes the compute-shader TODO.

**Verification:** A/B against phase 2 — identical picture at every zoom level and in both modes,
faster. Phase 2 is the reference implementation.

Separating the phases this way separates *adding a feature* from *changing an implementation*: by
phase 3 the toggle already works, so a wrong picture can only be the shader.

## Risks and open items

- **The capability check is a proxy.** `shader::Pipeline::new` receives only `&Device`, but compute
  support is an *adapter* downlevel flag that iced never exposes. The workable proxy is
  `device.limits().max_compute_workgroup_size_x > 0`, which is 0 in the WebGL2 downlevel limit set.
  On native, iced requests `Limits::default()` and falls back to `downlevel_defaults()`, both of
  which carry non-zero compute limits, so any desktop device that was created at all will take the
  compute path. A hypothetical compute-less native adapter would fail at pipeline creation rather
  than fall back. This must carry a `TODO` comment at the construction site as well as being
  recorded here.
- **The CPU fallback will not run on any desktop adapter**, so it is correct-by-construction and by
  review, not correct-by-observation. It is retained because it costs one branch and it is the
  reference the unit tests pin.

## Out of scope

- The level-0 staging copy (53.5 ms/chunk, the other ~55%). Parallelizing it with rayon or loading
  `.bin` data directly into mapped buffers were both considered and deliberately deferred.
- The `ceil`-vs-`floor` mip sizing and the resulting sub-bin positional error at the coarsest
  levels, already mitigated by capping `max_level` at 256 channels.
