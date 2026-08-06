# CPU Mipmap Refactor and Mode Toggle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor the CPU mipmap builder into reusable pieces, then make the max-hold/average toggle regenerate the pyramid — phases 1 and 2 of `docs/superpowers/specs/2026-08-07-gpu-mipmap-generation-design.md`.

**Architecture:** Today `create_spectrogram_buffer` writes the pyramid level-by-level straight into a mapped wgpu buffer, and the mipmap layout is duplicated between that writer and the `buf_offset_chan` reader in `update_buffers`. This plan extracts the layout arithmetic into shared helpers used by both, turns the level loop into a `cpu_mipmap_levels` function returning the levels as a contiguous `Vec<f32>`, and then reuses that function to patch levels 1+ in place via `queue.write_buffer` when the plotting mode changes. Phase 3 (compute shader) replaces only the *producer* of those bytes and is out of scope here.

**Tech Stack:** Rust, wgpu 27 via iced 0.14, bytemuck, itertools.

## Global Constraints

- All work is in `src/bin/rstrf/windows/rfplot/shader.rs`. No other file changes. No new dependencies.
- `MIPMAP_FACTOR` is `f64` = 4.0; existing code spells the integer form `MIPMAP_FACTOR as usize`. Follow that.
- Format with `cargo +nightly fmt --all` — nightly rustfmt only.
- `cargo clippy --bin rstrf --all-targets` must introduce no new warnings. The baseline is **5** pre-existing warnings, which must be left alone — `too_many_arguments (8/7)` on `create_buffers` in `shader.rs`, two `manual_range_contains` in `control.rs`, and two in the library.
- Tests run with `cargo test --bin rstrf`. The baseline is **33** passing, of which 8 are the `compute_mipmap` tests in `shader.rs`'s `mod tests`; none may regress.
- The test module already defines these helpers — reuse them, do not redefine:
  - `const MAX_HOLD: bool = false;` and `const AVERAGE: bool = true;`
  - `fn ramp(nslices: usize, nchan: usize) -> Vec<f32>` — `data[x][y] = x * 100 + y`
  - `fn assert_close(actual: &[f32], expected: &[f32])` — elementwise, 1e-3 tolerance
- Commit messages follow conventional commits and end with the trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.

---

### Task 1: Mipmap layout helpers

Extract the pyramid's layout arithmetic so the writer and the reader share one definition. Today `update_buffers` computes level offsets with an inline loop while `create_spectrogram_buffer` derives them implicitly by accumulating as it writes; nothing in the repo checks that the two agree.

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/shader.rs` — add free functions after `mipmap_buffer_size` (line ~76); replace the `buf_offset_chan` loop in `update_buffers` (lines ~242-245); replace the inline `chunk_len` computation in `create_spectrogram_buffers` (lines ~349-363)
- Test: same file, `mod tests` at the end

**Interfaces:**
- Consumes: `MIPMAP_FACTOR`, `mipmap_buffer_size` (both already exist)
- Produces:
  - `fn mipmap_level_nchan(nchan: usize, level: u32) -> usize`
  - `fn mipmap_level_count(nchan: usize) -> u32`
  - `fn mipmap_level_offset_chan(nchan: usize, level: u32) -> usize`
  - `fn mipmap_levels_len(nslices: usize, nchan: usize) -> usize`
  - `fn chunk_len(limits: &wgpu::Limits, spectrogram: &Spectrogram) -> usize`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/bin/rstrf/windows/rfplot/shader.rs`:

