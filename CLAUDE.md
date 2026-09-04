# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo build --release
cargo run --release [-- <WORKSPACE_FILE>]
cargo run --release -- pass-png --help   # batch pass-PNG subcommand
cargo run --bin rsmedfilt -- --help
cargo +nightly fmt --all  # Always use nightly rustfmt
cargo clippy
cargo test
```

Notable CLI flags (most are global across subcommands): `-v`/`--verbose` (repeatable), `--freq-range MIN MAX` (skip channels outside range at load time), `-W`/`-H` (window size).

System dependencies (Ubuntu): `build-essential libssl-dev pkg-config fontconfig libfontconfig1-dev libopenblas-dev`

A Nix flake is provided for reproducible builds; a Cachix cache at `rstrf.cachix.org` (built from CI) is available so rstrf does not need to be built locally.

`scripts/pass_png_historic.py` batches the `pass-png` subcommand over many days of spectrograms, grouping `.bin` files by recording session (rffft or rsmedfilt naming) and selecting the closest-epoch TLE per group from a historic TLE archive.

## Commits

Commits should ideally stand on their own: from reading a commit message (and the diff), it should be apparent what the commit is trying to do. If implementing a feature takes multiple logical chunks of work, these should be committed in separate commits.

After each commit, everything should compile, all tests should pass, and everything should work. WIP commits may violate this rule if appropriate (e.g. a commit would get too large/be hard to follow).

Commit messages should use the Conventional Commits style, with the types `fix`, `feat`, `build`, `chore`, `ci`, `docs`, `refactor`, `perf`, `test`, and `wip`.

If the message header is not enough to understand what a commit is doing, or why it is doing it, include a body with further explanation. Commit bodies should typically not exceed 1--2 paragraphs, but this is not a hard rule.

Agent-authored commits should **always** include a `Co-authored-by: $MODEL $VERSION <$EMAIL>` footer (e.g. `Co-authored-by: Claude Sonnet 5 <noreply@anthropic.com>`).

## Architecture

rSTRF is a GPU-accelerated satellite radio waterfall spectrogram viewer — a Rust rewrite of the `strf` toolkit's `rfplot`. It displays power-vs-frequency-vs-time spectrograms, overlays Doppler-shifted satellite tracks, and detects signals.

**Two binaries:**
- `src/bin/rstrf/` — the GUI application
- `src/bin/rsmedfilt.rs` — CLI median-filter preprocessor for `.bin` files

**Library crate** (`src/lib.rs` re-exports):
- `spectrogram.rs` — async load/save of STRF `.bin` files; `load_single` loads one file, `load` concatenates multiple with bounded concurrency (`buffer_unordered(8)`); both accept an optional `freq_range: Option<(u64, u64)>` to skip out-of-range channels at load time
- `orbit.rs` — TLE parsing, SGP4 propagation, Doppler prediction, GMST-based site coordinates; each `Satellite` carries a `transmitters: Vec<f64>` for multiple frequencies; predictions are split per pass
- `signal.rs` — `FitTrace` signal detection (frequency peaks above sigma threshold)
- `coord.rs` — type-stated coordinate transforms using `glam::Mat4` + `duplicate` macro (see below)
- `colormap.rs` — GPU-ready `[[f32;4];256]` colormaps (Viridis default, Magma, Turbo, etc.)
- `util.rs` — shared utilities: `minmax`, `to_index`, `clip_line` (Liang–Barsky)

**GUI layer uses iced 0.14 (Elm Architecture / `Daemon` mode):**

```
AppModel
  ├── shared_state: AppShared — satellites, frequencies, config, Space-Track client
  ├── windows: HashMap<window::Id, AnyWindow>
  │     ├── RFPlot window (windows/rfplot/)
  │     │     ├── state: State (mod.rs) — viewport, power, detection, spectrogram, display,
  │     │     │     marks, interaction, prediction cache; plus Message and the update handlers
  │     │     ├── viewport.rs — Viewport: zoom/pan math, clamping, snap-to-bounds
  │     │     ├── marks.rs — Marks (track points + signals), MarkAction
  │     │     ├── predictions.rs — PredictionKey and the pass-prediction cache
  │     │     ├── chart.rs — PlotChart and the plotters-iced2 Chart impl (axes, satellite
  │     │     │     curves, crosshair, marks)
  │     │     ├── interaction.rs — Interaction/MouseState/RectAction and the mouse and
  │     │     │     keyboard handlers
  │     │     ├── toolbar.rs — the toolbar and the collapsible controls panel
  │     │     └── shader::Program (shader.rs + shader.wgsl) — wgpu GPU render
  │     ├── SatManager window (windows/sat_manager.rs) — TLE loading, frequency editing, Space-Track sync
  │     └── preferences::Window (windows/preferences.rs) — Config editing (theme, site coords, credentials)
  └── pass_png: Option<PassPngMode> (pass_png.rs) — headless batch screenshot generator for satellite passes
