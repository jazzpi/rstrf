use std::{path::PathBuf, pin::Pin, sync::Arc};

use futures_util::{SinkExt, Stream};
use iced::{
    Element, Length, Padding, Subscription, Task,
    widget::{self, button, container},
    window,
};
use image::RgbaImage;
use plotters_iced2::ChartWidget;
use rfd::AsyncFileDialog;
use rstrf::{
    coord::{data_normalized, plot_area},
    menu::MenuItem,
    spectrogram::Spectrogram,
    util::DebugRgbaImage,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app::{self, AppEvent, AppShared},
    io_service,
    windows::{Window, WindowEffect, WindowOut, rfplot::control::Controls},
};

pub mod control;
pub mod overlay;
mod shader;

#[derive(Debug, Clone)]
pub enum Message {
    Control(control::Message),
    Overlay(overlay::Message),
    PickSpectrogram,
    LoadSpectrogram(Vec<PathBuf>),
    SpectrogramLoaded(Result<(Vec<PathBuf>, Spectrogram), String>),
    LoadProgress { loaded: usize, total: usize },
    GpuUploadDone,
    SetView(data_normalized::Rectangle),
    CaptureScreenshot(Option<PathBuf>),
    CapturedScreenshot(Result<(DebugRgbaImage, Option<PathBuf>), String>),
    SaveScreenshot(DebugRgbaImage, PathBuf),
    Nop,
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

#[derive(Clone, Copy, Debug)]
pub enum RectAction {
    Delete,
    Zoom,
}

#[derive(Clone, Copy, Debug)]
pub enum MarkAction {
    Trackpoint,
    Signal,
}

#[derive(Default, Clone, Copy, Debug)]
pub enum MouseState {
    #[default]
    Idle,
    Panning(plot_area::Point),
    DrawingRect {
        action: RectAction,
        corner1: plot_area::Point,
        corner2: plot_area::Point,
    },
    Marking(MarkAction),
}

#[derive(Serialize, Deserialize, PartialEq, Default, Clone)]
pub(crate) struct SharedState {
    pub controls: Controls,
    pub spectrogram_files: Vec<PathBuf>,
    #[serde(skip)]
    pub spectrogram: Option<Spectrogram>,
    /// The margin on the left/bottom of the plot area (for axes/labels)
    pub plot_area_margin: f32,
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
    shared: SharedState,
    overlay: overlay::Overlay,
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

impl PartialEq for RFPlot {
    fn eq(&self, other: &Self) -> bool {
        self.shared == other.shared && self.overlay == other.overlay && self.id == other.id
    }
}

impl RFPlot {
    pub fn new() -> Self {
        let shared = SharedState {
            plot_area_margin: 75.0,
            ..Default::default()
        };
        let id = Uuid::new_v4();
        Self {
            shared,
            overlay: overlay::Overlay::default(),
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
        rfplot.shared.spectrogram_files = files;
        rfplot.initial_view = Some(Box::new(view));
        rfplot
    }

    // TODO
    pub fn app_event(&mut self, _event: AppEvent, app: &AppShared) -> Task<WindowOut<Message>> {
        // Trigger a prediction cache check
        self.overlay
            .update(overlay::Message::RefreshCache, &self.shared, app)
            .map(Message::Overlay)
            .map(WindowOut::Msg)
    }
}

fn apply_initial_view(controls: &mut Controls, spec: &Spectrogram, iv: &InitialView) {
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
        controls.set_view_from_rect_da(&view_rect, &spec_bounds);
    }
    controls.set_power_range(iv.zmin, iv.zmax);
}

impl Window<Message> for RFPlot {
    fn init(&mut self, id: window::Id, app: &AppShared) -> Task<WindowOut<Message>> {
        let cmap_task = self
            .shared
            .controls
            .update(control::Message::UpdateColormap(
                app.config.default_colormap,
            ))
            .map(Message::Control)
            .map(WindowOut::Msg);
        let spec_task = if self.shared.spectrogram_files.is_empty() {
            Task::none()
        } else {
            self.update(
                id,
                Message::LoadSpectrogram(self.shared.spectrogram_files.clone()),
                app,
            )
        };
        Task::batch(vec![cmap_task, spec_task])
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

    fn view(&self, _app: &AppShared) -> Element<'_, WindowOut<Message>> {
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
        if self.shared.spectrogram.is_none() {
            return container(
                button("Open Spectrogram")
                    .style(button::primary)
                    .on_press(Message::PickSpectrogram.into()),
            )
            .center(Length::Fill)
            .into();
        }

        let controls = self.shared.controls.view(&self.shared).map(Message::from);

        let spectrogram: Element<'_, Message> = container(
            widget::shader(self)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: self.shared.plot_area_margin,
            left: self.shared.plot_area_margin,
        })
        .into();
        let plot_overlay: Element<'_, Message> = ChartWidget::new(self)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        let plot_area: Element<'_, Message> = widget::stack![spectrogram, plot_overlay,].into();

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
        // Handle messages that (can) trigger ToApp effects first
        match message {
            Message::GpuUploadDone => {
                self.loading_state = LoadingState::Idle;
                self.gpu_watcher = None;
                self.gpu_notify = None;
                if let Some(spec) = &self.shared.spectrogram {
                    return Task::done(WindowOut::Effect(WindowEffect::ToApp(
                        app::Message::RFPlotReady(id, spec.absolute_bounds()),
                    )));
                }
            }
            Message::SaveScreenshot(img, path) => {
                match img.0.save(&path) {
                    Ok(_) => log::info!("Saved screenshot to {path:?}"),
                    Err(e) => log::error!("Failed to save screenshot to {path:?}: {e}"),
                }
                return Task::done(WindowOut::Effect(WindowEffect::ToApp(
                    app::Message::ScreenshotSaved(path),
                )));
            }
            _ => (),
        };
        let result = match message {
            Message::Control(message) => self.shared.controls.update(message).map(Message::Control),
            Message::Overlay(message) => self
                .overlay
                .update(message, &self.shared, app)
                .map(Message::Overlay),
            Message::LoadSpectrogram(paths) => {
                let total = paths.len();
                self.pending_paths = paths;
                self.loading_state = LoadingState::LoadingFiles { loaded: 0, total };
                Task::none()
            }
            Message::LoadProgress { loaded, total } => {
                self.loading_state = LoadingState::LoadingFiles { loaded, total };
                Task::none()
            }
            Message::SpectrogramLoaded(result) => match result {
                Ok((paths, spec)) => {
                    log::info!("Loaded spectrogram: {spec:?}");
                    self.shared.controls.set_spectrogram(&spec);
                    if let Some(iv) = self.initial_view.take() {
                        apply_initial_view(&mut self.shared.controls, &spec, &iv);
                    }
                    let spec_id = spec.id;
                    self.shared.spectrogram = Some(spec);
                    self.shared.spectrogram_files = paths;

                    let notify = Arc::new(tokio::sync::Notify::new());
                    self.gpu_notify = Some(notify.clone());
                    self.gpu_watcher = Some(GpuDoneWatcher { spec_id, notify });
                    self.loading_state = LoadingState::GpuUploading;

                    self.overlay
                        .update(overlay::Message::SpectrogramUpdated, &self.shared, app)
                        .map(Message::Overlay)
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
            Message::SetView(rect) => self
                .shared
                .controls
                .update(control::Message::ZoomToRect(rect))
                .map(Message::Control),
            Message::Nop => Task::none(),
            // Handled by the outer match
            Message::GpuUploadDone | Message::SaveScreenshot(_, _) => unreachable!(),
        };
        result.map(WindowOut::Msg)
    }

    fn subscription(&self) -> Subscription<WindowOut<Message>> {
        let mut subs = Vec::new();

        if matches!(self.loading_state, LoadingState::LoadingFiles { .. }) {
            subs.push(
                io_service::load_subscription(self.pending_paths.clone()).map(|e| match e {
                    io_service::Event::Progress { loaded, total } => {
                        WindowOut::Msg(Message::LoadProgress { loaded, total })
                    }
                    io_service::Event::Done(r) => WindowOut::Msg(Message::SpectrogramLoaded(r)),
                }),
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
            self.shared
                .spectrogram
                .as_ref()
                .map(|s| s.start_time().to_string())
                .unwrap_or("Loading...".to_string())
        )
    }
}
