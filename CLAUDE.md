# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo build --release
cargo run --release [-- <WORKSPACE_FILE>]
cargo run --bin rsmedfilt -- --help
cargo clippy
cargo test
```

System dependencies (Ubuntu): `build-essential libssl-dev pkg-config fontconfig libfontconfig1-dev libopenblas-dev`

A Nix flake is provided for reproducible builds.

## Architecture

rSTRF is a GPU-accelerated satellite radio waterfall spectrogram viewer — a Rust rewrite of the `strf` toolkit's `rfplot`. It displays power-vs-frequency-vs-time spectrograms, overlays Doppler-shifted satellite tracks, and detects signals.

**Three binaries:**
- `src/bin/rstrf/` — the GUI application
- `src/bin/rsbinfmt.rs` — CLI pre-converter: strf `.bin` → `.rstrf`; thin wrapper around `spectrogram::resample_strf` (useful for faster subsequent loads; not required)
- `src/bin/rsmedfilt.rs` — CLI median-filter preprocessor for `.bin` files

**Library crate** (`src/lib.rs` re-exports):
- `spectrogram.rs` — async load/save of STRF `.bin` and `.rstrf` files; `resample_strf` resamples raw spectra onto a uniform time grid (median gap → slice length, gaps filled with `FILL_DB`); `load` routes `.bin` files through a combined resample pass and `.rstrf` files through direct decode, then concatenates
- `orbit.rs` — TLE parsing, SGP4 propagation, Doppler prediction, GMST-based site coordinates
- `signal.rs` — `FitTrace` signal detection (frequency peaks above sigma threshold)
- `coord.rs` — type-stated coordinate transforms using `glam::Mat4` + `duplicate` macro (see below)
- `colormap.rs` — GPU-ready `[[f32;4];256]` colormaps (Magma, Viridis, Turbo, etc.)

**GUI layer uses iced 0.14 (Elm Architecture / `Daemon` mode):**

```
AppModel
  ├── shared_state: AppShared — satellites, frequencies, config, Space-Track client
  └── windows: HashMap<window::Id, AnyWindow>
        ├── RFPlot window (windows/rfplot/)
        │     ├── Controls, Spectrogram
        │     ├── Overlay (overlay.rs) — axes, satellite curves, crosshair
        │     └── shader::Program (shader.rs + shader.wgsl) — wgpu GPU render
        ├── SatManager window (windows/sat_manager.rs) — TLE loading, frequency editing, Space-Track sync
        └── preferences::Window (windows/preferences.rs) — Config editing (theme, site coords, credentials)
```

**Message routing:** `app::Message` → `windows::Message` → window-specific. `windows::Message` has variants `RFPlot(rfplot::Message)`, `SatManager(sat_manager::Message)`, `Preferences(preferences::Message)`, and `ToApp(Box<app::Message>)`.

**Window dispatch via `AnyWindow`:** Windows are stored as a concrete enum (`AnyWindow`) rather than trait objects. Each window uses its own message type internally; the lift to `windows::Message` is centralized via `From<WindowOut<M>>` impls. `AppShared` is passed into `update` and `view` so windows can read shared state without messaging.

**Window effect escaping:** When a window needs to emit something outside its own message type, it returns `WindowOut::Effect(WindowEffect::ToApp(...))` instead of `WindowOut::Msg(...)`. The `From<WindowOut<M>> for windows::Message` impls map these to `Message::ToApp`.

**RFPlot rendering is a two-layer stack:**
1. `widget::shader(rfplot)` — wgpu pipeline uploading spectrogram as chunked storage buffers with offscreen culling; colormap lookup in fragment shader (`shader.wgsl`)
2. `ChartWidget` (plotters-iced2) — draws axes, grid, Doppler curves (green), track points (yellow), signal points (white), crosshair readout

## Key Patterns

**`AnyWindow` over `Box<dyn Window>` (`windows/mod.rs`):** Windows are stored as a concrete enum (`AnyWindow`) rather than trait objects. This lets each window define its own message type without boxing or lifting internally — the lift to `windows::Message` is done via `From<WindowOut<M>>` impls, not inside each window.

**Coordinate type safety (`coord.rs`):** The `duplicate` macro generates newtyped point types (`screen::Point`, `plot_area::Point`, `data_normalized::Point`, `data_absolute::Point`) and typed transform structs for all 12 pairwise combinations. Coordinate conversion is `point * transform`. This makes coordinate space errors compile errors.

**Serde for persistence:** `Config`, `RFPlot`, `SatManager`, `Controls`, `Overlay`, `Satellite`, `Site` are all `Serialize`/`Deserialize`. Transient state (loaded spectrogram data, computed predictions) uses `#[serde(skip)]`.

**Async I/O:** All file loading and Space-Track API calls use `Task::future(async { ... })`. CPU-intensive work uses `tokio::task::spawn_blocking`.

**Clippy allow:** `filter_map_bool_then` is suppressed globally in `Cargo.toml`.