```rust
    /// Level offsets and level sizes must describe the same layout: the offset of level L is the
    /// sum of the sizes of every level below it. `update_buffers` reads with the offsets, the
    /// pyramid builder writes with the sizes.
    #[test]
    fn level_offsets_are_the_running_sum_of_level_sizes() {
        for nchan in [4usize, 10, 63, 1024, 1250, 80000] {
            let mut expected = 0;
            for level in 0..=mipmap_level_count(nchan) {
                assert_eq!(
                    mipmap_level_offset_chan(nchan, level),
                    expected,
                    "nchan={}, level={}",
                    nchan,
                    level
                );
                expected += mipmap_level_nchan(nchan, level);
            }
        }
    }

    #[test]
    fn levels_len_covers_every_level_above_zero() {
        let nslices = 7;
        for nchan in [4usize, 10, 63, 1024, 1250, 80000] {
            let past_last = mipmap_level_offset_chan(nchan, mipmap_level_count(nchan) + 1);
            assert_eq!(
                mipmap_levels_len(nslices, nchan),
                nslices * (past_last - nchan),
                "nchan={}",
                nchan
            );
        }
    }

    /// The 4/3 budget in `mipmap_buffer_size` must hold for the levels we actually build.
    #[test]
    fn levels_fit_within_the_mipmap_buffer_budget() {
        let nslices = 7;
        for nchan in [4usize, 10, 63, 1024, 1250, 80000] {
            let data_len = nslices * nchan;
            let budget = mipmap_buffer_size(data_len * std::mem::size_of::<f32>())
                / std::mem::size_of::<f32>();
            assert!(
                data_len + mipmap_levels_len(nslices, nchan) <= budget,
                "nchan={}: {} + {} > {}",
                nchan,
                data_len,
                mipmap_levels_len(nslices, nchan),
                budget
            );
        }
    }

    #[test]
    fn fewer_than_four_channels_has_no_levels() {
        for nchan in 0..4 {
            assert_eq!(mipmap_level_count(nchan), 0, "nchan={}", nchan);
            assert_eq!(mipmap_levels_len(9, nchan), 0, "nchan={}", nchan);
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --bin rstrf shader::`
Expected: FAIL to compile — `cannot find function mipmap_level_count in this scope` (and the other three names).

- [ ] **Step 3: Add the helpers**

Insert directly after `mipmap_buffer_size` (around line 78) in `src/bin/rstrf/windows/rfplot/shader.rs`:

```rust
/// Channels per slice at `level`, where level 0 is the raw data.
fn mipmap_level_nchan(nchan: usize, level: u32) -> usize {
    nchan / (MIPMAP_FACTOR as usize).pow(level)
}

/// Number of mipmap levels above level 0. The chain stops once a level would have no full group
/// of `MIPMAP_FACTOR` input channels left.
fn mipmap_level_count(nchan: usize) -> u32 {
    let mut n = nchan;
    let mut levels = 0;
    while n >= MIPMAP_FACTOR as usize {
        n /= MIPMAP_FACTOR as usize;
        levels += 1;
    }
    levels
}

/// Offset of `level` within one chunk's mipmap block, in channels. Multiply by the chunk's slice
/// count to get the offset in `f32` elements.
fn mipmap_level_offset_chan(nchan: usize, level: u32) -> usize {
    (0..level).map(|i| mipmap_level_nchan(nchan, i)).sum()
}

/// Total `f32` elements occupied by levels 1.. of one chunk.
fn mipmap_levels_len(nslices: usize, nchan: usize) -> usize {
    let mut total = 0;
    let mut n = nchan;
    while n >= MIPMAP_FACTOR as usize {
        n /= MIPMAP_FACTOR as usize;
        total += n;
    }
    nslices * total
}

/// Slices per GPU chunk, bounded by the largest storage buffer the device will bind. Returns 0 if
/// a single slice does not fit.
fn chunk_len(limits: &wgpu::Limits, spectrogram: &Spectrogram) -> usize {
    let max_buf_size =
        (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size) as usize;
    let slice_size = mipmap_buffer_size(spectrogram.nchan * std::mem::size_of::<f32>());
    spectrogram.nslices.min(max_buf_size / slice_size)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --bin rstrf shader::`
Expected: PASS, 12 tests (8 existing + 4 new).

- [ ] **Step 5: Use `mipmap_level_offset_chan` in `update_buffers`**

