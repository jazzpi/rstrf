use std::{path::PathBuf, pin::Pin, sync::Arc};

use futures_util::{SinkExt, Stream};
use iced::{
    Element, Length, Padding, Subscription, Task,
    alignment::{Horizontal, Vertical},
    widget::{self, button, container},
    window,
};
use image::RgbaImage;
use plotters_iced2::ChartWidget;
use rfd::AsyncFileDialog;
use rstrf::{
    async_cache::AsyncCache,
    colormap::Colormap,
    coord::{data_absolute, data_normalized, plot_area},
    menu::MenuItem,
    orbit, signal,
    spectrogram::Spectrogram,
    util::DebugRgbaImage,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app::{AppEvent, AppShared},
    io_service,
    windows::{Window, WindowEffect, WindowOut},
};

mod chart;
mod interaction;
mod marks;
mod predictions;
mod shader;
mod toolbar;
mod viewport;

use chart::PlotChart;
use interaction::{Interaction, MouseState, RectAction};
use marks::{MarkAction, Marks, signals_filename};
use viewport::Viewport;

#[derive(Debug, Clone)]
pub enum Message {
    View(ViewMsg),
    Display(DisplayMsg),
    Marks(MarksMsg),
    Predictions(PredictionsMsg),
    /// No-op that forces a redraw.
    ///
    /// For simplicity, we handle keyboard/mouse interaction in `Chart::update()` through `Cell`s.
    /// This allows us to emit a message anyways (which is what triggers a redraw).
    Refresh,
    PickSpectrogram,
    LoadSpectrogram(Vec<PathBuf>),
    SpectrogramLoaded(Result<(Vec<PathBuf>, Spectrogram), String>),
    LoadProgress {
        loaded: usize,
        total: usize,
    },
    GpuUploadDone,
    SetView(data_normalized::Rectangle),
    CaptureScreenshot(Option<PathBuf>),
    CapturedScreenshot(Result<(DebugRgbaImage, Option<PathBuf>), String>),
    SaveScreenshot(DebugRgbaImage, PathBuf),
    Nop,
}

/// Which part of the data space is on screen: the x/y viewport and the power (colour) range.
#[derive(Debug, Clone)]
pub enum ViewMsg {
    UpdateZoomX(f32),
    UpdateZoomY(f32),
    PanningDelta(plot_area::Vector),
    ZoomDelta(plot_area::Point, f32),
    ZoomDeltaX(plot_area::Point, f32),
    ZoomDeltaY(plot_area::Point, f32),
    ResetView,
    ZoomToRect(data_normalized::Rectangle),
    UpdateMinPower(f32),
    UpdateMaxPower(f32),
}

/// How the plot is presented: which layers are drawn, and in what style.
#[derive(Debug, Clone)]
pub enum DisplayMsg {
    TogglePredictions,
    ToggleGrid,
    ToggleCrosshair,
    ToggleAbsoluteAxes,
    SetControlsVisible(bool),
    UpdateColormap(Colormap),
    UpdateAveragePlotting(bool),
}

/// Track points and signals, plus the detection parameters that produce them.
#[derive(Debug, Clone)]
pub enum MarksMsg {
    MarkTrackpoints,
    MarkSignals,
    AddTrackPoint(data_absolute::Point),
    AddSignal(data_absolute::Point),
    DeleteMark(MarkAction, data_absolute::Point),
    DeleteInRect(data_absolute::Rectangle),
    MarkCentroid(data_absolute::Rectangle),
    ClearAll,
    FindSignals,
    FoundSignals(Vec<data_absolute::Point>),
    SaveSignals,
    WriteSignals(String, Option<PathBuf>),
    SpectrogramUpdated,
    UpdateSignalSigma(f32),
    UpdateTrackBW(f32),
}

