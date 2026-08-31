# RFPlot Window State Merge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix two blocked features in the RFPlot window (chart draw code can't see the satellite catalog; the toolbar can't see the marks) by merging `RFPlot`'s three loosely-coupled pieces (`Controls`, `Overlay`, `SharedState`) into one state tree, then land the two features as one-line changes on top of it.

**Architecture:** `Overlay` is currently a second "component" (its own struct, `Default` impl, `PartialEq` impl, and `update()`/`build_chart()`/`handle_mouse()`/`handle_keyboard()` methods) nested inside `RFPlot`, even though nothing dispatches it independently — it's just a bag of fields that `RFPlot` and `Controls` already reach into directly. This plan relocates `Overlay`'s fields onto the existing `SharedState` struct (grouped into `Display`/`Marks`/`Interaction` sub-structs), deletes `Overlay` as a type, and renames `SharedState` to `State`. Separately, it introduces a `PlotChart` view-model so the `plotters_iced2::Chart` implementation can borrow `AppShared` (needed to read the satellite catalog) without threading it through `RFPlot` itself.

**Tech Stack:** Rust, iced 0.14 (Daemon/Elm architecture), plotters + plotters-iced2, serde.

**Spec:** none — this plan is self-contained; it does not depend on any other document in the repo.

## Global Constraints

- Format with `cargo +nightly fmt --all` (nightly rustfmt only — stable rustfmt will reformat differently and create noise diffs).
- `cargo clippy` must stay clean (the crate suppresses `filter_map_bool_then` globally in `Cargo.toml`; no other lints are suppressed).
- `cargo test` must pass after every task.
- `RFPlot`, `SharedState`, `Overlay`, and `Controls` all derive `Serialize`/`Deserialize`, but no live code path in this binary actually saves or loads that data: the only files that call `serde_json::from_reader`/`to_string` on window state (`src/bin/rstrf/workspace.rs`, `src/bin/rstrf/windows/workspace.rs`) reference a `crate::panes` module that does not exist and are not pulled in by any `mod` declaration, so they are not part of the compiled binary. Renaming/moving fields in this plan does not need `#[serde(alias)]` shims or any on-disk migration — verify this is still true before each task by running `cargo build` and confirming those two files produce no errors (they shouldn't produce anything at all, since they aren't compiled).
- Only `Config` (`src/bin/rstrf/app.rs`, `load_config`/`save_config`) is actually persisted to disk.

## Manual Verification Checklist

Several tasks in this plan are pure relocations with no intended behavior change. For those, "passing tests" isn't sufficient proof — `cargo test` doesn't cover GUI rendering or interaction, and the only way to confirm nothing broke is to actually look at the running app.

**The agent must never run `cargo run --release` (or any other command that opens the GUI window) itself** — the app is a real window on the user's desktop (`DISPLAY=:0`), and there is no `xdotool`/`scrot`/`import` in this environment for the agent to drive it or see it, so an agent-launched window is just an uncontrolled popup with no way to close the loop. Whenever a task below reaches a manual-verification step, stop and hand off to the user instead of running anything: ask them to run `cargo run --release` themselves with a spectrogram loaded, and to confirm the items below still work exactly as before the task. Wait for their confirmation before moving on to the next task.

1. Pan (drag) and zoom (scroll wheel, over plot / over each axis) the spectrogram.
2. Toggle grid, predictions, crosshair, and absolute/relative axes (toolbar buttons).
3. Mark a track point and a signal (toolbar buttons + click on plot); right-click a mark to delete it.
4. Draw a delete-rect (`d` + drag), a zoom-rect (`z` + drag), and a mark-centroid-rect (`m` + drag).
5. Press `r` to reset the view; confirm marks are cleared.
6. Save signals (toolbar button) and confirm the file dialog / `out.dat` write still works.
7. Reset view, load a new spectrogram, confirm predictions still populate (if satellites + site are configured).

Referred to below as "the manual verification checklist." Every step in this plan that says "run the manual verification checklist" means: stop, ask the user to do the above themselves, and wait for their go-ahead — never run `cargo run` for this purpose.

---

### Task 1: Introduce `PlotChart`, a borrowing view-model, and move the `Chart` impl onto it

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/mod.rs`
- Modify: `src/bin/rstrf/windows/rfplot/overlay.rs`

**Interfaces:**
- Produces: `struct PlotChart<'a> { rfplot: &'a RFPlot }`, defined in `mod.rs`, constructed at the one call site in `RFPlot::view()`.

This is a pure relocation: the `Chart` trait's three methods move from `impl Chart<super::Message> for RFPlot` to `impl Chart<super::Message> for PlotChart<'_>`, with every `self.shared`/`self.overlay` becoming `self.rfplot.shared`/`self.rfplot.overlay`. No behavior changes.

- [ ] **Step 1: Add the `PlotChart` struct to `mod.rs`**

Add this directly above `impl Window<Message> for RFPlot {` (currently the line right after the `RFPlot::app_event`/`apply_initial_view` block):

```rust
struct PlotChart<'a> {
    rfplot: &'a RFPlot,
}
```

- [ ] **Step 2: Update the call site in `RFPlot::view()`**

In `mod.rs`, find:

```rust
        let plot_overlay: Element<'_, Message> = ChartWidget::new(self)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
```

Change `ChartWidget::new(self)` to `ChartWidget::new(PlotChart { rfplot: self })`.

- [ ] **Step 3: Update the import list in `overlay.rs`**

Find:

```rust
use super::{MouseState, RFPlot, RectAction, SharedState, control};
```

