# GPU Mipmap Compute Shader Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the spectrogram's frequency-axis mipmap in a compute shader instead of on the CPU, and make the max-hold/average toggle a re-dispatch with no upload — phase 3 of `docs/superpowers/specs/2026-08-07-gpu-mipmap-generation-design.md`.

**Architecture:** A compute pipeline runs one dispatch per mipmap level, reading level *k-1* and writing level *k* through a single `var<storage, read_write>` binding on the spectrogram buffer that the render pipeline already binds read-only. Per-level parameters live in one uniform buffer indexed by a dynamic offset. The CPU path from phases 1–2 stays as the fallback for adapters without compute support, and remains the reference implementation the unit tests pin.

**Tech Stack:** Rust, wgpu 27 via iced 0.14, WGSL, bytemuck.

## Prerequisite

**Phases 1 and 2 must be merged first** — `docs/superpowers/plans/2026-08-07-cpu-mipmap-refactor-and-mode-toggle.md`. This plan consumes `mipmap_level_nchan`, `mipmap_level_count`, `mipmap_level_offset_chan`, `chunk_len`, `cpu_mipmap_levels`, `repatch_mipmaps_cpu`, the retained `SpectrogramChunk::spectrogram` handle, and `PrimitiveData::average`. None of those exist yet. If `cargo build` fails on a missing name from that list, stop — the prerequisite is not in place.

## Global Constraints

- Work is in `src/bin/rstrf/windows/rfplot/shader.rs` plus one new file `src/bin/rstrf/windows/rfplot/mipmap.wgsl`. No other files. No new dependencies.
- `MIPMAP_FACTOR` is `f64` = 4.0; existing code spells the integer form `MIPMAP_FACTOR as usize`. Follow that.
- Format with `cargo +nightly fmt --all` — nightly rustfmt only.
- `cargo clippy --bin rstrf --all-targets` must introduce no new warnings. Baseline before phases 1–2 was **5**; re-measure after the prerequisite lands and hold that number.
- Tests run with `cargo test --bin rstrf`. Baseline after phases 1–2 is **38** passing. None may regress.
- The test module already defines `MAX_HOLD`, `AVERAGE`, `ramp(nslices, nchan)` and `assert_close(actual, expected)` — reuse them, do not redefine.
- The compute shader itself has **no automated coverage** — that was a deliberate decision recorded in the spec. Its arithmetic is covered indirectly by unit-testing `mip_params_bytes`, and the shader is validated by visual A/B against the CPU path.
- Commit messages follow conventional commits and end with the trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.

---

### Task 1: Shader and per-level parameters

The compute shader and the pure function that generates its uniform data. Nothing is wired up yet, so this task is fully testable on the CPU.

**Files:**
- Create: `src/bin/rstrf/windows/rfplot/mipmap.wgsl`
- Modify: `src/bin/rstrf/windows/rfplot/shader.rs` — add `MipParams`, `MIP_PARAMS_STRIDE`, `WORKGROUP_SIZE` and `mip_params_bytes` near the other mipmap free functions (after `chunk_len`)
- Test: `src/bin/rstrf/windows/rfplot/shader.rs`, `mod tests`

**Interfaces:**
- Consumes: `mipmap_level_nchan`, `mipmap_level_count`, `mipmap_level_offset_chan` (phase 1)
- Produces:
  - `const MIP_PARAMS_STRIDE: usize = 256;`
  - `const WORKGROUP_SIZE: u32 = 64;`
  - `struct MipParams` — 32 bytes, `bytemuck::Pod`
  - `fn mip_params_bytes(nslices: usize, nchan: usize, average: bool) -> Vec<u8>`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `shader.rs`:

