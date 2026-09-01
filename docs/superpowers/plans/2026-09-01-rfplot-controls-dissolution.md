# RFPlot: dissolving `Controls`

A plan for finishing the RFPlot window's state consolidation. The previous round merged
`Overlay` and `SharedState` into a single `State` on the `RFPlot` window; this round
removes the last sub-component, `Controls`.

## Why

`Controls` is a grab-bag. Its ten fields cover four unrelated concerns: viewport
(`zoom_max`, `log_scale`, `center`), power scaling (`power_bounds`, `power_range`),
signal detection (`signal_sigma`, `track_bw`), and presentation (`colormap`,
`average_plotting`, `show_controls`). Only the first two have invariants; the rest are
loose values that happen to share a struct.

More importantly, `Controls` still carries the full component triad — its own message
enum, an `update` that consumes it, and a `view` — even though nothing dispatches it
independently. iced's `Daemon` dispatches per `window::Id`, so the *window* is the only
real component here; anything below it should be data plus functions over that data.
Keeping the triad has three visible costs:

- `Controls::view(&self, state: &State)` is a method on a struct that is itself reachable
  through its own parameter, and reads at least one of its own fields (`colormap`) via
  that parameter rather than via `self`. Nothing enforces that the two agree.
- `Controls::update(&mut self, ..)` cannot reach sibling state, so "reset the view and
  clear the marks" cannot be expressed in one place. `RFPlot::update` intercepts
  `ResetView` in a pre-pass, mutates `state.marks`, then falls through to the real
  dispatch — which is why that function matches its message twice and needs an
  `unreachable!()` arm to stay exhaustive.
- `Controls::update` returns a `Task` that is now unconditionally `Task::none()`.

Separately, `rfplot::Message`'s two wrapper variants are named for topics (`View`,
`Marks`) while their payloads are still the old owner-named enums. The labels are
inaccurate: `Marks(overlay::Message)` carries the four display toggles and the three
prediction-cache variants, and `View(control::Message)` carries the two signal-detection
parameters.

## Target shape

`Controls` disappears rather than being renamed. Its fields redistribute:

| fields | destination |
|---|---|
| `zoom_max`, `log_scale`, `center` | `Viewport`, a new `viewport.rs` |
| `power_bounds`, `power_range` | `PowerRange`, in `mod.rs` beside `Display`/`Marks` |
| `signal_sigma`, `track_bw` | `Detection`, in `mod.rs` |
| `colormap`, `average_plotting`, `show_controls` | `Display` (grows) |

`Viewport` and `PowerRange` own their invariants through named mutators — no message
type, no `update`. The single remaining match on the viewport message enum moves up to
`State::update_view`, where each arm becomes a one-line delegation. Routing belongs to
the component; mutation belongs to the type being mutated.

Both should stay `Copy`, as `Controls` is today, so the shader's `Primitive` path stays
cheap.

## Module privacy

A child module can read private items of its ancestors. `control.rs` is
`rfplot::control`, so a view function there can already reach `state.display.colormap`,
`state.marks.signals`, and anything else declared in `mod.rs`, without accessors. Only
`Viewport` — a sibling module — needs them, and only two: `log_scale()` and `zoom_max()`,
for the zoom sliders' current values and ranges.

## Commits

Run `cargo test && cargo clippy && cargo +nightly fmt --all` after each. `cargo test`
only reaches the pure math, so `cargo run --release -- pass-png` is the cheapest
end-to-end check — worth running at least after F and after H.

### A — `test:` characterization tests for view-bounds snapping

`Controls::update` ends with an unconditional `snap_to_bounds()`, and four of its arms
rely on it entirely rather than snapping themselves: `PanningDelta`, `ZoomDelta`,
`ZoomDeltaX`, `ZoomDeltaY`. Panning is already covered by
`pan_large_delta_snaps_back_in_bounds`; add the three zoom-delta cases. Zoom out at an
off-centre point and assert the resulting bounds stay within `[0, 1]` on both axes.

These pass as written. They exist to catch a dropped snap in commit C, where the trailing
call gets folded into four separate mutators.

### B — `fix:` snap the view after `set_data_bounds`

`set_spectrogram` calls `set_data_bounds`, which recomputes `zoom_max` and re-clamps
`log_scale`, but is not routed through `update` and so never snaps. If a newly loaded
spectrogram has a smaller span than the previous one, `zoom_max` shrinks, the view grows,
and with `center` near an edge the bounds can end up outside `[0, 1]` until the next pan
nudges them back.

Write the failing test first: small view, panned to an edge, then `set_data_bounds` with a
smaller total span. Then add the snap. Kept separate from C because it is a behaviour
change, not a relocation.

### C — `refactor:` extract `Viewport`