In `update_buffers`, replace these lines (around 242-245):

```rust
        let mut buf_offset_chan = 0;
        for i in 0..mipmap_level {
            buf_offset_chan += spectrogram.nchan / MIPMAP_FACTOR.powi(i as i32) as usize;
        }
```

with:

```rust
        let buf_offset_chan = mipmap_level_offset_chan(spectrogram.nchan, mipmap_level);
```

Also replace the `mipmap_stride` / `nchan` derivation above it (around lines 226-227):

```rust
        let mipmap_stride = MIPMAP_FACTOR.powi(mipmap_level as i32) as usize;
        let nchan = (spectrogram.nchan / mipmap_stride) as u32;
```

with:

```rust
        let mipmap_stride = (MIPMAP_FACTOR as usize).pow(mipmap_level);
        let nchan = mipmap_level_nchan(spectrogram.nchan, mipmap_level) as u32;
```

`pixel_height / mipmap_stride as f32` on the following line still works unchanged.

- [ ] **Step 6: Use `chunk_len` in `create_spectrogram_buffers`**

Replace lines ~349-363 of `create_spectrogram_buffers`:

```rust
        let limits = device.limits();
        let max_buf_size =
            (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size) as usize;
        let data = spectrogram.data.as_slice().unwrap();
        let slice_size = mipmap_buffer_size(spectrogram.nchan * std::mem::size_of::<f32>());
        let max_chunk_len = max_buf_size / slice_size;
        let chunk_len = spectrogram.nslices.min(max_chunk_len);
        if chunk_len == 0 {
            log::error!(
                "Spectrogram is too large to render ({} bytes per slice, max buffer size is {})",
                slice_size,
                max_buf_size
            );
            return Vec::new();
        }
```

with:

```rust
        let limits = device.limits();
        let data = spectrogram.data.as_slice().unwrap();
        let chunk_len = chunk_len(&limits, spectrogram);
        if chunk_len == 0 {
            log::error!(
                "Spectrogram is too large to render ({} bytes per slice, max buffer size is {})",
                mipmap_buffer_size(spectrogram.nchan * std::mem::size_of::<f32>()),
                (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size),
            );
            return Vec::new();
        }
```

- [ ] **Step 7: Verify the whole binary still builds and passes**

Run: `cargo test --bin rstrf && cargo clippy --bin rstrf --all-targets 2>&1 | grep -c "^warning: [a-z]" && cargo +nightly fmt --all -- --check`
Expected: 37 tests pass; the clippy count prints `5`, the pre-existing baseline; fmt check silent.

- [ ] **Step 8: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/shader.rs
git commit -m "$(cat <<'EOF'
refactor: share mipmap layout arithmetic between writer and reader

The level offsets used by update_buffers and the level sizes used by the
pyramid builder described the same layout in two places, with nothing
checking that they agreed.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `cpu_mipmap_levels`