```

**Loading pipeline:** `io_service.rs` wraps `load_single` in an `iced::Subscription` that streams `Progress { loaded, total }` events as files complete, then a final `Done` event — enabling the progress indicator in the UI.

**Message routing:** `app::Message` → `windows::Message` → window-specific. `windows::Message` has variants `RFPlot(rfplot::Message)`, `SatManager(sat_manager::Message)`, `Preferences(preferences::Message)`, and `ToApp(Box<app::Message>)`.

**Window dispatch via `AnyWindow`:** Windows are stored as a concrete enum (`AnyWindow`) rather than trait objects. Each window uses its own message type internally; the lift to `windows::Message` is centralized via `From<WindowOut<M>>` impls. `AppShared` is passed into `update` and `view` so windows can read shared state without messaging.

**Window effect escaping:** When a window needs to emit something outside its own message type, it returns `WindowOut::Effect(WindowEffect::ToApp(...))` instead of `WindowOut::Msg(...)`. The `From<WindowOut<M>> for windows::Message` impls map these to `Message::ToApp`.

**RFPlot rendering is a two-layer stack:**
1. `widget::shader(rfplot)` — wgpu pipeline uploading spectrogram as chunked storage buffers; colormap lookup in fragment shader (`shader.wgsl`)
2. `ChartWidget` (plotters-iced2) — wraps `PlotChart<'a>` (`chart.rs`), a view-model struct borrowing `&State` and `&AppShared` so the `Chart` trait impl can reach both; its methods delegate to inherent methods on `State` (`build_chart` in `chart.rs`, `handle_mouse`/`handle_keyboard` in `interaction.rs`). Draws axes, grid, Doppler curves coloured by TLE classification, track points (yellow), signal points (white), crosshair readout; supports absolute-axes mode where the y-axis shows raw frequency and grid snaps to absolute-frequency multiples

**Marks (`marks.rs`):** `Marks` owns two `Vec<data_absolute::Point>`. Track points are kept sorted by time — `insert_track_point` is the only way in, and the fields are private to the module so the ordering cannot be broken from outside. Right-clicking the plot deletes the closest mark within `DELETE_TOLERANCE_PX` screen pixels, found via `closest_mark()` in `interaction.rs`.

## Key Patterns

**`AnyWindow` over `Box<dyn Window>` (`windows/mod.rs`):** Windows are stored as a concrete enum (`AnyWindow`) rather than trait objects. This lets each window define its own message type without boxing or lifting internally — the lift to `windows::Message` is done via `From<WindowOut<M>>` impls, not inside each window.

**Coordinate type safety (`coord.rs`):** The `duplicate` macro generates newtyped point types (`screen::Point`, `plot_area::Point`, `data_normalized::Point`, `data_absolute::Point`) and typed transform structs for all 12 pairwise combinations. Coordinate conversion is `point * transform`. This makes coordinate space errors compile errors.

**Serde for persistence:** `Config`, `RFPlot`, `SatManager`, `State`, `Satellite`, `Site` are all `Serialize`/`Deserialize`. Transient state (loaded spectrogram data, computed predictions) uses `#[serde(skip)]`.

**Async I/O:** All file loading and Space-Track API calls use `Task::future(async { ... })`. CPU-intensive work uses `tokio::task::spawn_blocking`.

**Clippy allow:** `filter_map_bool_then` is suppressed globally in `Cargo.toml`.

**Mouse/interaction state via `Cell` (`interaction.rs`):** Transient per-frame UI state that only the view needs (crosshair position, mouse drag state, keyboard modifiers) is stored in `Cell<T>` fields on `State`'s `Interaction` sub-struct and mutated directly from `handle_mouse`/`handle_keyboard` (inherent methods on `State`) and from `PlotChart`'s `Chart::update` impl, rather than round-tripped through `Message` variants. A `Message::Refresh` no-op is dispatched afterwards: `Chart::update` cannot request a repaint, and iced only redraws when a message is published.

**One component per window:** Everything below the window level is data plus functions over it, not a sub-component. `rfplot::Message` groups its variants by the part of `State` they mutate (`View`, `Display`, `Marks`, `Predictions`), each routed to a `State::update_*` method; views (`toolbar::view`, `chart.rs`) and input handlers are free to read across the whole of `State`, since they are read-only and follow layout rather than ownership.
