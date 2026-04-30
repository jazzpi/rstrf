# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo build --release
cargo run --release [-- <WORKSPACE_FILE>]
cargo run --bin rsmedfilt -- --help
cargo clippy
```

System dependencies (Ubuntu): `build-essential libssl-dev pkg-config fontconfig libfontconfig1-dev libopenblas-dev`

A Nix flake is provided for reproducible builds. There is no test suite.

## Architecture

rSTRF is a GPU-accelerated satellite radio waterfall spectrogram viewer — a Rust rewrite of the `strf` toolkit's `rfplot`. It displays power-vs-frequency-vs-time spectrograms, overlays Doppler-shifted satellite tracks, and detects signals.

**Two binaries:**
- `src/bin/rstrf/` — the GUI application
- `src/bin/rsmedfilt.rs` — CLI median-filter preprocessor for `.bin` files

**Library crate** (`src/lib.rs` re-exports):
- `spectrogram.rs` — async load/save of STRF `.bin` files, dB conversion, multi-file concatenation
- `orbit.rs` — TLE parsing, SGP4 propagation, Doppler prediction, GMST-based site coordinates
- `signal.rs` — `FitTrace` signal detection (frequency peaks above sigma threshold)
- `coord.rs` — type-stated coordinate transforms using `glam::Mat4` + `duplicate` macro (see below)
- `colormap.rs` — GPU-ready `[[f32;4];256]` colormaps (Magma, Viridis, Turbo, etc.)

**GUI layer uses iced 0.14 (Elm Architecture / `Daemon` mode):**

```
AppModel
  └── windows: HashMap<window::Id, Box<dyn Window>>
        ├── workspace::Window
        │     └── Workspace
        │           └── PaneGridState (= pane_grid::State<AnyPane>)
        │                 ├── RFPlot pane (panes/rfplot/)
        │                 │     ├── SharedState (Controls, Spectrogram)
        │                 │     ├── Overlay (overlay.rs) — axes, satellite curves, crosshair
        │                 │     └── shader::Program (shader.rs + shader.wgsl) — wgpu GPU render
        │                 ├── SatManager pane — TLE loading, frequency editing, Space-Track sync
        │                 └── Dummy pane — bootstrapping placeholder
        └── preferences::Window — Config editing (theme, site coords, credentials)
```

**Message routing:** `app::Message` → `windows::Message` → `PaneMessage { id, message: panes::Message }` → pane-specific. `panes::Message` has variants `RFPlot(rfplot::Message)`, `SatManager(sat_manager::Message)`, `ToWorkspace(workspace::Message)`, `ToApp(Box<app::Message>)`, and `ReplacePane(Pane)`.

**Pane dispatch via `AnyPane`:** `PaneGridState` holds `AnyPane`, a concrete enum over all pane types (`RFPlot`, `SatManager`, `Dummy`). Update logic (`init`, `update`, `workspace_event`) is dispatched through `AnyPane`, not through the `PaneWidget` trait — this lets each pane use its own message type internally. The single lift from a pane-local message type to `panes::Message` happens in `AnyPane::update`, not inside each pane. `PaneWidget` is only for rendering/serialization (`view`, `title`, `to_tree`).

**Pane effect escaping:** When a pane needs to send a message outside its own type (e.g., SatManager triggering a workspace event), it returns `PaneOut::Effect(PaneEffect::ToWorkspace(...))` instead of `PaneOut::Msg(...)`. `AnyPane::update` maps these to the appropriate `panes::Message` variant.

**RFPlot rendering is a two-layer stack:**
1. `widget::shader(rfplot)` — wgpu pipeline uploading spectrogram as chunked storage buffers with offscreen culling; colormap lookup in fragment shader (`shader.wgsl`)
2. `ChartWidget` (plotters-iced2) — draws axes, grid, Doppler curves (green), track points (yellow), signal points (white), crosshair readout

**Workspace persistence:** `PaneTree` (Split/Leaf enum) is serialized to JSON. Reconstruction from `PaneTree` to `pane_grid::State` is done iteratively (see comment in `panes/mod.rs` — `from_configuration` is insufficient).

## Key Patterns

**`AnyPane` over `Box<dyn Pane>` (`panes/mod.rs`):** Panes are stored as a concrete enum (`AnyPane`) rather than trait objects. This lets each pane define its own message type without boxing or lifting internally — the lift to `panes::Message` is centralized in `AnyPane::update`. The `PaneWidget` trait exists only for the rendering/serialization surface (`view`, `title`, `to_tree`).

**Coordinate type safety (`coord.rs`):** The `duplicate` macro generates newtyped point types (`screen::Point`, `plot_area::Point`, `data_normalized::Point`, `data_absolute::Point`) and typed transform structs for all 12 pairwise combinations. Coordinate conversion is `point * transform`. This makes coordinate space errors compile errors.

**Serde for persistence:** `Workspace`, `Config`, `RFPlot`, `SatManager`, `Controls`, `Overlay`, `Satellite`, `Site` are all `Serialize`/`Deserialize`. Transient state (loaded spectrogram data, computed predictions) uses `#[serde(skip)]`.

**Async I/O:** All file loading and Space-Track API calls use `Task::future(async { ... })`. CPU-intensive work uses `tokio::task::spawn_blocking`.

**Clippy allow:** `filter_map_bool_then` is suppressed globally in `Cargo.toml`.