New `viewport.rs` holding `zoom_max`, `log_scale`, `center`, with:

- mutators that each carry their own clamping *and* snapping: `set_zoom_x`, `set_zoom_y`,
  `pan_by`, `zoom_at`, `zoom_x_at`, `zoom_y_at`, `reset`, `set_view_from_rect_dn`,
  `set_view_from_rect_da`, `set_data_bounds`
- readers: `bounds`, `size`, `data_normalized`, `log_scale`, `zoom_max`

`Controls` keeps its `update` for now and gains a `viewport` field; each arm becomes a
delegation. Move the five viewport tests (`default_size_is_full_view`,
`default_bounds_covers_unit_square`, `update_zoom_x_changes_width`,
`reset_view_restores_full_view`, `pan_large_delta_snaps_back_in_bounds`) plus the new ones
from A and B.

Folding one trailing `snap_to_bounds()` into four separate mutators is the single place in
this plan where a silent regression can enter. `set_view_from_rect_dn` is the exception —
it already snaps itself and needs no change. That also means `ZoomToRect` currently snaps
twice, once inside `set_view_from_rect_dn` and once from the trailing call; dropping the
trailing call resolves that incidentally.

### D — `refactor:` extract `PowerRange`

Same shape, smaller: `power_bounds` and `power_range`, with `set_min`, `set_max`,
`set_bounds`, `set_range`, preserving the two existing clamps (range clamped into bounds;
min never above max). Declare it in `mod.rs`. Move the three power tests. Reasonable to
fold into C.

### E — `refactor:` move the controls panel to a free function

`Controls::view(&self, state: &State)` becomes `pub fn view(state: &State) -> Element<..>`,
staying in `control.rs`. The aliasing goes away with the receiver. `Controls` is now
fields plus `update`.

### F — `refactor:` dissolve `Controls`

Redistribute the remaining fields per the table above; `viewport` and `power` become
`State` fields; `Controls::update(msg)` becomes `State::update_view(&mut self, msg)` in
`mod.rs`, with arms delegating to the mutators or assigning directly. The viewport message
enum stays in `control.rs` until it is split in H, so that file is left holding the enum
and `view`.

Call sites to update:

- `shader.rs` — `Primitive` currently copies the whole of `Controls` but reads only
  `bounds()`, `power_range()`, `colormap()`, and `average_plotting()`. Give it exactly
  those four.
- `overlay.rs` — `bounds()` in the coordinate transforms, plus `track_bw()` and
  `signal_sigma()`.
- `mod.rs` — `set_spectrogram`, `apply_initial_view`, and the `Window::init` /
  `app_event` config-sync paths.

### G — `refactor:` make `RFPlot::update` pure routing

With `State` owning both the viewport and the marks, `ResetView` becomes
`self.viewport.reset(); self.marks.clear();` inside `State::update_view`. The pre-dispatch
match arm and the `unreachable!()` arm both go; the `WindowEffect`-producing arms stay,
since those are genuinely the component's. `reset_view_clears_marks_and_zoom` guards this.

### H — `refactor:` regroup the messages by topic

Mechanical once the fields are partitioned: each variant follows the field it mutates.
`control::Message` and `overlay::Message` dissolve into topical enums, leaving `control.rs`
holding only `view`. The one construction site outside the window is the
`SetControlsVisible` message built in `pass_png.rs`. Split into two commits if it grows
unwieldy — re-bucket the variants first, re-home the handlers second.

### I — `refactor:` give `Marks` its invariant

Track points are kept sorted by time, but the `binary_search` insert that maintains this
sits inline in the update handler. Move it to `Marks::insert_track_point`, and add `clear`
plus accessors. Independent of the rest; can land any time after F.

### J — `refactor:` split files, refresh the docs

- `overlay.rs` splits into `chart.rs` (`PlotChart`, the `Chart` impl, and `build_chart`)
  and a home for the mouse/keyboard handlers
- `PredictionKey` and the cache-refresh logic move to `predictions.rs`
- `control.rs` becomes `toolbar.rs`, now that only the view remains
- the `overlay.rs` module doc comment still describes "the plot overlay" and needs
  rewriting; so does the RFPlot section of `CLAUDE.md`

Last, so that every hunk here is a move rather than a rewrite.

## Notes

- The workspace serialization path is not compiled: neither `workspace.rs` is named by a
  `mod` declaration, and both import a `crate::panes` module that no longer exists. Field
  moves in this plan therefore need no `#[serde(alias)]` shims or on-disk migration.
  Confirm this is still true before relying on it.
- `Controls::update`'s return type is already vestigial: every arm falls through to
  `Task::none()`. Whatever replaces it need not return a `Task` at all.