/// The satellite pass prediction cache.
#[derive(Debug, Clone)]
pub enum PredictionsMsg {
    /// Force a prediction cache check without any other side effects.
    RefreshCache,
    PredictionsReady(predictions::PredictionKey, orbit::Predictions),
    PredictionFailed,
}

impl From<ViewMsg> for Message {
    fn from(message: ViewMsg) -> Self {
        Message::View(message)
    }
}

impl From<DisplayMsg> for Message {
    fn from(message: DisplayMsg) -> Self {
        Message::Display(message)
    }
}

impl From<MarksMsg> for Message {
    fn from(message: MarksMsg) -> Self {
        Message::Marks(message)
    }
}

impl From<PredictionsMsg> for Message {
    fn from(message: PredictionsMsg) -> Self {
        Message::Predictions(message)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Display {
    show_predictions: bool,
    show_grid: bool,
    show_crosshair: bool,
    absolute_axes: bool,
    show_controls: bool,
    colormap: Colormap,
    average_plotting: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            show_predictions: true,
            show_grid: false,
            show_crosshair: false,
            absolute_axes: true,
            show_controls: true,
            colormap: Default::default(),
            average_plotting: false,
        }
    }
}

/// The displayable power range, clamped to the possible range of the loaded spectrogram.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct PowerRange {
    /// Possible power range
    bounds: (f32, f32),
    /// Current power range for display
    range: (f32, f32),
}

impl PowerRange {
    pub fn set_bounds(&mut self, bounds: (f32, f32)) {
        self.bounds = bounds;
        self.range = if self.range == (0.0, 0.0) {
            bounds
        } else {
            (
                self.range.0.clamp(bounds.0, bounds.1),
                self.range.1.clamp(bounds.0, bounds.1),
            )
        };
    }

    pub fn set_min(&mut self, min: f32) {
        self.range.0 = min.min(self.range.1);
    }

    pub fn set_max(&mut self, max: f32) {
        self.range.1 = max.max(self.range.0);
    }

    /// Override the displayed power range. Clamps to the current bounds.
    pub fn set_range(&mut self, min: Option<f32>, max: Option<f32>) {
        if let Some(v) = min {
            self.range.0 = v.clamp(self.bounds.0, self.bounds.1);
        }
        if let Some(v) = max {
            self.range.1 = v.clamp(self.bounds.0, self.bounds.1);
        }
    }

    pub fn bounds(&self) -> (f32, f32) {
        self.bounds
    }