```rust
    fn params_at(bytes: &[u8], level: u32) -> MipParams {
        let at = (level as usize - 1) * MIP_PARAMS_STRIDE;
        // `pod_read_unaligned`, not `from_bytes`: a `Vec<u8>` carries no alignment guarantee, and
        // `from_bytes` panics rather than reading unaligned.
        bytemuck::pod_read_unaligned(&bytes[at..at + std::mem::size_of::<MipParams>()])
    }

    /// Each level's dispatch must read the level below it and write where `update_buffers`
    /// expects to find it. These offsets are the same layout the CPU builder writes.
    #[test]
    fn mip_params_offsets_match_the_level_layout() {
        let (nslices, nchan) = (314usize, 80000usize);
        let bytes = mip_params_bytes(nslices, nchan, MAX_HOLD);

        assert_eq!(
            bytes.len(),
            mipmap_level_count(nchan) as usize * MIP_PARAMS_STRIDE
        );

        for level in 1..=mipmap_level_count(nchan) {
            let p = params_at(&bytes, level);
            assert_eq!(
                p.src_offset as usize,
                nslices * mipmap_level_offset_chan(nchan, level - 1),
                "level {} src",
                level
            );
            assert_eq!(
                p.dst_offset as usize,
                nslices * mipmap_level_offset_chan(nchan, level),
                "level {} dst",
                level
            );
            assert_eq!(
                p.nchan_in as usize,
                mipmap_level_nchan(nchan, level - 1),
                "level {} nchan_in",
                level
            );
            assert_eq!(p.nslices as usize, nslices, "level {} nslices", level);
        }
    }

    /// Level 1 reads level 0, which starts at the beginning of the chunk.
    #[test]
    fn mip_params_first_level_reads_from_zero() {
        let bytes = mip_params_bytes(7, 1024, MAX_HOLD);
        let p = params_at(&bytes, 1);
        assert_eq!(p.src_offset, 0);
        assert_eq!(p.dst_offset, 7 * 1024);
        assert_eq!(p.nchan_in, 1024);
    }

    #[test]
    fn mip_params_carries_the_plotting_mode() {
        for average in [MAX_HOLD, AVERAGE] {
            let bytes = mip_params_bytes(7, 1024, average);
            for level in 1..=mipmap_level_count(1024) {
                assert_eq!(params_at(&bytes, level).average, average as u32);
            }
        }
    }

    #[test]
    fn mip_params_is_empty_when_there_are_no_levels() {
        for nchan in 0..4 {
            assert!(mip_params_bytes(7, nchan, MAX_HOLD).is_empty(), "nchan={}", nchan);
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --bin rstrf mip_params`
Expected: FAIL to compile — `cannot find function mip_params_bytes in this scope`.

- [ ] **Step 3: Add the constants, struct and generator**

Insert after `chunk_len` in `shader.rs`:

```rust
/// Invocations per workgroup in `mipmap.wgsl`. A multiple of every common SIMD width, and well
/// under the 256 that `max_compute_invocations_per_workgroup` guarantees.
const WORKGROUP_SIZE: u32 = 64;

/// Stride between consecutive `MipParams` in the uniform buffer. Dynamic offsets must be a
/// multiple of `min_uniform_buffer_offset_alignment`; 256 is the value in `Limits::default()`, and
/// any device requiring less is satisfied by it too, since the requirement is always a power of
/// two no greater than 256.
const MIP_PARAMS_STRIDE: usize = 256;

/// One dispatch's worth of parameters. Must match `struct MipParams` in `mipmap.wgsl`, including
/// the padding: WGSL rounds uniform struct sizes up to a multiple of 16.
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct MipParams {
    src_offset: u32,
    dst_offset: u32,
    nchan_in: u32,
    nslices: u32,
    average: u32,
    _padding: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<MipParams>() == 32);
const _: () = assert!(std::mem::size_of::<MipParams>() <= MIP_PARAMS_STRIDE);

/// Parameters for every dispatch of one chunk, laid out at `MIP_PARAMS_STRIDE` intervals so a
/// dynamic offset can select a level.
fn mip_params_bytes(nslices: usize, nchan: usize, average: bool) -> Vec<u8> {
    let levels = mipmap_level_count(nchan);
    let mut bytes = vec![0u8; levels as usize * MIP_PARAMS_STRIDE];
    for level in 1..=levels {
        let params = MipParams {
            src_offset: (nslices * mipmap_level_offset_chan(nchan, level - 1)) as u32,
            dst_offset: (nslices * mipmap_level_offset_chan(nchan, level)) as u32,
            nchan_in: mipmap_level_nchan(nchan, level - 1) as u32,
            nslices: nslices as u32,
            average: average as u32,
            _padding: [0; 3],
        };
        let at = (level as usize - 1) * MIP_PARAMS_STRIDE;
        bytes[at..at + std::mem::size_of::<MipParams>()]
            .copy_from_slice(bytemuck::bytes_of(&params));
    }
    bytes
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --bin rstrf mip_params`
Expected: PASS, 4 tests.

- [ ] **Step 5: Write the compute shader**

Create `src/bin/rstrf/windows/rfplot/mipmap.wgsl`:

```wgsl
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
```

- [ ] **Step 6: Verify**