Change `RFPlot` to `PlotChart` (it's the only remaining use of the bare `RFPlot` name in this file):

```rust
use super::{MouseState, PlotChart, RectAction, SharedState, control};
```

- [ ] **Step 4: Move the `Chart` impl onto `PlotChart`**

Replace the entire `impl Chart<super::Message> for RFPlot { ... }` block (the block containing `build_chart`, `update`, and `mouse_interaction`) with:

```rust
impl Chart<super::Message> for PlotChart<'_> {
    type State = ();

    fn build_chart<DB: DrawingBackend>(&self, _state: &Self::State, chart: ChartBuilder<DB>) {
        match self.rfplot.overlay.build_chart(chart, &self.rfplot.shared) {
            Ok(()) => (),
            Err(e) => log::error!("Error building chart: {:?}", e),
        }
    }

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<super::Message>) {
        let bounds = Rectangle {
            x: bounds.x + self.rfplot.shared.plot_area_margin,
            y: bounds.y,
            width: bounds.width - self.rfplot.shared.plot_area_margin,
            height: bounds.height - self.rfplot.shared.plot_area_margin,
        };
        match event {
            canvas::Event::Mouse(event) => {
                self.rfplot
                    .overlay
                    .handle_mouse(event, bounds, cursor, &self.rfplot.shared)
            }
            canvas::Event::Keyboard(event) => {
                if let keyboard::Event::ModifiersChanged(modifiers) = event {
                    self.rfplot.overlay.modifiers.set(*modifiers);
                    return (Status::Ignored, None);
                }
                self.rfplot
                    .overlay
                    .handle_keyboard(event, bounds, cursor, &self.rfplot.shared)
            }
            _ => {
                log::debug!("{:?}", event);
                (Status::Ignored, None)
            }
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            match self.rfplot.overlay.mouse_state.get() {
                MouseState::Idle => mouse::Interaction::Idle,
                MouseState::Panning(_) => mouse::Interaction::Grabbing,
                MouseState::DrawingRect { .. } | MouseState::Marking(_) => {
                    mouse::Interaction::Crosshair
                }
            }
        } else {
            mouse::Interaction::Idle
        }
    }
}
```

- [ ] **Step 5: Build and test**

Run: `cargo build --release && cargo test && cargo clippy`
Expected: builds clean, all existing tests pass, no new clippy warnings. This task only renames field-access paths (`self.X` → `self.rfplot.X`) — if it compiles, the behavior is provably identical, so no manual checkpoint is needed for this task.

- [ ] **Step 6: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/mod.rs src/bin/rstrf/windows/rfplot/overlay.rs
git commit -m "refactor: move rfplot Chart impl onto a PlotChart view-model"
```

---

### Task 2: Thread `AppShared` into `PlotChart`

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/mod.rs`

**Interfaces:**
- Consumes: `PlotChart<'a>` from Task 1.
- Produces: `PlotChart<'a> { rfplot: &'a RFPlot, app: &'a AppShared }` — the `app` field is what Task 3 will read.

`AppShared` is already imported in `mod.rs` (`use crate::{app::{AppEvent, AppShared}, ...}`), and `RFPlot::view(&self, app: &AppShared)` already has it in scope at the one call site.

- [ ] **Step 1: Add the field**

```rust
struct PlotChart<'a> {
    rfplot: &'a RFPlot,
    app: &'a AppShared,
}
```

- [ ] **Step 2: Update the call site**

```rust
ChartWidget::new(PlotChart { rfplot: self, app })
```

(field-init shorthand — `app` is already the name of `view`'s parameter.)

- [ ] **Step 3: Build**

Run: `cargo build --release`
Expected: builds with a `field \`app\` is never read` (dead_code) warning — this is expected and resolved by Task 3, not a mistake.

- [ ] **Step 4: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/mod.rs
git commit -m "refactor: thread AppShared into PlotChart"
```

---

### Task 3: Color predictions by satellite classification

**Files:**
- Modify: `src/bin/rstrf/app.rs`
- Modify: `src/bin/rstrf/windows/rfplot/overlay.rs`
- Modify: `src/bin/rstrf/windows/rfplot/mod.rs`

**Interfaces:**
- Consumes: `PlotChart.app: &AppShared` from Task 2; `Satellite.elements.classification: sgp4::Classification` (already exists on `rstrf::orbit::Satellite`, via the `sgp4` crate, which is already a direct dependency of this package and needs no new `Cargo.toml` entry).
- Produces: `AppShared::satellite_classification(&self, norad_id: u64) -> Option<sgp4::Classification>`; `prediction_color(classification: Option<sgp4::Classification>) -> RGBColor` (private to `overlay.rs`).

Renders orbit predictions in orange for `sgp4::Classification::Classified` satellites and red for `sgp4::Classification::Secret`, leaving `Unclassified` (and satellites not found in the catalog) as the current green.

- [ ] **Step 1: Write the failing test for `AppShared::satellite_classification`**

Add to `src/bin/rstrf/app.rs` (there is no existing `#[cfg(test)]` module in this file — add one at the end):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn test_satellite() -> Satellite {
        let line1 = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
        let line2 = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";
        Satellite::from_tle(Some("V1".to_string()), line1, line2, &HashMap::new()).unwrap()
    }

    #[test]
    fn satellite_classification_finds_matching_norad_id() {
        let mut shared = AppShared::default();
        shared.satellites = vec![(test_satellite(), true)];
        assert_eq!(
            shared.satellite_classification(5),
            Some(sgp4::Classification::Unclassified)
        );
    }

    #[test]
    fn satellite_classification_returns_none_for_unknown_id() {
        let shared = AppShared::default();
        assert_eq!(shared.satellite_classification(99999), None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --bin rstrf satellite_classification`
Expected: FAIL with "no method named `satellite_classification` found"

- [ ] **Step 3: Implement `AppShared::satellite_classification`**

Add to the existing `impl AppShared { ... }` block in `app.rs`, alongside `active_satellites`/`active_satellite_ids`/`site`:

```rust
    pub fn satellite_classification(&self, norad_id: u64) -> Option<sgp4::Classification> {
        self.satellites
            .iter()
            .find(|(sat, _)| sat.norad_id() == norad_id)
            .map(|(sat, _)| sat.elements.classification.clone())
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --bin rstrf satellite_classification`
Expected: PASS

- [ ] **Step 5: Write the failing test for `prediction_color`**

Add to the existing `#[cfg(test)] mod tests` block in `overlay.rs` (it already exists, starting around the `closest_mark` tests):

```rust
    #[test]
    fn prediction_color_maps_classification_to_expected_rgb() {
        assert_eq!(prediction_color(None), GREEN);
        assert_eq!(
            prediction_color(Some(sgp4::Classification::Unclassified)),
            GREEN
        );
        assert_eq!(
            prediction_color(Some(sgp4::Classification::Classified)),
            ORANGE
        );
        assert_eq!(prediction_color(Some(sgp4::Classification::Secret)), RED);
    }
```

- [ ] **Step 6: Run the test to verify it fails**

Run: `cargo test --bin rstrf prediction_color`
Expected: FAIL with "cannot find function `prediction_color`" / "cannot find value `ORANGE`"

- [ ] **Step 7: Implement `prediction_color` and the `ORANGE` constant**

Add near the top of `overlay.rs`, after the `DELETE_TOLERANCE_PX` constant:

```rust
const ORANGE: RGBColor = RGBColor(255, 165, 0);

fn prediction_color(classification: Option<sgp4::Classification>) -> RGBColor {
    match classification {
        Some(sgp4::Classification::Secret) => RED,
        Some(sgp4::Classification::Classified) => ORANGE,
        Some(sgp4::Classification::Unclassified) | None => GREEN,
    }
}
```

(`RED`/`GREEN`/`RGBColor` are already in scope via the existing `use plotters::prelude::*;`. `sgp4::Classification` is referenced fully-qualified, so no new `use` is needed — `sgp4` is already a direct dependency of this package.)

- [ ] **Step 8: Run the test to verify it passes**

Run: `cargo test --bin rstrf prediction_color`
Expected: PASS

- [ ] **Step 9: Wire the color into `build_chart`**

In `overlay.rs`, change the `build_chart` signature (inside `impl Overlay`) from:

```rust
    fn build_chart<DB: DrawingBackend>(
        &self,
        mut chart: ChartBuilder<DB>,
        shared: &SharedState,
    ) -> Result<(), String> {
```

to:

```rust
    fn build_chart<DB: DrawingBackend>(
        &self,
        mut chart: ChartBuilder<DB>,
        shared: &SharedState,
        app: &AppShared,
    ) -> Result<(), String> {
```

Then, in the predictions-drawing loop, change:

```rust
            for prediction in predictions.iter_satellites() {
                let (id, passes) = prediction;
                log::trace!("Plotting {} passes for satellite {}", passes.len(), id);
```

to compute the color once per satellite:

```rust
            for prediction in predictions.iter_satellites() {
                let (id, passes) = prediction;
                let color = prediction_color(app.satellite_classification(id));
                log::trace!("Plotting {} passes for satellite {}", passes.len(), id);
```

and change both draw calls inside that loop from `&GREEN` to `&color`:

```rust
                        chart
                            .draw_series(LineSeries::new(
                                izip!(time.iter(), freq.iter())
                                    .map(|(&t, &f)| (t as f32, (f as f32 - spectrogram.freq))),
                                &color,
                            ))
                            .map_err(|e| {
                                format!("Could not draw line for satellite {}: {:?}", id, e)
                            })?
                            .label(format!("{:06}", id));

                        let first_time = (time[first_visible] as f32).max(x.start);
                        let first_freq = freq[first_visible] as f32 - spectrogram.freq;
                        chart
                            .draw_series(vec![Text::new(
                                format!("{:06}", id),
                                (first_time, first_freq),
                                ("sans-serif", 12).into_font().color(&color),
                            )])
                            .map_err(|e| {
                                format!("Could not draw label for satellite {}: {:?}", id, e)
                            })?;
```

- [ ] **Step 10: Update the one call site in `mod.rs`**

In the `impl Chart<super::Message> for PlotChart<'_>` block (from Task 1), change:

```rust
        match self.rfplot.overlay.build_chart(chart, &self.rfplot.shared) {
```

to:

```rust
        match self
            .rfplot
            .overlay
            .build_chart(chart, &self.rfplot.shared, self.app)
        {
```

- [ ] **Step 11: Build and test**

Run: `cargo build --release && cargo test && cargo clippy`
Expected: builds clean, all tests (including the two new ones) pass.

- [ ] **Step 12: Hand off to the user: ask them to visually confirm the coloring**

The unit tests only prove `prediction_color`'s mapping is right in isolation, not that both draw calls (the `LineSeries` and the label `Text`) actually use the freshly computed `color` instead of one of them silently staying on the old `&GREEN` — that mistake would still compile, so this is the one thing in Task 3 that needs eyes on the actual render.

Real satellite catalogs are almost always `Unclassified`, so this won't show up by just loading a normal catalog. Ask the user to, once, on a catalog file they already have loaded (or any TLE file used by the catalog/frequency-file path):

1. Pick one active satellite's TLE and change the classification character at column 8 of its line 1 (the character right after the norad id, e.g. `1 25544U ...` → `1 25544C ...` for Classified, or `...S ...` for Secret).
2. Reload the catalog, confirm predictions are enabled and that satellite has a visible pass in the loaded spectrogram's time/frequency window.
3. Confirm that satellite's prediction line + label render orange (`Classified`) or red (`Secret`), while every other satellite's predictions stay green.
4. Revert the TLE edit afterwards (this is a test-only, throwaway change to a local file, not something to commit).

Wait for their confirmation before committing.

- [ ] **Step 13: Commit**

```bash
git add src/bin/rstrf/app.rs src/bin/rstrf/windows/rfplot/overlay.rs src/bin/rstrf/windows/rfplot/mod.rs
git commit -m "feat: color predictions by satellite classification"
```

---

### Task 4: Extract `Display` (the show/hide toggles) out of `Overlay` and onto `SharedState`

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/mod.rs`
- Modify: `src/bin/rstrf/windows/rfplot/overlay.rs`

**Interfaces:**
- Produces: `Display { show_predictions: bool, show_grid: bool, show_crosshair: bool, absolute_axes: bool }`, a new field `pub display: Display` on `SharedState`.

This is the first of three field-relocation tasks (`Display`, then `Marks`, then `Interaction`) that together empty out `Overlay` so it can be deleted in Task 7. Each is a pure move: no new behavior. The manual checklist only runs once, after Task 7, when the whole relocation is done — not after each of these three individually — since a mistake in any of them would surface again in the very next task that touches the same code.

**Important non-obvious detail:** `SharedState` currently derives `PartialEq` automatically (comparing `controls`, `spectrogram_files`, `spectrogram`, `plot_area_margin`). `Overlay`, in contrast, has a *manual* `PartialEq` impl that only compares `track_points`, `signals`, `crosshair`, and `absolute_axes` — it deliberately excludes `show_predictions`, `show_grid`, `show_crosshair`, `mouse_state`, and `modifiers` from equality. `RFPlot`'s own manual `PartialEq` combines both (`self.shared == other.shared && self.overlay == other.overlay && self.id == other.id`), and this combined equality is what `plotters_iced2::Chart`/`iced::widget::shader::Program` use to decide whether to redraw — so it is load-bearing, not incidental. This task starts preserving that exact combined semantics: `#[derive(PartialEq)]` must come off `SharedState` in this task, replaced with a manual impl, because `Display` will carry three fields (`show_predictions`, `show_grid`, `show_crosshair`) that must NOT count towards equality, alongside one (`absolute_axes`) that must.

- [ ] **Step 1: Add `Display` to `mod.rs`**

Add near the top of `mod.rs`, after the `MouseState` enum:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Display {
    show_predictions: bool,
    show_grid: bool,
    show_crosshair: bool,
    absolute_axes: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            show_predictions: true,
            show_grid: false,
            show_crosshair: false,
            absolute_axes: true,
        }
    }
}
```

- [ ] **Step 2: Add the field to `SharedState` and switch to a manual `PartialEq`**

Change:

```rust
#[derive(Serialize, Deserialize, PartialEq, Default, Clone)]
pub(crate) struct SharedState {
    pub controls: Controls,
    pub spectrogram_files: Vec<PathBuf>,
    #[serde(skip)]
    pub spectrogram: Option<Spectrogram>,
    /// The margin on the left/bottom of the plot area (for axes/labels)
    pub plot_area_margin: f32,
}
```

to:

```rust
#[derive(Serialize, Deserialize, Default, Clone)]
pub(crate) struct SharedState {
    pub controls: Controls,
    pub spectrogram_files: Vec<PathBuf>,
    #[serde(skip)]
    pub spectrogram: Option<Spectrogram>,
    /// The margin on the left/bottom of the plot area (for axes/labels)
    pub plot_area_margin: f32,
    pub display: Display,
}

impl PartialEq for SharedState {
    fn eq(&self, other: &Self) -> bool {
        self.controls == other.controls
            && self.spectrogram_files == other.spectrogram_files
            && self.spectrogram == other.spectrogram
            && self.plot_area_margin == other.plot_area_margin
            && self.display.absolute_axes == other.display.absolute_axes
    }
}
```

- [ ] **Step 3: Remove the fields from `Overlay` and read/write them via `shared.display` instead**

In `overlay.rs`:

- Remove `show_predictions: bool`, `show_grid: bool`, `show_crosshair: bool`, `absolute_axes: bool` from the `Overlay` struct definition, and their initializers from `impl Default for Overlay`.
- Remove them from `impl PartialEq for Overlay` (only `track_points`, `signals`, `crosshair` remain there — `absolute_axes` moved to `SharedState`'s new manual impl in Step 2).
- Every method on `Overlay` that read `self.show_predictions`/`self.show_grid`/`self.show_crosshair`/`self.absolute_axes` already takes `shared: &SharedState` as a parameter (this is true of `build_chart`, `handle_mouse`, `handle_keyboard`, `status`) — change those reads to `shared.display.show_predictions` etc.
- The `TogglePredictions`/`ToggleGrid`/`ToggleCrosshair`/`ToggleAbsoluteAxes` arms inside `Overlay::update` currently do `self.show_predictions = !self.show_predictions;` (and similarly for the other three). `Overlay::update`'s signature is `pub fn update(&mut self, message: Message, shared: &SharedState, app: &AppShared) -> Task<Message>` — `shared` is borrowed immutably, so it can't be mutated in place. Change the signature to take `shared: &mut SharedState`, and change these four arms to mutate `shared.display.*` instead of `self.*`.

To find every remaining reference, run:

```bash
grep -n 'self\.show_predictions\|self\.show_grid\|self\.show_crosshair\|self\.absolute_axes' src/bin/rstrf/windows/rfplot/overlay.rs
```

and fix each until the grep is empty.

- [ ] **Step 4: Update the two callers of `Overlay::update`**

In `mod.rs`, `Overlay::update` is called from `RFPlot::app_event` and from `RFPlot::update`'s `Message::Overlay` and `Message::SpectrogramLoaded` arms. Change each `&self.shared` argument to `&mut self.shared` at those three call sites.

- [ ] **Step 5: Build and test**

Run: `cargo build --release && cargo test && cargo clippy`
Expected: builds clean (the compiler will point at every remaining `self.show_*`/`self.absolute_axes` reference in `overlay.rs` as an error — fix each), all tests pass. No manual checkpoint here — see the note above.

- [ ] **Step 6: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/mod.rs src/bin/rstrf/windows/rfplot/overlay.rs
git commit -m "refactor: move Overlay's display toggles onto SharedState"
```

---

### Task 5: Extract `Marks` (track points + signals) out of `Overlay` and onto `SharedState`

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/mod.rs`
- Modify: `src/bin/rstrf/windows/rfplot/overlay.rs`

**Interfaces:**
- Produces: `Marks { track_points: Vec<data_absolute::Point>, signals: Vec<data_absolute::Point> }`, a new field `pub marks: Marks` on `SharedState`. Unlike `Display`, both fields of `Marks` participate in equality — this matches `Overlay`'s current manual `PartialEq`, so `Marks` can safely `#[derive(PartialEq)]` in full.

- [ ] **Step 1: Add `Marks` to `mod.rs`**

Add next to `Display`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
struct Marks {
    track_points: Vec<data_absolute::Point>,
    signals: Vec<data_absolute::Point>,
}
```

This needs `data_absolute` in scope — `mod.rs` does not currently import it; add `data_absolute` to the existing `use rstrf::coord::{data_normalized, plot_area};` import, making it `use rstrf::coord::{data_absolute, data_normalized, plot_area};`.

- [ ] **Step 2: Add the field to `SharedState` and extend the manual `PartialEq`**

Add `pub marks: Marks,` to the `SharedState` struct (from Task 4), and extend the `impl PartialEq for SharedState` block with `&& self.marks == other.marks`.

- [ ] **Step 3: Remove the fields from `Overlay` and read/write them via `shared.marks` instead**

In `overlay.rs`:

- Remove `track_points: Vec<data_absolute::Point>` and `signals: Vec<data_absolute::Point>` from the `Overlay` struct, and their initializers from `impl Default for Overlay`.
- Remove `track_points`/`signals` from `impl PartialEq for Overlay` (only `crosshair` remains there after this task).
- `Overlay::update`'s `AddTrackPoint`, `AddSignal`, `DeleteMark`, `ClearAll`, `FindSignals`, `FoundSignals`, `DeleteInRect`, `MarkCentroid`, `SaveSignals` arms all read or mutate `self.track_points`/`self.signals` — after Task 4, `Overlay::update` already takes `shared: &mut SharedState`, so change these to `shared.marks.track_points`/`shared.marks.signals`.
- `Overlay::build_chart`, `handle_mouse` read `self.track_points`/`self.signals` (e.g. the `closest_mark(pos, ..., &self.track_points, &self.signals)` call, and the two `draw_series` calls over `self.track_points.iter()`/`self.signals.iter()`) — both already take `shared: &SharedState`, so change these to `shared.marks.track_points`/`shared.marks.signals`.

To find every remaining reference, run:

```bash
grep -n 'self\.track_points\|self\.signals' src/bin/rstrf/windows/rfplot/overlay.rs
```

and fix each until the grep is empty.

- [ ] **Step 4: Build and test**

Run: `cargo build --release && cargo test && cargo clippy`
Expected: builds clean, all tests pass.

No manual checkpoint here — see the note at the top of Task 4.

- [ ] **Step 5: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/mod.rs src/bin/rstrf/windows/rfplot/overlay.rs
git commit -m "refactor: move Overlay's track points/signals onto SharedState"
```

---

### Task 6: Extract `Interaction` (the `Cell`-based mouse/keyboard state) out of `Overlay` and onto `SharedState`

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/mod.rs`
- Modify: `src/bin/rstrf/windows/rfplot/overlay.rs`

**Interfaces:**
- Produces: `Interaction { crosshair: Cell<Option<data_absolute::Point>>, mouse_state: Cell<MouseState>, modifiers: Cell<keyboard::Modifiers> }`, a new field `pub interaction: Interaction` on `SharedState`.

**Important non-obvious detail:** only `crosshair` participates in `Overlay`'s current equality check — `mouse_state` and `modifiers` are deliberately excluded (they're pure transient interaction bookkeeping, not state that should trigger a redraw comparison). Do **not** derive `PartialEq` on `Interaction` — deriving it would silently start comparing `mouse_state`/`modifiers` too, changing behavior. Keep the hand-written comparison at the `SharedState` level, touching only `.crosshair`.

- [ ] **Step 1: Add `Interaction` to `mod.rs`**

`keyboard::Modifiers` needs `iced::keyboard` in scope in `mod.rs` — add `keyboard` to the existing `iced::{...}` import list.

```rust
#[derive(Debug, Default)]
struct Interaction {
    crosshair: Cell<Option<data_absolute::Point>>,
    mouse_state: Cell<MouseState>,
    modifiers: Cell<keyboard::Modifiers>,
}
```

(`MouseState: Default` and `keyboard::Modifiers: Default` both already hold — `Overlay`'s current `Default` impl relies on exactly this via `Cell::new(MouseState::Idle)`/`Cell::new(keyboard::Modifiers::default())`, so `#[derive(Default)]` on `Interaction` reproduces the same initial values.)

`Cell` needs `std::cell::Cell` in scope in `mod.rs` — add `use std::cell::Cell;` (or fold it into an existing `std::` import if one exists).

- [ ] **Step 2: Add the field to `SharedState`, skip it in serde, and extend the manual `PartialEq`**

Add `#[serde(skip)] pub interaction: Interaction,` to the `SharedState` struct, and extend `impl PartialEq for SharedState` with `&& self.interaction.crosshair == other.interaction.crosshair` (not `mouse_state`, not `modifiers`).

- [ ] **Step 3: Remove the fields from `Overlay` and read/write them via `shared.interaction` instead**

In `overlay.rs`:

- Remove `crosshair`, `mouse_state`, `modifiers` (all three `Cell<_>` fields, all `#[serde(skip)]`) from the `Overlay` struct, and their initializers from `impl Default for Overlay`.
- Remove `crosshair` from `impl PartialEq for Overlay` — after this task, `Overlay`'s manual `PartialEq` impl has nothing left to compare (`track_points`/`signals` moved in Task 5, `absolute_axes` in Task 4, `crosshair` now); leave the impl in place with an empty-bodied `true` for now — Task 7 deletes the whole `Overlay` struct anyway.
- `handle_mouse` and `handle_keyboard` both already take `shared: &SharedState` — change every `self.crosshair`/`self.mouse_state`/`self.modifiers` in their bodies to `shared.interaction.crosshair`/`shared.interaction.mouse_state`/`shared.interaction.modifiers`.
- The `impl Chart<super::Message> for PlotChart<'_>` block in `mod.rs` (from Task 1) reads `self.rfplot.overlay.modifiers.set(*modifiers)` (in `update`) and `self.rfplot.overlay.mouse_state.get()` (in `mouse_interaction`) — change both to `self.rfplot.shared.interaction.modifiers`/`self.rfplot.shared.interaction.mouse_state`.

To find every remaining reference, run:

```bash
grep -n 'self\.crosshair\|self\.mouse_state\|self\.modifiers' src/bin/rstrf/windows/rfplot/overlay.rs src/bin/rstrf/windows/rfplot/mod.rs
```

and fix each until the grep is empty.

- [ ] **Step 4: Build and test**

Run: `cargo build --release && cargo test && cargo clippy`
Expected: builds clean, all tests pass.

No manual checkpoint here — see the note at the top of Task 4. (This task is the highest-risk of the three relocations, since a mistake here wouldn't be a compile error — it'd show up as broken crosshair/panning/rect-drawing. Task 7's checklist run is where that gets caught.)

- [ ] **Step 5: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/mod.rs src/bin/rstrf/windows/rfplot/overlay.rs
git commit -m "refactor: move Overlay's Cell-based interaction state onto SharedState"
```

---

### Task 7: Delete `Overlay`; fold `prediction_cache` and its methods onto `SharedState`

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/mod.rs`
- Modify: `src/bin/rstrf/windows/rfplot/overlay.rs`

**Interfaces:**
- Consumes: `SharedState` now carrying `display`, `marks`, `interaction` (Tasks 4–6).
- Produces: `SharedState` also carrying `#[serde(skip)] pub prediction_cache: AsyncCache<PredictionKey, orbit::Predictions>`. The `Overlay` type, its `Default` impl, and its `PartialEq` impl are deleted entirely. Its methods (`build_chart`, `handle_mouse`, `handle_keyboard`, `status`, `check_cache`, `update`) become inherent methods on `SharedState` (`impl SharedState { ... }` in `overlay.rs`), each dropping the now-redundant `shared: &SharedState`/`&mut SharedState` parameter (`self` already is what `shared` used to be).

At this point `Overlay` is an empty shell — every field it had has moved to `SharedState`. This task removes the shell and its methods' redundant second `self`-like parameter.

- [ ] **Step 1: Move `prediction_cache` to `SharedState`**

Add `#[serde(skip)] pub prediction_cache: AsyncCache<PredictionKey, orbit::Predictions>,` to the `SharedState` struct in `mod.rs`. This needs `rstrf::async_cache::AsyncCache` and the `PredictionKey`/`orbit` types in scope in `mod.rs` — add:

```rust
use rstrf::async_cache::AsyncCache;
```

and use `rfplot::overlay::PredictionKey` (already `pub(crate)` in `overlay.rs`) and `rstrf::orbit` (already imported in `mod.rs` — check the existing `use rstrf::{..., orbit, ...}` or add it) for the type.

Delete the `#[serde(skip)] prediction_cache: AsyncCache<PredictionKey, orbit::Predictions>,` field from `Overlay` and its `AsyncCache::default()` initializer from `impl Default for Overlay`.

- [ ] **Step 2: Delete `Overlay`'s struct, `Default`, and `PartialEq` definitions**

Delete the `struct Overlay { ... }` definition (now empty), its `impl Default for Overlay { ... }`, and its `impl PartialEq for Overlay { ... }` (now comparing nothing, per Task 6).

- [ ] **Step 3: Turn `impl Overlay { ... }` into `impl SharedState { ... }`**

Change `impl Overlay {` (the block containing `build_chart`, `handle_mouse`, `handle_keyboard`, `status`, `check_cache`, `update`) to `impl SharedState {`, and drop the now-redundant second parameter from each method, since `self` already provides what that parameter used to:

- `fn build_chart<DB: DrawingBackend>(&self, mut chart: ChartBuilder<DB>, shared: &SharedState, app: &AppShared)` → `fn build_chart<DB: DrawingBackend>(&self, mut chart: ChartBuilder<DB>, app: &AppShared)`. Every `shared.` inside the body becomes `self.`.
- `fn handle_mouse(&self, event: &mouse::Event, bounds: Rectangle, cursor: mouse::Cursor, shared: &SharedState)` → drop `shared`, `shared.` → `self.` inside.
- `fn handle_keyboard(&self, event: &keyboard::Event, bounds: Rectangle, cursor: mouse::Cursor, shared: &SharedState)` → same.
- `fn status(&self, app: &AppShared) -> Option<&str>` — unchanged signature (it never took a separate `shared` parameter).
- `fn check_cache(&mut self, shared: &SharedState, app: &AppShared) -> Task<Message>` → `fn check_cache(&mut self, app: &AppShared) -> Task<Message>`. Its one caller, `prediction_key(shared, app)`, becomes `prediction_key(self, app)` — but `prediction_key`'s signature is `fn prediction_key(shared: &SharedState, app: &AppShared) -> Option<PredictionKey>`, so no change is needed there beyond the call-site argument.
- `pub fn update(&mut self, message: Message, shared: &mut SharedState, app: &AppShared) -> Task<Message>` → `pub fn update(&mut self, message: Message, app: &AppShared) -> Task<Message>`. Every `shared.` inside the body becomes `self.`; every call to `self.check_cache(shared, app)` becomes `self.check_cache(app)`.

Rename the module-level `Message` type's doc references from "Overlay" to "the plot's marks/display/prediction state" only if convenient — not required for this task (Task 9 handles the message enum naming).

- [ ] **Step 4: Update the three callers**

In `mod.rs`:

- `RFPlot::app_event`: `self.overlay.update(overlay::Message::RefreshCache, &self.shared, app)` → `self.shared.update(overlay::Message::RefreshCache, app)`.
- `RFPlot::update`'s `Message::Overlay` arm: `self.overlay.update(message, &self.shared, app)` → `self.shared.update(message, app)`.
- `RFPlot::update`'s `Message::SpectrogramLoaded` arm: `self.overlay.update(overlay::Message::SpectrogramUpdated, &self.shared, app)` → `self.shared.update(overlay::Message::SpectrogramUpdated, app)`.
- `RFPlot::view`: `let status = self.overlay.status(app);` → `let status = self.shared.status(app);`.
- Remove the `overlay: overlay::Overlay` field from the `RFPlot` struct (and its `overlay::Overlay::default()` initializer in `RFPlot::new()`, and `self.overlay == other.overlay` from `RFPlot`'s manual `PartialEq`).
- The `impl Chart<super::Message> for PlotChart<'_>` block (Task 1) reads `self.rfplot.overlay.build_chart(...)`, `self.rfplot.overlay.handle_mouse(...)`, `self.rfplot.overlay.handle_keyboard(...)` — change each to `self.rfplot.shared.build_chart(...)`, `self.rfplot.shared.handle_mouse(...)`, `self.rfplot.shared.handle_keyboard(...)` (and drop the now-redundant `&self.rfplot.shared` argument each was passing).

To find every remaining reference to the deleted `overlay` field or type, run:

```bash
grep -rn '\.overlay\b\|overlay::Overlay\b' src/bin/rstrf/windows/rfplot/
```

and fix each until the grep only shows the (still-valid) `mod overlay;`/`overlay::Message`/`overlay::PredictionKey` references.

- [ ] **Step 5: Build and test**

Run: `cargo build --release && cargo test && cargo clippy`
Expected: builds clean, all tests pass.

- [ ] **Step 6: Hand off to the user: ask them to run the manual verification checklist**

This is the first real checkpoint since Task 1 — it covers everything moved in Tasks 4, 5, 6, and this task's deletion of `Overlay` in one pass. Pay particular attention to the crosshair readout and mouse-drag interactions (panning, rect-drawing): that's the `Interaction` relocation from Task 6, the one place in this whole sequence where a mistake wouldn't have been a compile error.

- [ ] **Step 7: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/mod.rs src/bin/rstrf/windows/rfplot/overlay.rs
git commit -m "refactor: delete Overlay, fold its methods onto SharedState"
```

---

### Task 8: Rename `SharedState` to `State`; make `ResetView` clear marks directly

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/mod.rs`
- Modify: `src/bin/rstrf/windows/rfplot/overlay.rs`
- Modify: `src/bin/rstrf/windows/rfplot/control.rs`

**Interfaces:**
- Produces: `State` (renamed from `SharedState`) — by this point it already holds `controls`, `spectrogram_files`, `spectrogram`, `plot_area_margin`, `display`, `marks`, `interaction`, `prediction_cache`, i.e. everything `Controls` and the former `Overlay` used to hold separately. No new fields move in this task — `Controls` already lived on `SharedState` from the start, so "folding Controls in" is already done; this task is the rename plus one behavior simplification.

- [ ] **Step 1: Rename `SharedState` to `State` everywhere**

Run:

```bash
grep -rl 'SharedState' src/bin/rstrf/windows/rfplot/ | xargs sed -i 's/SharedState/State/g'
```

Rename the `shared: State` field on `RFPlot` to `state: State` as well (for clarity, since it's no longer "shared with a sibling component" — nothing else needs it now that `Overlay` is gone):

```bash
grep -rl '\.shared\b\|shared:' src/bin/rstrf/windows/rfplot/ | xargs sed -i 's/self\.shared/self.state/g; s/&self\.shared/\&self.state/g'
```

Do this rename by hand rather than blindly trusting a second global `shared` → `state` substitution — `shared` also appears as a local parameter name and in the `prediction_key(shared: &State, ...)` free function signature, which should also become `state` for consistency, but do it deliberately field-by-field rather than a blanket sed, since `shared` is a common enough word to appear in unrelated contexts (double check with `grep -n '\bshared\b' src/bin/rstrf/windows/rfplot/*.rs` afterwards and confirm every remaining hit is either intentional or renamed).

- [ ] **Step 2: Simplify `ResetView` to clear marks directly**

In `control.rs`, `Controls::update`'s `ResetView` arm currently does:

```rust
            Message::ResetView => {
                self.log_scale = Vec2::new(ZOOM_MIN, ZOOM_MIN);
                self.center = data_normalized::Point::new(0.5, 0.5);
                return Task::done(rfplot::overlay::Message::ClearAll.into());
            }
```

`Controls::update` only ever sees `&mut self` (just the `Controls` struct), so it has no path to `state.marks` directly — that's exactly the round-trip-through-the-runtime this task removes. Leave `Controls::update`'s `ResetView` arm resetting only `log_scale`/`center` (remove the `Task::done(...)` line, return `Task::none()` implicitly via falling through to `self.snap_to_bounds(); Task::none()` like every other arm). Then, in `mod.rs`'s `RFPlot::update`, intercept `Message::Control(control::Message::ResetView)` in the early match block (the one that already special-cases `GpuUploadDone`/`SaveScreenshot`/`LoadSpectrogram` "before" the generic dispatch), clearing the marks directly:

```rust
            Message::Control(control::Message::ResetView) => {
                self.state.marks = Default::default();
            }
```

Let this fall through to the generic dispatch afterwards (don't `return` here) so `Message::Control(control::Message::ResetView)` still reaches `self.state.controls.update(message)` and resets the zoom/center as before — i.e. add this as a non-returning side effect at the top of `update()`, then let the existing `let result = match message { Message::Control(message) => self.state.controls.update(message), ... }` still run afterwards for the same message. (This mirrors the existing pattern where `GpuUploadDone` etc. are matched once for their `WindowEffect`, before the generic dispatch match runs.)

- [ ] **Step 3: Build and test**

Run: `cargo build --release && cargo test && cargo clippy`
Expected: builds clean, all tests pass.

- [ ] **Step 4: Hand off to the user: ask them to run the manual verification checklist**

Specifically re-check checklist item 5 (reset view clears marks) since its implementation changed in this task.

- [ ] **Step 5: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/mod.rs src/bin/rstrf/windows/rfplot/overlay.rs src/bin/rstrf/windows/rfplot/control.rs
git commit -m "refactor: rename SharedState to State; clear marks directly on ResetView"
```

---

### Task 9: Regroup `rfplot::Message` by topic (`View`/`Marks`) instead of by former owner

**Files:**
- Modify: `src/bin/rstrf/windows/rfplot/mod.rs`

**Interfaces:**
- Produces: `rfplot::Message::View(control::Message)` (renamed from `Message::Control`), `rfplot::Message::Marks(overlay::Message)` (renamed from `Message::Overlay`). Purely a rename — no field or logic changes.

This is the most mechanical task in the plan: `Message::Control`/`Message::Overlay` are compiler-checked names, so every call site that needs updating will be a compile error if missed.

- [ ] **Step 1: Rename the enum variants**

In `mod.rs`, change:

```rust
pub enum Message {
    Control(control::Message),
    Overlay(overlay::Message),
    ...
}

impl From<control::Message> for Message {
    fn from(message: control::Message) -> Self {
        Message::Control(message)
    }
}

impl From<overlay::Message> for Message {
    fn from(message: overlay::Message) -> Self {
        Message::Overlay(message)
    }
}
```

to:

```rust
pub enum Message {
    View(control::Message),
    Marks(overlay::Message),
    ...
}

impl From<control::Message> for Message {
    fn from(message: control::Message) -> Self {
        Message::View(message)
    }
}

impl From<overlay::Message> for Message {
    fn from(message: overlay::Message) -> Self {
        Message::Marks(message)
    }
}
```

- [ ] **Step 2: Fix every remaining reference**

Run:

```bash
cargo build --release 2>&1 | grep -E 'Message::(Control|Overlay)'
```

and rename each match arm/constructor site (`Message::Control(message) => ...` → `Message::View(message) => ...`, `Message::Overlay(message) => ...` → `Message::Marks(message) => ...`, and the exhaustiveness-check line at the bottom of `RFPlot::update`'s match: `Message::GpuUploadDone | Message::SaveScreenshot(_, _) | Message::LoadSpectrogram(_) => unreachable!()` is unaffected, but double check the compiler doesn't flag it) until `cargo build --release` succeeds.

- [ ] **Step 3: Build and test**

Run: `cargo build --release && cargo test && cargo clippy`
Expected: builds clean, all tests pass.

No manual checkpoint here — `Message::Control`/`Message::Overlay` are compiler-checked names (per this task's opening note), so a missed rename is a compile error, not a runtime regression.

- [ ] **Step 4: Commit**

```bash
git add src/bin/rstrf/windows/rfplot/mod.rs
git commit -m "refactor: rename rfplot::Message variants to View/Marks"
```

---

### Task 10: Disable the "Save Signals" toolbar button when there are no signals

**Files:**
- Modify: `src/bin/rstrf/widgets/mod.rs`
- Modify: `src/bin/rstrf/windows/rfplot/control.rs`
- Modify: `src/bin/rstrf/windows/sat_manager.rs`

**Interfaces:**
- Produces: `ToolbarButton::Icon { icon, tooltip, msg, enabled: bool, style }` and `ToolbarButton::LabeledIcon { icon, label, tooltip, msg, enabled: bool, style }` (new `enabled` field on both variants); `icon_button`/`labeled_icon_button`/`tooltip_button` gain a `enabled: bool` parameter, calling `.on_press_maybe(enabled.then_some(msg))` instead of `.on_press(msg)`.

This is the payoff of the merge: `Controls::view(&self, shared: &State)` already receives the whole window's state, so once `State` carries `marks` (Task 5), this is a one-line read plus the toolbar-button plumbing to actually disable a button.

- [ ] **Step 1: Add `enabled: bool` to `tooltip_button`, `icon_button`, `labeled_icon_button`**

In `src/bin/rstrf/widgets/mod.rs`, change:

```rust
pub fn tooltip_button<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    tooltip_label: &'a str,
    msg: Message,
    style: impl Fn(&Theme, button::Status) -> button::Style + Clone + 'a,
    width: impl Into<Length>,
) -> Element<'a, Message> {
    tooltip(
        button(content)
            .width(width)
            .height(26)
            .padding(4)
            .style(style)
            .on_press(msg),
        container(text(tooltip_label))
            .padding(5)
            .style(container::dark),
        tooltip::Position::Bottom,
    )
    .delay(Duration::from_millis(500))
    .into()
}
```

to:

```rust
pub fn tooltip_button<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    tooltip_label: &'a str,
    msg: Message,
    enabled: bool,
    style: impl Fn(&Theme, button::Status) -> button::Style + Clone + 'a,
    width: impl Into<Length>,
) -> Element<'a, Message> {
    tooltip(
        button(content)
            .width(width)
            .height(26)
            .padding(4)
            .style(style)
            .on_press_maybe(enabled.then_some(msg)),
        container(text(tooltip_label))
            .padding(5)
            .style(container::dark),
        tooltip::Position::Bottom,
    )
    .delay(Duration::from_millis(500))
    .into()
}
```

Change `icon_button` and `labeled_icon_button` to take and forward the same `enabled: bool` parameter (inserted right after `msg: Message` in each signature, forwarded as the corresponding new argument to their `tooltip_button(...)` call).

- [ ] **Step 2: Add `enabled: bool` to `ToolbarButton::Icon`/`LabeledIcon` and its `view()`**

Change:

```rust
pub enum ToolbarButton<Message: Clone> {
    Icon {
        icon: Icon,
        tooltip: &'static str,
        msg: Message,
        style: fn(&Theme, button::Status) -> button::Style,
    },
    LabeledIcon {
        icon: Icon,
        label: &'static str,
        tooltip: &'static str,
        msg: Message,
        style: fn(&Theme, button::Status) -> button::Style,
    },
    Submenu {
        toplevel: Box<ToolbarButton<Message>>,
        submenu: Vec<ToolbarButton<Message>>,
    },
}
```

to add `enabled: bool` to both `Icon` and `LabeledIcon` (not `Submenu`), and update `ToolbarButton::view()`'s two matching arms to forward `*enabled`:

```rust
            ToolbarButton::Icon {
                icon,
                tooltip,
                msg,
                enabled,
                style,
            } => icon_button(*icon, tooltip, msg.clone(), *enabled, *style),
            ToolbarButton::LabeledIcon {
                icon,
                label,
                tooltip,
                msg,
                enabled,
                style,
            } => labeled_icon_button(*icon, label, tooltip, msg.clone(), *enabled, *style),
```

- [ ] **Step 3: Fix every existing `ToolbarButton::Icon`/`LabeledIcon` literal**

Run:

```bash
cargo build --release 2>&1 | grep -B2 'missing field `enabled`'
```

and add `enabled: true,` to each of the 15 existing struct literals this reports (12 in `control.rs`, 3 in `sat_manager.rs`) until the build succeeds. This includes the `Submenu`'s `toplevel: Box::new(ToolbarButton::Icon { ... })` in `control.rs` and the colormap-loop `LabeledIcon` in `control.rs`.

- [ ] **Step 4: Disable the Save Signals button when there are no signals**

In `control.rs`, `Controls::view(&self, shared: &State)` builds the toolbar. Change the Save Signals button from:

```rust
            ToolbarButton::Icon {
                icon: Icon::Save,
                tooltip: "Save signals to out.dat",
                msg: rfplot::overlay::Message::SaveSignals.into(),
                enabled: true,
                style: widget::button::primary,
            },
```

to:

```rust
            ToolbarButton::Icon {
                icon: Icon::Save,
                tooltip: "Save signals to out.dat",
                msg: rfplot::overlay::Message::SaveSignals.into(),
                enabled: !shared.marks.signals.is_empty(),
                style: widget::button::primary,
            },
```

This requires `marks`/`signals` on `State` to be visible from `control.rs` — confirm they're `pub` (Task 5 declared `pub marks: Marks` on `State`/`SharedState`, but `Marks`'s own `signals`/`track_points` fields were left private in Task 5's `struct Marks { track_points: ..., signals: ... }`). Make `signals` (and, for consistency, `track_points`) `pub` on `Marks` in `mod.rs`.

- [ ] **Step 5: Build and test**

Run: `cargo build --release && cargo test && cargo clippy`
Expected: builds clean, all tests pass.

- [ ] **Step 6: Hand off to the user to verify the new behavior**

Do not run `cargo run` yourself. Ask the user to run it with a spectrogram loaded and confirm:
1. The Save Signals button is greyed out / unclickable when no signals are marked.
2. Marking a signal (toolbar "Mark signals" + click on the plot) makes the button clickable.
3. Saving, then clearing all marks (toolbar "Clear signals & track points" or reset view), greys the button out again.
4. The rest of the manual verification checklist still passes — no other toolbar button should have changed (they're all still `enabled: true` unconditionally).

Wait for their confirmation before committing.

- [ ] **Step 7: Commit**

```bash
git add src/bin/rstrf/widgets/mod.rs src/bin/rstrf/windows/rfplot/control.rs src/bin/rstrf/windows/sat_manager.rs
git commit -m "feat: disable Save Signals button when there are no signals"
```