    pub fn range(&self) -> (f32, f32) {
        self.range
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub(crate) struct Detection {
    /// Threshold for signal detection
    signal_sigma: f32,
    /// Bandwidth around track points
    track_bw: f32,
}

impl Default for Detection {
    fn default() -> Self {
        Self {
            signal_sigma: 5.0,
            track_bw: 10e3,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub(crate) struct State {
    pub viewport: Viewport,
    pub power: PowerRange,
    pub detection: Detection,
    pub spectrogram_files: Vec<PathBuf>,
    #[serde(skip)]
    spectrogram: Option<Spectrogram>,
    /// The margin on the left/bottom of the plot area (for axes/labels)
    pub plot_area_margin: f32,
    pub display: Display,
    pub marks: Marks,
    #[serde(skip)]
    pub interaction: Interaction,
    #[serde(skip)]
    pub prediction_cache: AsyncCache<predictions::PredictionKey, orbit::Predictions>,
}

impl State {
    pub fn spectrogram(&self) -> Option<&Spectrogram> {
        self.spectrogram.as_ref()
    }

    pub fn set_spectrogram(&mut self, spectrogram: Option<Spectrogram>, paths: Vec<PathBuf>) {
        let spec = spectrogram.as_ref().unwrap();
        self.power.set_bounds(spec.power_bounds);
        let data = spec.bounds();
        self.viewport.set_data_bounds(data.0.width, data.0.height);
        self.spectrogram = spectrogram;
        self.spectrogram_files = paths;
    }

    pub fn update_view(&mut self, message: ViewMsg) {
        match message {
            ViewMsg::UpdateZoomX(zoom_x) => self.viewport.set_zoom_x(zoom_x),
            ViewMsg::UpdateZoomY(zoom_y) => self.viewport.set_zoom_y(zoom_y),
            ViewMsg::PanningDelta(delta) => self.viewport.pan_by(delta),
            ViewMsg::ZoomDelta(plot_pos, delta) => self.viewport.zoom_at(plot_pos, delta),
            ViewMsg::ZoomDeltaX(plot_pos, delta) => self.viewport.zoom_x_at(plot_pos, delta),
            ViewMsg::ZoomDeltaY(plot_pos, delta) => self.viewport.zoom_y_at(plot_pos, delta),
            ViewMsg::ResetView => {
                self.viewport.reset();
                self.marks.clear();
            }
            ViewMsg::ZoomToRect(rect) => self.viewport.set_view_from_rect_dn(&rect),
            ViewMsg::UpdateMinPower(min_power) => self.power.set_min(min_power),
            ViewMsg::UpdateMaxPower(max_power) => self.power.set_max(max_power),
        }
    }

    pub fn update_display(&mut self, message: DisplayMsg) {
        let display = &mut self.display;
        match message {
            DisplayMsg::TogglePredictions => display.show_predictions = !display.show_predictions,
            DisplayMsg::ToggleGrid => display.show_grid = !display.show_grid,
            DisplayMsg::ToggleCrosshair => display.show_crosshair = !display.show_crosshair,
            DisplayMsg::ToggleAbsoluteAxes => display.absolute_axes = !display.absolute_axes,
            DisplayMsg::SetControlsVisible(visible) => display.show_controls = visible,
            DisplayMsg::UpdateColormap(colormap) => display.colormap = colormap,
            DisplayMsg::UpdateAveragePlotting(average) => display.average_plotting = average,
        }
    }
}

impl State {
    pub fn update_marks(&mut self, message: MarksMsg, app: &AppShared) -> Task<Message> {
        match message {
            MarksMsg::MarkTrackpoints => {
                if matches!(self.interaction.mouse_state.get(), MouseState::Idle) {
                    self.interaction
                        .mouse_state
                        .set(MouseState::Marking(MarkAction::Trackpoint));
                }
                Task::none()
            }
            MarksMsg::MarkSignals => {
                if matches!(self.interaction.mouse_state.get(), MouseState::Idle) {
                    self.interaction
                        .mouse_state
                        .set(MouseState::Marking(MarkAction::Signal));
                }
                Task::none()
            }
            MarksMsg::AddTrackPoint(pos) => {
                log::debug!("Adding track point at position: {:?}", pos);
                self.marks.insert_track_point(pos);
                Task::none()
            }
            MarksMsg::AddSignal(pos) => {
                log::debug!("Manually adding signal at position: {:?}", pos);
                self.marks.signals_mut().push(pos);
                Task::none()
            }
            MarksMsg::DeleteMark(action, point) => {
                log::debug!("Deleting {:?} mark at position: {:?}", action, point);
                self.marks.remove(action, point);
                Task::none()
            }
            MarksMsg::DeleteInRect(rect) => {
                self.marks.retain(|p| !rect.contains(*p));
                Task::none()
            }
            MarksMsg::MarkCentroid(rect) => {
                let centroid = self
                    .spectrogram
                    .as_ref()
                    .and_then(|spec| signal::centroid(spec, rect));
                self.marks.signals_mut().extend(centroid);
                Task::none()
            }
            MarksMsg::ClearAll => {
                self.marks.clear();
                Task::none()
            }
            MarksMsg::FindSignals => {
                if self.marks.track_points().len() < 2 {
                    Task::none()
                } else {
                    let Some(spectrogram) = &self.spectrogram else {
                        log::error!("No spectrogram loaded, cannot find signals");
                        return Task::none();
                    };
                    let spectrogram = spectrogram.clone();
                    let track_points = self.marks.track_points().to_vec();
                    let sigma = self.detection.signal_sigma;
                    let track_bw = self.detection.track_bw;
                    Task::future(async move {
                        tokio::task::spawn_blocking(move || {
                            let signals = signal::find_signals(
                                &spectrogram,
                                &track_points,
                                track_bw,
                                signal::SignalDetectionMethod::FitTrace { sigma },
                            );
                            let signals = match signals {
                                Err(e) => {
                                    log::error!("Error finding signals: {}", e);
                                    Vec::new()
                                }
                                Ok(signals) => {
                                    log::info!("Found {} signal peaks", signals.len());
                                    signals
                                }
                            };
                            MarksMsg::FoundSignals(signals).into()
                        })
                        .await
                        .unwrap()
                    })
                }
            }
            MarksMsg::FoundSignals(signals) => {
                *self.marks.signals_mut() = signals;
                Task::none()
            }
            MarksMsg::SaveSignals => {
                let Some(spectrogram) = &self.spectrogram else {
                    log::error!("No spectrogram loaded, cannot save signals");
                    return Task::none();
                };
                let Some(site_id) = app.site_id else {
                    log::error!("No site configured, cannot save signals");
                    return Task::none();
                };
                let start_time = spectrogram.start_time();
                let start_mjd = start_time.timestamp_millis() as f64 / 86_400_000.0 + 40587.0;
                let center_freq = spectrogram.freq as f64;
                let suggested = signals_filename(start_time, center_freq, self.marks.signals())
                    .unwrap_or_else(|| "out.dat".to_owned());
                let mut output = String::new();
                for sig in self.marks.signals() {
                    let mjd = start_mjd + sig.0.x as f64 / 86400.0;
                    let freq = center_freq + sig.0.y as f64;
                    output.push_str(&format!("{mjd:.6} {freq:.6} 5.000000 {site_id}\n"));
                }
                Task::future(async move {
                    let path = AsyncFileDialog::new()
                        .set_file_name(suggested.as_str())
                        .save_file()
                        .await
                        .map(|f| f.path().to_path_buf());
                    MarksMsg::WriteSignals(output, path).into()
                })
            }
            MarksMsg::WriteSignals(_, None) => Task::none(),
            MarksMsg::WriteSignals(output, Some(path)) => {
                let n = output.lines().count();
                match std::fs::write(&path, &output) {
                    Ok(()) => log::info!("Wrote {n} signals to {path:?}"),
                    Err(e) => log::error!("Failed to write {path:?}: {e}"),
                }
                Task::none()
            }
            MarksMsg::SpectrogramUpdated => {
                self.marks.clear();
                self.interaction.crosshair.set(None);
                self.check_cache(app)
            }
            MarksMsg::UpdateSignalSigma(sigma) => {
                self.detection.signal_sigma = sigma;
                Task::none()
            }
            MarksMsg::UpdateTrackBW(bw) => {
                self.detection.track_bw = bw;
                Task::none()
            }
        }
    }

    pub fn update_predictions(
        &mut self,
        message: PredictionsMsg,
        app: &AppShared,
    ) -> Task<Message> {
        match message {
            PredictionsMsg::RefreshCache => {
                self.prediction_cache.reset();
                self.check_cache(app)
            }
            PredictionsMsg::PredictionsReady(key, predictions) => {
                log::debug!("Using {} satellite predictions", predictions.n_satellites());
                self.prediction_cache.store(key, predictions);
                Task::none()
            }
            PredictionsMsg::PredictionFailed => {
                log::error!("Prediction failed");
                Task::none()
            }
        }
    }
}

/// Initial view constraints set from CLI args, applied once the spectrogram is loaded.
#[derive(Clone, PartialEq)]
pub struct InitialView {
    pub fmin: Option<f64>,
    pub fmax: Option<f64>,
    /// Unix timestamps (seconds)
    pub tmin: Option<f64>,
    pub tmax: Option<f64>,
    pub zmin: Option<f32>,
    pub zmax: Option<f32>,
}

#[derive(Default, Clone, PartialEq)]
enum LoadingState {
    #[default]
    Idle,
    LoadingFiles {
        loaded: usize,
        total: usize,
    },
    GpuUploading,
}

/// Subscription identity + wakeup handle for the GPU-upload-done signal.
/// Hashed by `spec_id` so iced restarts the subscription for each new spectrogram.
#[derive(Clone)]
struct GpuDoneWatcher {
    spec_id: Uuid,
    notify: Arc<tokio::sync::Notify>,
}

impl std::hash::Hash for GpuDoneWatcher {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.spec_id.hash(state);
    }
}

fn gpu_done_stream(
    watcher: &GpuDoneWatcher,
) -> Pin<Box<dyn Stream<Item = WindowOut<Message>> + Send>> {
    let notify = watcher.notify.clone();
    Box::pin(iced::stream::channel(1, async move |mut sender| {
        notify.notified().await;
        sender
            .send(WindowOut::Msg(Message::GpuUploadDone))
            .await
            .ok();
        std::future::pending::<()>().await;
    }))
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RFPlot {
    state: State,
    id: Uuid,
    #[serde(skip)]
    initial_view: Option<Box<InitialView>>,
    #[serde(skip)]
    loading_state: LoadingState,
    #[serde(skip)]
    pending_paths: Vec<PathBuf>,
    /// Watcher passed to the GPU-done subscription; keyed by `spec_id`.
    #[serde(skip)]
    gpu_watcher: Option<GpuDoneWatcher>,
    /// Handle given to `Primitive` so `prepare()` can fire the wakeup.
    #[serde(skip)]
    pub gpu_notify: Option<Arc<tokio::sync::Notify>>,
}

impl RFPlot {
    pub fn new() -> Self {
        let state = State {
            plot_area_margin: 75.0,
            ..Default::default()
        };
        let id = Uuid::new_v4();
        Self {
            state,
            id,
            initial_view: None,
            loading_state: LoadingState::default(),
            pending_paths: Vec::new(),
            gpu_watcher: None,
            gpu_notify: None,
        }
    }

    pub fn with_initial_view(files: Vec<PathBuf>, view: InitialView) -> Self {
        let mut rfplot = Self::new();
        rfplot.state.spectrogram_files = files;
        rfplot.initial_view = Some(Box::new(view));
        rfplot
    }

    // TODO
    pub fn app_event(&mut self, event: AppEvent, app: &AppShared) -> Task<WindowOut<Message>> {
        if matches!(event, AppEvent::ConfigUpdated) {
            self.state.update_display(DisplayMsg::UpdateAveragePlotting(
                app.config.average_plotting,
            ));
        }
        // Trigger a prediction refresh (in case we e.g. changed the site coordinates)
        self.state
            .update_predictions(PredictionsMsg::RefreshCache, app)
            .map(WindowOut::Msg)
    }
}

fn apply_initial_view(state: &mut State, iv: &InitialView) {
    let Some(spec) = state.spectrogram() else {
        log::error!("Tried to apply initial view but no spectrogram is loaded");
        return;
    };
    let spec_bounds = spec.bounds();
    let length_secs = spec_bounds.0.width as f64;
    let bw = spec_bounds.0.height as f64;
    let center_freq = spec.freq as f64;

    let t_min = iv.tmin.unwrap_or(0.0) as f32;
    let t_max = iv.tmax.unwrap_or(length_secs) as f32;
    let f_min = iv.fmin.map(|f| f - center_freq).unwrap_or(-bw / 2.0) as f32;
    let f_max = iv.fmax.map(|f| f - center_freq).unwrap_or(bw / 2.0) as f32;

    if t_max > t_min && f_max > f_min {
        use rstrf::coord::data_absolute;
        let view_rect = data_absolute::Rectangle::new(
            data_absolute::Point::new(t_min, f_min),
            data_absolute::Size::new(t_max - t_min, f_max - f_min),
        );
        state
            .viewport
            .set_view_from_rect_da(&view_rect, &spec_bounds);
    }
    state.power.set_range(iv.zmin, iv.zmax);
}

impl Window<Message> for RFPlot {
    fn init(&mut self, id: window::Id, app: &AppShared) -> Task<WindowOut<Message>> {
        self.state
            .update_display(DisplayMsg::UpdateColormap(app.config.default_colormap));
        self.state.update_display(DisplayMsg::UpdateAveragePlotting(
            app.config.average_plotting,
        ));
        if self.state.spectrogram_files.is_empty() {
            Task::none()
        } else {
            self.update(
                id,
                Message::LoadSpectrogram(self.state.spectrogram_files.clone()),
                app,
            )
        }
    }

    fn menu_bar(&self) -> Vec<MenuItem<WindowOut<Message>>> {
        vec![MenuItem::Submenu {
            label: "File".to_string(),
            msg: Some(Message::Nop.into()),
            items: vec![MenuItem::Button {
                label: "Load spectrogram(s)".to_string(),
                msg: Some(Message::PickSpectrogram.into()),
            }],
        }]
    }

    fn view<'a>(&'a self, app: &'a AppShared) -> Element<'a, WindowOut<Message>> {
        match &self.loading_state {
            LoadingState::LoadingFiles { loaded, total } => {
                return container(widget::text(format!(
                    "Loading spectrograms... {loaded}/{total}"
                )))
                .center(Length::Fill)
                .into();
            }
            LoadingState::GpuUploading => {
                // The shader must be in the tree so prepare() fires and creates GPU buffers.
                // The text overlay communicates loading status on top.
                let shader: Element<'_, Message> = widget::shader(self)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into();
                let loading_text: Element<'_, Message> =
                    container(widget::text("Uploading to GPU..."))
                        .center(Length::Fill)
                        .into();
                let stack: Element<'_, Message> = widget::stack![shader, loading_text]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into();
                return stack.map(WindowOut::Msg);
            }
            LoadingState::Idle => {}
        }

        // The plot is implemented as a stack of two layers: the spectrogram itself (see
        // `shader.rs`) and the overlay (see `overlay.rs`).
        if self.state.spectrogram().is_none() {
            return container(
                button("Open Spectrogram")
                    .style(button::primary)
                    .on_press(Message::PickSpectrogram.into()),
            )
            .center(Length::Fill)
            .into();
        }

        let controls = toolbar::view(&self.state).map(Message::from);

        let spectrogram: Element<'_, Message> = container(
            widget::shader(self)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: self.state.plot_area_margin,
            left: self.state.plot_area_margin,
        })
        .into();
        let plot_overlay: Element<'_, Message> = ChartWidget::new(PlotChart {
            state: &self.state,
            app,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

        let status = self.state.status(app);

        let mut stack = widget::stack![spectrogram, plot_overlay];
        if let Some(status) = status {
            let indicator: Element<'_, Message> = container(
                container(widget::text(status).size(12))
                    .style(container::secondary)
                    .padding(Padding {
                        left: 8.0,
                        right: 8.0,
                        top: 4.0,
                        bottom: 4.0,
                    })
                    .height(Length::Shrink)
                    .width(Length::Shrink),
            )
            .align_x(Horizontal::Left)
            .align_y(Vertical::Bottom)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
            stack = stack.push(indicator);
        }
        let plot_area: Element<'_, Message> = stack.into();

        let contents: Element<'_, Message> = widget::column![controls, plot_area]
            .padding(8)
            .spacing(4)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        contents.map(WindowOut::Msg)
    }

    fn update(
        &mut self,
        id: window::Id,
        message: Message,
        app: &AppShared,
    ) -> Task<WindowOut<Message>> {
        let result = match message {
            Message::GpuUploadDone => {
                self.loading_state = LoadingState::Idle;
                self.gpu_watcher = None;
                self.gpu_notify = None;
                if let Some(spec) = &self.state.spectrogram() {
                    return Task::done(WindowOut::Effect(WindowEffect::PlotReady(
                        id,
                        spec.absolute_bounds(),
                    )));
                }
                Task::none()
            }
            Message::SaveScreenshot(img, path) => {
                match img.0.save(&path) {
                    Ok(_) => log::info!("Saved screenshot to {path:?}"),
                    Err(e) => log::error!("Failed to save screenshot to {path:?}: {e}"),
                }
                return Task::done(WindowOut::Effect(WindowEffect::ScreenshotSaved(path)));
            }
            Message::LoadSpectrogram(paths) => {
                let total = paths.len();
                self.pending_paths = paths;
                self.loading_state = LoadingState::LoadingFiles { loaded: 0, total };
                return Task::done(WindowOut::Effect(WindowEffect::ReloadCatalog));
            }
            Message::View(message) => {
                self.state.update_view(message);
                Task::none()
            }
            Message::Display(message) => {
                self.state.update_display(message);
                Task::none()
            }
            Message::Marks(message) => self.state.update_marks(message, app),
            Message::Predictions(message) => self.state.update_predictions(message, app),
            Message::Refresh => Task::none(),
            Message::LoadProgress { loaded, total } => {
                self.loading_state = LoadingState::LoadingFiles { loaded, total };
                Task::none()
            }
            Message::SpectrogramLoaded(result) => match result {
                Ok((paths, spec)) => {
                    log::info!("Loaded spectrogram: {spec:?}");
                    let spec_id = spec.id;
                    self.state.set_spectrogram(Some(spec), paths);
                    if let Some(iv) = self.initial_view.take() {
                        apply_initial_view(&mut self.state, &iv);
                    }

                    let notify = Arc::new(tokio::sync::Notify::new());
                    self.gpu_notify = Some(notify.clone());
                    self.gpu_watcher = Some(GpuDoneWatcher { spec_id, notify });
                    self.loading_state = LoadingState::GpuUploading;

                    self.state.update_marks(MarksMsg::SpectrogramUpdated, app)
                }
                Err(err) => {
                    log::error!("Failed to load spectrogram: {err}");
                    self.loading_state = LoadingState::Idle;
                    Task::none()
                }
            },
            Message::PickSpectrogram => Task::future(async {
                let files = AsyncFileDialog::new()
                    .add_filter("Supported spectrogram formats", &["rstrf", "bin"])
                    .add_filter("rSTRF spectrograms", &["rstrf"])
                    .add_filter("STRF spectrograms", &["bin"])
                    .add_filter("All files", &["*"])
                    .pick_files()
                    .await;
                if let Some(files) = files
                    && !files.is_empty()
                {
                    Message::LoadSpectrogram(files.iter().map(|f| f.path().to_path_buf()).collect())
                } else {
                    Message::Nop
                }
            }),
            Message::CaptureScreenshot(path) => window::screenshot(id).map(move |screenshot| {
                let width = screenshot.size.width;
                let height = screenshot.size.height;
                Message::CapturedScreenshot(
                    RgbaImage::from_raw(width, height, screenshot.rgba.to_vec())
                        .map(|img| (img.into(), path.clone()))
                        .ok_or_else(|| "Screenshot buffer size mismatch".to_string()),
                )
            }),
            Message::CapturedScreenshot(Err(err)) => {
                log::error!("Failed to capture screenshot: {err}");
                Task::none()
            }
            Message::CapturedScreenshot(Ok((img, Some(path)))) => {
                Task::done(Message::SaveScreenshot(img, path))
            }
            Message::CapturedScreenshot(Ok((img, None))) => Task::future(async move {
                match AsyncFileDialog::new()
                    .add_filter("PNG image", &["png"])
                    .add_filter("All files", &["*"])
                    .set_file_name("screenshot.png")
                    .save_file()
                    .await
                {
                    Some(file) => Message::SaveScreenshot(img, file.path().to_path_buf()),
                    None => Message::Nop,
                }
            }),
            Message::SetView(rect) => {
                self.state.update_view(ViewMsg::ZoomToRect(rect));
                Task::none()
            }
            Message::Nop => Task::none(),
        };
        result.map(WindowOut::Msg)
    }

    fn subscription(&self, app: &AppShared) -> Subscription<WindowOut<Message>> {
        let mut subs = Vec::new();

        if matches!(self.loading_state, LoadingState::LoadingFiles { .. }) {
            subs.push(
                io_service::load_subscription(self.pending_paths.clone(), app.freq_range).map(
                    |e| match e {
                        io_service::Event::Progress { loaded, total } => {
                            WindowOut::Msg(Message::LoadProgress { loaded, total })
                        }
                        io_service::Event::Done(r) => WindowOut::Msg(Message::SpectrogramLoaded(r)),
                    },
                ),
            );
        }

        if let Some(watcher) = &self.gpu_watcher {
            subs.push(Subscription::run_with(watcher.clone(), gpu_done_stream));
        }

        Subscription::batch(subs)
    }

    fn title(&self) -> String {
        format!(
            "Plot: {}",
            self.state
                .spectrogram()
                .map(|s| s.start_time().to_string())
                .unwrap_or("Loading...".to_string())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstrf::coord::data_absolute;

    use crate::{app::AppShared, windows::Window};

    #[test]
    fn set_power_bounds_initializes_range() {
        let mut p = PowerRange::default();
        p.set_bounds((-50.0, -10.0));
        assert_eq!(p.range(), (-50.0, -10.0));
    }

    #[test]
    fn set_power_bounds_clamps_existing_range() {
        let mut p = PowerRange::default();
        p.set_bounds((-50.0, -10.0));
        p.set_bounds((-30.0, -20.0));
        let (lo, hi) = p.range();
        assert!(lo >= -30.0 && lo <= -20.0, "lo={}", lo);
        assert!(hi >= -30.0 && hi <= -20.0, "hi={}", hi);
    }

    #[test]
    fn update_min_power_cannot_exceed_max() {
        let mut p = PowerRange::default();
        p.set_bounds((-50.0, -10.0));
        p.set_max(-20.0);
        p.set_min(-10.0);
        let (lo, hi) = p.range();
        assert!(lo <= hi, "lo={} > hi={}", lo, hi);
    }

    #[test]
    fn reset_view_clears_marks_and_zoom() {
        let mut rfplot = RFPlot::new();
        rfplot
            .state
            .marks
            .insert_track_point(data_absolute::Point::new(1.0, 2.0));
        rfplot
            .state
            .marks
            .signals_mut()
            .push(data_absolute::Point::new(3.0, 4.0));
        rfplot.state.update_view(ViewMsg::UpdateZoomX(5.0));

        let app = AppShared::default();
        let _ = rfplot.update(
            window::Id::unique(),
            Message::View(ViewMsg::ResetView),
            &app,
        );

        assert!(rfplot.state.marks.track_points().is_empty());
        assert!(rfplot.state.marks.signals().is_empty());
        assert!((rfplot.state.viewport.size().0.width - 1.0).abs() < 1e-6);
        assert!((rfplot.state.viewport.size().0.height - 1.0).abs() < 1e-6);
    }

    #[test]
    fn set_controls_visible_changes_visibility() {
        let mut state = State::default();
        assert!(state.display.show_controls);
        state.update_display(DisplayMsg::SetControlsVisible(false));
        assert!(!state.display.show_controls);
        state.update_display(DisplayMsg::SetControlsVisible(true));
        assert!(state.display.show_controls);
    }
}