Run: `cargo test --bin rstrf && cargo clippy --bin rstrf --all-targets && cargo +nightly fmt --all -- --check`
Expected: 42 tests pass, no new clippy warnings, fmt silent. The `.wgsl` file is not compiled yet — it is not referenced by any `include_str!`.

- [ ] **Step 7: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/mipmap.wgsl src/bin/rstrf/windows/rfplot/shader.rs
git commit -m "$(cat <<'EOF'
feat: add mipmap compute shader and its dispatch parameters

Not wired up yet. mip_params_bytes is pure and unit-tested, which is the
only automated coverage the GPU path gets; the shader itself is
validated by A/B against the CPU builder.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Compute pipeline

Create the pipeline once, alongside the render pipeline. The layout must be written out by hand rather than derived — see Step 3.

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/shader.rs` — `Pipeline` struct (lines ~124-127) and `impl shader::Pipeline for Pipeline::new` (lines ~129-186)

**Interfaces:**
- Consumes: `MipParams` (Task 1)
- Produces: `Pipeline::mipmap: Option<wgpu::ComputePipeline>` — `None` means the CPU fallback is in use

- [ ] **Step 1: Add the field**

```rust
pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    /// `None` on adapters without compute support; the CPU path builds the pyramid instead.
    mipmap: Option<wgpu::ComputePipeline>,
    instances: HashMap<Uuid, PrimitiveData>,
}
```

- [ ] **Step 2: Build the pipeline in `Pipeline::new`**

Insert before the `Self { … }` literal at the end of `new` (around line 181):

```rust
        // TODO: this is a proxy for the real capability. Compute support is an *adapter* downlevel
        // flag (`DownlevelFlags::COMPUTE_SHADERS`) and iced never hands the adapter to
        // `shader::Pipeline::new`, so we infer it from the compute limits being non-zero — they are
        // literally 0 in wgpu's WebGL2 downlevel limit set. On native, iced requests
        // `Limits::default()` and falls back to `downlevel_defaults()`, both of which carry
        // non-zero compute limits, so a compute-less native adapter would fail at pipeline creation
        // rather than fall back here.
        let mipmap = (device.limits().max_compute_workgroup_size_x >= WORKGROUP_SIZE).then(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("spectrogram.mipmap.shader"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                    "mipmap.wgsl"
                ))),
            });

            // The layout cannot be auto-derived here: `has_dynamic_offset` has no WGSL spelling,
            // so `layout: None` would produce a binding with dynamic offsets disabled.
            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("spectrogram.mipmap.bind_group_layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: true,
                                min_binding_size: std::num::NonZeroU64::new(
                                    std::mem::size_of::<MipParams>() as u64,
                                ),
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: false },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });

            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("spectrogram.mipmap.pipeline_layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("spectrogram.mipmap.pipeline"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some("mipmap_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        });
```

and change the literal to:

```rust
        Self {
            pipeline,
            mipmap,
            instances: HashMap::new(),
        }
```

- [ ] **Step 3: Verify the shader compiles and the app still runs**

Run: `cargo run --release -- <a .bin file>`
Expected: the app opens and renders exactly as before — nothing dispatches yet. Any WGSL syntax or layout error surfaces here as a panic from wgpu's validation at `create_compute_pipeline`, naming the offending line of `mipmap.wgsl`.

- [ ] **Step 4: Verify the build is clean**

Run: `cargo test --bin rstrf && cargo clippy --bin rstrf --all-targets && cargo +nightly fmt --all -- --check`
Expected: 42 tests pass, no new clippy warnings, fmt silent.

- [ ] **Step 5: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/shader.rs
git commit -m "$(cat <<'EOF'
feat: create the mipmap compute pipeline

Layout is spelled out by hand rather than auto-derived, because
has_dynamic_offset has no WGSL spelling and layout: None would disable
it. Capability detection is a documented proxy; see the TODO.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Build the pyramid on the GPU at load

Wire the dispatch into buffer creation and stop doing the work on the CPU when compute is available. This is the task that produces the load-time win.

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/shader.rs` — `chunk_len` (phase 1), `SpectrogramChunk` (lines ~103-109), `create_spectrogram_buffer`, `create_spectrogram_buffers` (lines ~342-468), `create_buffers` and `update_buffers` call sites

