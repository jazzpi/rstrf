use std::path::PathBuf;

use iced::{
    Element, Length, Padding, Task, keyboard,
    widget::{self, button, container},
};
use plotters_iced2::ChartWidget;
use rfd::AsyncFileDialog;
use rstrf::{coord::plot_area, menu::MenuItem, spectrogram::Spectrogram};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app::{AppEvent, AppShared},
    windows::{Window, WindowOut, rfplot::control::Controls},
};

mod control;
pub mod overlay;
mod shader;

#[derive(Debug, Clone)]
pub enum Message {
    Control(control::Message),
    Overlay(overlay::Message),
    PickSpectrogram,
    LoadSpectrogram(Vec<PathBuf>),
    SpectrogramLoaded(Result<(Vec<PathBuf>, Spectrogram), String>),
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

#[derive(Default)]
pub struct MouseInteraction {
    pub mouse: MouseState,
    pub modifiers: keyboard::Modifiers,
}

#[derive(Clone, Copy, Debug)]
pub enum RectAction {
    Delete,
    Zoom,
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
}

#[derive(Serialize, Deserialize, PartialEq, Default, Clone)]
struct SharedState {
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

#[derive(Serialize, Deserialize, PartialEq, Clone)]
pub struct RFPlot {
    shared: SharedState,
    overlay: overlay::Overlay,
    id: Uuid,
    #[serde(skip)]
    initial_view: Option<Box<InitialView>>,
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
    fn init(&mut self, app: &AppShared) -> Task<WindowOut<Message>> {
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

    fn update(&mut self, message: Message, app: &AppShared) -> Task<WindowOut<Message>> {
        let result = match message {
            Message::Control(message) => self.shared.controls.update(message).map(Message::Control),
            Message::Overlay(message) => self
                .overlay
                .update(message, &self.shared, app)
                .map(Message::Overlay),
            Message::LoadSpectrogram(paths) => Task::future(async move {
                let spec = rstrf::spectrogram::load(&paths).await;
                Message::SpectrogramLoaded(spec.map(|s| (paths, s)).map_err(|e| format!("{e:?}")))
            }),
            Message::SpectrogramLoaded(result) => match result {
                Ok((paths, spec)) => {
                    log::info!("Loaded spectrogram: {spec:?}");
                    self.shared.controls.set_power_bounds(spec.power_bounds);
                    if let Some(iv) = self.initial_view.take() {
                        apply_initial_view(&mut self.shared.controls, &spec, &iv);
                    }
                    self.shared.spectrogram = Some(spec);
                    self.shared.spectrogram_files = paths;
                    self.overlay
                        .update(overlay::Message::SpectrogramUpdated, &self.shared, app)
                        .map(Message::Overlay)
                }
                Err(err) => {
                    log::error!("Failed to load spectrogram: {err}");
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
            Message::Nop => Task::none(),
        };
        result.map(WindowOut::Msg)
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