Turn the pyramid loop inside `create_spectrogram_buffer` into a function that returns the levels as one contiguous `Vec<f32>`, so Task 3 can upload them without a second implementation. Also drop the `f32::MIN` sentinel in favour of seeding max-hold from the first sample, matching what the compute shader will do in phase 3.

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/shader.rs` — replace `compute_mipmap` (lines ~519-544) and the mapped-range block in `create_spectrogram_buffer` (lines ~491-514)
- Test: same file, `mod tests`

**Interfaces:**
- Consumes: `mipmap_levels_len`, `mipmap_level_count`, `mipmap_level_offset_chan` (Task 1)
- Produces:
  - `fn compute_mipmap_into(data: &[f32], nslices: usize, nchan_in: usize, average: bool, out: &mut [f32])`
  - `fn compute_mipmap(data: &[f32], nslices: usize, nchan_in: usize, average: bool) -> Vec<f32>` — unchanged signature, now a wrapper
  - `fn cpu_mipmap_levels(data: &[f32], nslices: usize, nchan: usize, average: bool) -> Vec<f32>`

All three are associated functions on `impl Pipeline`, called as `Pipeline::…` from tests.

- [ ] **Step 1: Write the failing test**

Add to `mod tests`:

```rust
    /// Each level must land at the offset `update_buffers` will read it from, and hold exactly what
    /// aggregating the level below it produces. `nchan = 20` so that level 2 has a partial group.
    #[test]
    fn cpu_mipmap_levels_places_each_level_at_its_offset() {
        for average in [MAX_HOLD, AVERAGE] {
            let (nslices, nchan) = (3usize, 20usize);
            let data = ramp(nslices, nchan);
            let levels = Pipeline::cpu_mipmap_levels(&data, nslices, nchan, average);

            let mut prev = data.clone();
            let mut nchan_in = nchan;
            for level in 1..=mipmap_level_count(nchan) {
                let expected = Pipeline::compute_mipmap(&prev, nslices, nchan_in, average);
                // Offsets are absolute within the chunk; `levels` starts after level 0.
                let start = nslices * mipmap_level_offset_chan(nchan, level) - data.len();
                assert_close(&levels[start..start + expected.len()], &expected);
                prev = expected;
                nchan_in /= 4;
            }
            assert_eq!(levels.len(), mipmap_levels_len(nslices, nchan));
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --bin rstrf cpu_mipmap_levels`
Expected: FAIL to compile — `no function or associated item named cpu_mipmap_levels found`.

- [ ] **Step 3: Replace `compute_mipmap` with the `_into` form plus a wrapper**

Replace the whole of `compute_mipmap` (lines ~519-544) with:

```rust
    fn compute_mipmap_into(
        data: &[f32],
        nslices: usize,
        nchan_in: usize,
        average: bool,
        out: &mut [f32],
    ) {
        let stride = MIPMAP_FACTOR as usize;
        let nchan_out = nchan_in / stride;
        for x in 0..nslices {
            for y in 0..nchan_out {
                let src = x * nchan_in + y * stride;
                out[x * nchan_out + y] = if average {
                    let mut sum = 0.0;
                    for i in 0..stride {
                        sum += data[src + i];
                    }
                    sum / stride as f32
                } else {
                    // Seed from the first sample rather than a sentinel, so no magic constant has
                    // to stay in sync with the compute shader.
                    let mut max = data[src];
                    for i in 1..stride {
                        max = max.max(data[src + i]);
                    }
                    max
                };
            }
        }
    }

    fn compute_mipmap(data: &[f32], nslices: usize, nchan_in: usize, average: bool) -> Vec<f32> {
        let mut out = vec![0.0; nslices * (nchan_in / MIPMAP_FACTOR as usize)];
        Self::compute_mipmap_into(data, nslices, nchan_in, average, &mut out);
        out
    }

    /// Levels 1.. concatenated, laid out exactly as they sit in `spec_data` after level 0.
    fn cpu_mipmap_levels(data: &[f32], nslices: usize, nchan: usize, average: bool) -> Vec<f32> {
        let stride = MIPMAP_FACTOR as usize;
        let mut out = vec![0.0; mipmap_levels_len(nslices, nchan)];

        // Level 1 reads the caller's data; every level after reads the one before it out of `out`.
        let mut nchan_in = nchan;
        let mut src = 0..0;
        let mut written = 0;
        while nchan_in >= stride {
            let len = nslices * (nchan_in / stride);
            let (done, rest) = out.split_at_mut(written);
            let src_data: &[f32] = if written == 0 { data } else { &done[src.clone()] };
            Self::compute_mipmap_into(src_data, nslices, nchan_in, average, &mut rest[..len]);
            src = written..written + len;
            written += len;
            nchan_in /= stride;
        }
        out
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --bin rstrf shader::`
Expected: PASS, 13 tests. In particular `max_hold_handles_all_negative_values` still passes — seeding from sample 0 is what makes that correct without `f32::MIN`.

- [ ] **Step 5: Use `cpu_mipmap_levels` in `create_spectrogram_buffer`**

Replace the mapped-range block (lines ~491-514) — everything from `{` after the `create_buffer` call through the closing `}` before `buffer.unmap();`:

```rust
        {
            // Need to drop the mapped views before unmapping
            let mut view = buffer.slice(..).get_mapped_range_mut();
            let floats: &mut [f32] = bytemuck::cast_slice_mut(&mut view[..]);
            floats[..data.len()].copy_from_slice(bytemuck::cast_slice(data));
            let levels = Self::cpu_mipmap_levels(data, nslices, nchan, average);
            floats[data.len()..][..levels.len()].copy_from_slice(&levels);
        }
```

Then change the `mut nchan: usize` parameter of `create_spectrogram_buffer` to `nchan: usize` — the loop that mutated it is gone.

- [ ] **Step 6: Verify**

Run: `cargo test --bin rstrf && cargo clippy --bin rstrf --all-targets && cargo +nightly fmt --all -- --check`
Expected: 38 tests pass, no new clippy warnings, fmt silent.

- [ ] **Step 7: Manual check — the rendering is unchanged**

Run: `cargo run --release -- <a .bin file>`
Expected: the waterfall looks identical to before this task at every zoom level, in both max-hold and average mode. This task is a pure refactor; any visible change is a bug.

- [ ] **Step 8: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/shader.rs
git commit -m "$(cat <<'EOF'
refactor: extract cpu_mipmap_levels from buffer creation

Returning the pyramid as one contiguous Vec lets the mode toggle upload
it without a second implementation. Max-hold now seeds from the first
sample instead of f32::MIN, matching what the compute shader will do.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Regenerate the pyramid when the plotting mode changes

`average` is baked into the pyramid at buffer-creation time, so toggling max-hold/average currently leaves stale mipmap data and only changes what the fragment shader does with the ≤4 samples it reads. This closes that TODO on the CPU path. Phase 3 will add a fast GPU arm alongside it.

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/shader.rs` — `SpectrogramChunk` (lines ~103-109), `PrimitiveData` (lines ~117-122), `create_buffers` (lines ~325-340), the `SpectrogramChunk` construction in `create_spectrogram_buffers` (lines ~455-461), `update_buffers` (after the colormap block, around line 296), plus a new `repatch_mipmaps_cpu`

**Interfaces:**
- Consumes: `chunk_len` (Task 1), `cpu_mipmap_levels` (Task 2)
- Produces: `fn repatch_mipmaps_cpu(device: &wgpu::Device, queue: &wgpu::Queue, spectrogram: &Spectrogram, chunks: &[SpectrogramChunk], average: bool)`

- [ ] **Step 1: Retain the spectrogram buffer handle**

`as_entire_binding()` borrows rather than moves, so the buffer is simply dropped at the end of the closure today. Add it to the struct instead.

In `SpectrogramChunk` (lines ~103-109) add a field:

```rust
struct SpectrogramChunk {
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    instances: wgpu::Buffer,
    /// Retained so the mode toggle can patch mipmap levels in place.
    spectrogram: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    nslices: u32,
}
```

and in the `SpectrogramChunk { … }` literal at the end of the `create_spectrogram_buffers` closure (lines ~455-461) add `spectrogram: spectrogram_buffer,` — after the `create_bind_group` call, which borrows it.

- [ ] **Step 2: Track `average` on `PrimitiveData`**

In `PrimitiveData` (lines ~117-122):

```rust
struct PrimitiveData {
    buffers: Buffers,
    spectrogram_id: Uuid,
    colormap: Colormap,
    average: bool,
    depth: DepthTarget,
}
```

In `create_buffers`, which already takes an `average: bool` parameter, add `average,` to the `PrimitiveData { … }` literal (around line 336, next to `colormap,`).

- [ ] **Step 3: Run the build to verify it compiles**

Run: `cargo build --bin rstrf`
Expected: success. If `create_spectrogram_buffers` errors with "missing field `spectrogram`", Step 1's literal was not updated.

- [ ] **Step 4: Add `repatch_mipmaps_cpu`**

Insert after `cpu_mipmap_levels` in `impl Pipeline`:

```rust
    /// Recompute levels 1.. for the current plotting mode and upload them over the existing ones.
    /// Level 0 is untouched, and the buffer objects are unchanged, so bind groups stay valid.
    fn repatch_mipmaps_cpu(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        spectrogram: &Spectrogram,
        chunks: &[SpectrogramChunk],
        average: bool,
    ) {
        let chunk_len = chunk_len(&device.limits(), spectrogram);
        if chunk_len == 0 {
            return;
        }
        let data = spectrogram.data.as_slice().unwrap();
        for (chunk_data, chunk) in izip!(data.chunks(chunk_len * spectrogram.nchan), chunks) {
            let levels = Self::cpu_mipmap_levels(
                chunk_data,
                chunk.nslices as usize,
                spectrogram.nchan,
                average,
            );
            queue.write_buffer(
                &chunk.spectrogram,
                std::mem::size_of_val(chunk_data) as u64,
                bytemuck::cast_slice(&levels),
            );
        }
    }
```

The write offset is the size of that chunk's level 0, which is exactly where `cpu_mipmap_levels`' output begins.

- [ ] **Step 5: Hook up the invalidation in `update_buffers`**

In the `else if primitive_data.spectrogram_id != spectrogram.id` arm (around line 276), the buffers are rebuilt with the current mode, so record it. Add next to `primitive_data.spectrogram_id = spectrogram.id;`:

```rust
            primitive_data.average = primitive.controls.average_plotting();
```

Then, immediately after the colormap block that ends around line 296, add:

```rust
        let average = primitive.controls.average_plotting();
        if primitive_data.average != average {
            Self::repatch_mipmaps_cpu(
                device,
                queue,
                spectrogram,
                &primitive_data.buffers.spectrogram,
                average,
            );
            primitive_data.average = average;
        }
```

- [ ] **Step 6: Verify it builds and the tests still pass**

Run: `cargo test --bin rstrf && cargo clippy --bin rstrf --all-targets && cargo +nightly fmt --all -- --check`
Expected: 38 tests pass, no new clippy warnings, fmt silent.

- [ ] **Step 7: Manual check — the toggle changes the picture**

There is no automated coverage for this; it needs a GPU and a real spectrogram.

Run: `cargo run --release -- <a .bin file with narrow-band signals>`

1. Zoom out on the frequency axis far enough that the mipmap engages — `RUST_LOG=trace` and watch for `mipmap level 1` or higher in the `Updating buffers for primitive` line.
2. Toggle the average/max-hold control.

Expected: narrow-band carriers visibly dim in average mode and pop back in max-hold. Before this task the zoomed-out picture barely changed, because only the fragment shader's ≤4 samples responded to the toggle while the mipmap stayed as built. Expect a stall of a few hundred ms on a large workspace — that is the cost phase 3 removes.

- [ ] **Step 8: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/shader.rs
git commit -m "$(cat <<'EOF'
feat: regenerate the mipmap when the plotting mode changes

The pyramid had max-hold or average baked in at creation, so toggling
the mode left stale mipmap levels and only affected the handful of
samples the fragment shader reads per pixel.

Recomputing on the CPU costs a few hundred ms on a large workspace; the
compute-shader path in phase 3 makes it a re-dispatch with no upload.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Not in this plan

Phase 3 of the spec — `mipmap.wgsl`, the compute pipeline, `ChunkMipmap`, `encode_mipmap_pass`, `redispatch_mipmaps`, and the compute-capability branch — gets its own plan. It replaces only the producer of the mipmap bytes; `cpu_mipmap_levels` and `repatch_mipmaps_cpu` become the fallback arm unchanged.