**Interfaces:**
- Consumes: `Pipeline::mipmap` (Task 2), `mip_params_bytes`, `MIP_PARAMS_STRIDE`, `WORKGROUP_SIZE` (Task 1), `mipmap_level_count`, `mipmap_level_nchan` (phase 1)
- Produces:
  - `struct ChunkMipmap { params: wgpu::Buffer, bind_group: wgpu::BindGroup, nchan: usize }`
  - `SpectrogramChunk::mipmap: Option<ChunkMipmap>`
  - `fn encode_mipmap_pass(pipeline: &wgpu::ComputePipeline, encoder: &mut wgpu::CommandEncoder, chunk: &SpectrogramChunk)`

- [ ] **Step 1: Keep the dispatch's `y` dimension in range by construction**

`dispatch_workgroups`' y dimension is the chunk's slice count, and `max_compute_workgroups_per_dimension` is 65535. Bound it where chunking is decided rather than reasoning about it at the dispatch site. In `chunk_len` (added in phase 1), change the final expression to:

```rust
    spectrogram
        .nslices
        .min(max_buf_size / slice_size)
        .min(limits.max_compute_workgroups_per_dimension as usize)
```

- [ ] **Step 2: Add `ChunkMipmap` and the struct field**

Next to `SpectrogramChunk` (around line 103):

```rust
/// GPU-side mipmap state for one chunk. `None` when the CPU built the pyramid, or when the chunk
/// has too few channels for any level.
struct ChunkMipmap {
    /// `MipParams` per level at `MIP_PARAMS_STRIDE` intervals, selected by dynamic offset.
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Level-0 channel count, enough to re-derive every level's dispatch size.
    nchan: usize,
}

struct SpectrogramChunk {
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    instances: wgpu::Buffer,
    /// Retained so the mode toggle can patch mipmap levels in place.
    spectrogram: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    mipmap: Option<ChunkMipmap>,
    nslices: u32,
}
```

- [ ] **Step 3: Add the dispatch encoder**

Add to `impl Pipeline`, after `cpu_mipmap_levels`:

```rust
    /// One dispatch per mipmap level, reading level k-1 and writing level k. wgpu inserts a memory
    /// barrier before each dispatch because `spec_data` is bound `read_write`, which is an
    /// exclusive usage and therefore never eligible for barrier elision.
    fn encode_mipmap_pass(
        pipeline: &wgpu::ComputePipeline,
        encoder: &mut wgpu::CommandEncoder,
        chunk: &SpectrogramChunk,
    ) {
        let Some(mipmap) = &chunk.mipmap else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("spectrogram.mipmap.pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        for level in 1..=mipmap_level_count(mipmap.nchan) {
            let nchan_out = mipmap_level_nchan(mipmap.nchan, level) as u32;
            let offset = (level as usize - 1) * MIP_PARAMS_STRIDE;
            pass.set_bind_group(0, &mipmap.bind_group, &[offset as u32]);
            pass.dispatch_workgroups(nchan_out.div_ceil(WORKGROUP_SIZE), chunk.nslices, 1);
        }
    }
```

- [ ] **Step 4: Skip the CPU pyramid when the GPU will build it**

`create_spectrogram_buffer` currently always fills levels 1+. Add a `gpu_mipmap: bool` parameter after `average`, and guard the fill:

```rust
            floats[..data.len()].copy_from_slice(bytemuck::cast_slice(data));
            if !gpu_mipmap {
                let levels = Self::cpu_mipmap_levels(data, nslices, nchan, average);
                floats[data.len()..][..levels.len()].copy_from_slice(&levels);
            }
```

- [ ] **Step 5: Create the per-chunk mipmap state and dispatch**

`create_spectrogram_buffers` takes `pipeline: &wgpu::RenderPipeline` today. Add a second parameter `mipmap_pipeline: Option<&wgpu::ComputePipeline>` after it, and pass `mipmap_pipeline.is_some()` as `gpu_mipmap` to the `create_spectrogram_buffer` call (line ~418).

Then, after the render `bind_group` is created (line ~451) and *before* the existing `queue.submit`, build the compute state and the chunk, and dispatch:

```rust
            let mipmap = mipmap_pipeline
                .filter(|_| mipmap_level_count(spectrogram.nchan) > 0)
                .map(|compute| {
                    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(format!("{prefix}.buffer.mip_params").as_str()),
                        contents: &mip_params_bytes(
                            nslices as usize,
                            spectrogram.nchan,
                            average,
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                    let layout = compute.get_bind_group_layout(0);
                    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(format!("{prefix}.bind_group.mipmap").as_str()),
                        layout: &layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &params,
                                    offset: 0,
                                    // One level's worth. The dynamic offset shifts this window, so
                                    // binding the whole buffer would put offset+size out of bounds.
                                    size: std::num::NonZeroU64::new(
                                        std::mem::size_of::<MipParams>() as u64,
                                    ),
                                }),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: spectrogram_buffer.as_entire_binding(),
                            },
                        ],
                    });
                    ChunkMipmap {
                        params,
                        bind_group,
                        nchan: spectrogram.nchan,
                    }
                });

            let chunk = SpectrogramChunk {
                uniform: uniform_buffer,
                vertices: vertex_buffer,
                instances: instance_buffer,
                spectrogram: spectrogram_buffer,
                bind_group,
                mipmap,
                nslices: nslices as u32,
            };
```

Then replace the existing `queue.submit(std::iter::empty());` and the trailing `SpectrogramChunk { … }` literal with:

```rust
            // `create_buffer_init` parks a host-visible staging copy of `chunk` in the queue's
            // pending writes until the next submit. Flush + wait here so that staging memory is
            // reclaimed before we build the next chunk, capping the transient overhead at one chunk
            // instead of holding a full second copy of the whole spectrogram in RAM until the next
            // frame happens to be rendered.
            //
            // The mipmap dispatch rides along in the same submit. `unmap()` inside
            // `create_spectrogram_buffer` queued level 0 into pending writes, which flush ahead of
            // command buffers, so level 0 is present before the first dispatch reads it.
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(format!("{prefix}.encoder.mipmap").as_str()),
            });
            if let Some(compute) = mipmap_pipeline {
                Self::encode_mipmap_pass(compute, &mut encoder, &chunk);
            }
            queue.submit(Some(encoder.finish()));
            let _ = device.poll(wgpu::PollType::wait_indefinitely());

            chunk
```

- [ ] **Step 6: Update the two call sites**

In `create_buffers` (line ~319), pass the compute pipeline through. `create_buffers` is called from the `or_insert_with_key` closure in `update_buffers`, which already captures `&self.pipeline`; capture `self.mipmap.as_ref()` the same way. Add a `mipmap_pipeline: Option<&wgpu::ComputePipeline>` parameter to `create_buffers` and forward it.

In the `else if primitive_data.spectrogram_id != spectrogram.id` arm of `update_buffers` (line ~269), add `self.mipmap.as_ref(),` after `&self.pipeline,`.

- [ ] **Step 7: Verify the build**

Run: `cargo test --bin rstrf && cargo clippy --bin rstrf --all-targets && cargo +nightly fmt --all -- --check`
Expected: 42 tests pass, no new clippy warnings, fmt silent.

- [ ] **Step 8: Manual A/B against the CPU path**

This is the shader's only real validation, so do it properly.

1. Run `cargo run --release -- <a .bin file with narrow-band signals>`. Zoom out on the frequency axis until `RUST_LOG=trace` shows `mipmap level 1` or higher in the `Updating buffers for primitive` line. Screenshot at two or three zoom levels, in both max-hold and average mode.
2. Temporarily force the fallback: change the condition in Task 2 Step 2 to `false.then(|| { … })`.
3. Rebuild, repeat the same zoom levels and modes, screenshot again.
4. Compare. The images must match. A mismatch that grows with zoom-out points at the offsets in `mip_params_bytes`; a mismatch confined to the top of the plot points at the early-return bound in `mipmap.wgsl`; torn or noisy output points at synchronization.
5. Revert the forced fallback.

Also confirm the win: time the load with `RUST_LOG=debug` and compare the wall time between the two builds. Expect roughly 1.8x on buffer creation.

- [ ] **Step 9: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/shader.rs
git commit -m "$(cat <<'EOF'
perf: build the spectrogram mipmap in a compute shader

One dispatch per level, reading level k-1 and writing level k in place.
Removes ~45% of the CPU work in create_spectrogram_buffer; the rest is
the level-0 staging copy, which is unchanged.

The CPU builder stays as the fallback for adapters without compute and
as the reference the unit tests pin.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Re-dispatch on mode toggle

Phase 2 made the toggle correct by recomputing and re-uploading on the CPU. This makes it cheap: rewrite ~2 KB of parameters per chunk and re-run the dispatches, with no pixel data crossing the bus.

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/shader.rs` — the `primitive_data.average` block in `update_buffers` (added in phase 2), plus a new `redispatch_mipmaps`

**Interfaces:**
- Consumes: `encode_mipmap_pass` (Task 3), `mip_params_bytes` (Task 1), `repatch_mipmaps_cpu` (phase 2)
- Produces: `fn redispatch_mipmaps(device: &wgpu::Device, queue: &wgpu::Queue, pipeline: &wgpu::ComputePipeline, chunks: &[SpectrogramChunk], average: bool)`

- [ ] **Step 1: Add the re-dispatch**

Add to `impl Pipeline`, after `encode_mipmap_pass`:

```rust
    /// Rebuild every chunk's pyramid for a new plotting mode. `write_buffer` lands in the queue's
    /// pending writes, which flush ahead of the command buffers in the same submit, so every
    /// dispatch sees its new `average` flag.
    fn redispatch_mipmaps(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &wgpu::ComputePipeline,
        chunks: &[SpectrogramChunk],
        average: bool,
    ) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("spectrogram.mipmap.encoder.redispatch"),
        });
        for chunk in chunks {
            let Some(mipmap) = &chunk.mipmap else {
                continue;
            };
            queue.write_buffer(
                &mipmap.params,
                0,
                &mip_params_bytes(chunk.nslices as usize, mipmap.nchan, average),
            );
            Self::encode_mipmap_pass(pipeline, &mut encoder, chunk);
        }
        queue.submit(Some(encoder.finish()));
    }
```

- [ ] **Step 2: Branch the toggle**

Replace the body of the `if primitive_data.average != average` block added in phase 2:

```rust
        let average = primitive.controls.average_plotting();
        if primitive_data.average != average {
            match &self.mipmap {
                Some(compute) => Self::redispatch_mipmaps(
                    device,
                    queue,
                    compute,
                    &primitive_data.buffers.spectrogram,
                    average,
                ),
                None => Self::repatch_mipmaps_cpu(
                    device,
                    queue,
                    spectrogram,
                    &primitive_data.buffers.spectrogram,
                    average,
                ),
            }
            primitive_data.average = average;
        }
```

- [ ] **Step 3: Verify the build**

Run: `cargo test --bin rstrf && cargo clippy --bin rstrf --all-targets && cargo +nightly fmt --all -- --check`
Expected: 42 tests pass, no new clippy warnings, fmt silent.

- [ ] **Step 4: Manual check — the toggle is correct and fast**

Run: `cargo run --release -- <a .bin file with narrow-band signals>`

1. Zoom out until the mipmap engages (`RUST_LOG=trace`, `mipmap level 1` or higher).
2. Toggle average/max-hold repeatedly.

Expected: narrow-band carriers dim in average mode and pop back in max-hold, identically to how it behaved after phase 2 — same picture, no stall. If phase 2's behaviour is still available in git history, A/B a screenshot pair to confirm the images match.

- [ ] **Step 5: Remove the resolved TODOs**

Delete the two comments in `create_spectrogram_buffer` that this work resolves:

```rust
            // TODO: Can we compute the mipmaps in a compute shader instead?
```
```rust
                // TODO: We need to recompute the mipmap if the average/max-hold mode changes...
```

The capability-proxy TODO from Task 2 Step 2 **stays** — it is unresolved.

- [ ] **Step 6: Update the module docs**

The module header in `shader.rs` says the pyramid is built on the CPU. In the paragraph beginning "That leaves the fragment shader with up to 4x supersampling", add after the mode list:

```rust
//! The pyramid is built by a compute shader in [mipmap.wgsl](mipmap.wgsl), one dispatch per level,
//! each reading the level below it. Switching between the two modes re-runs those dispatches, so
//! no pixel data crosses the bus. On adapters without compute support the same pyramid is built on
//! the CPU instead.
```

- [ ] **Step 7: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/shader.rs
git commit -m "$(cat <<'EOF'
perf: re-dispatch instead of re-uploading on mode toggle

Switching max-hold/average now rewrites ~2KB of dispatch parameters per
chunk and re-runs the compute pass, rather than recomputing the pyramid
on the CPU and uploading ~33MB per chunk.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Self-review notes

Two things a reader should know about this plan's limits:

- **The shader has no automated test.** Tasks 1 and 3-4 test the arithmetic around it (`mip_params_bytes`) but not the WGSL. Task 3 Step 8's A/B against the forced CPU fallback is the real verification and should not be skipped.
- **The capability proxy is knowingly incomplete.** A native adapter without `DownlevelFlags::COMPUTE_SHADERS` but with non-zero compute limits would take the compute path and fail at `create_compute_pipeline` rather than falling back. No such adapter is known; the `TODO` records it.
