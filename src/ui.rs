use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, FontFamily, RichText, Vec2};

use crate::graphics::{Frame, Triangle};
use crate::i18n::{Language, Text};
use crate::icons::{Icon, IconButton};
use crate::mpv::Playback;
use crate::settings::Preferences;
use crate::settings_ui::{self, SettingsPage};
use crate::vr::RuntimeInfo;

const PANEL: Color32 = Color32::from_rgb(25, 31, 42);
const BACKDROP: Color32 = Color32::from_rgb(8, 11, 17);
const ACCENT: Color32 = Color32::from_rgb(139, 195, 255);
const MUTED: Color32 = Color32::from_rgb(165, 177, 194);
const ERROR: Color32 = Color32::from_rgb(255, 173, 157);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum VrPhase {
    #[default]
    Idle,
    Probing,
    Starting,
    Waiting,
    Playing,
    Stopping,
}

impl VrPhase {
    pub fn can_toggle(self) -> bool {
        !matches!(self, Self::Probing | Self::Stopping)
    }

    fn label(self, language: Language) -> &'static str {
        match self {
            Self::Idle => language.text(Text::VrOff),
            Self::Probing => language.text(Text::OpenXrChecking),
            Self::Starting => language.text(Text::VrConnecting),
            Self::Waiting => language.text(Text::VrWaitingForHeadset),
            Self::Playing => language.text(Text::VrPlaying),
            Self::Stopping => language.text(Text::VrLeaving),
        }
    }
}

#[derive(Debug)]
pub enum Action {
    Open,
    OpenFile(PathBuf),
    PreviousFile,
    NextFile,
    TogglePause,
    Seek(f64),
    SeekBy(f64),
    Volume(f64),
    VolumeBy(f64),
    ToggleMute,
    ToggleFullscreen,
    ProbeVr,
    ToggleVr,
    Preferences {
        before: Preferences,
        edited: Preferences,
    },
}

pub struct PlayerUi {
    pub playback: Playback,
    pub preferences: Preferences,
    pub phase: VrPhase,
    pub runtime: Option<RuntimeInfo>,
    pub vr_error: Option<String>,
    pub media_available: bool,
    pub picker_open: bool,
    settings_open: bool,
    settings_page: SettingsPage,
    scrub: Option<f64>,
}

impl PlayerUi {
    pub fn new(
        media_available: bool,
        media_error: Option<String>,
        preferences: Preferences,
    ) -> Self {
        let mut playback = Playback::default();
        playback.volume = f64::from(preferences.initial_volume);
        if let Some(error) = media_error {
            playback.set_error(error);
        }
        Self {
            playback,
            preferences,
            phase: VrPhase::Idle,
            runtime: None,
            vr_error: None,
            media_available,
            picker_open: false,
            settings_open: false,
            settings_page: SettingsPage::default(),
            scrub: None,
        }
    }

    pub fn reset_scrub(&mut self) {
        self.scrub = None;
    }

    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        frame: Option<Frame>,
        preview: Triangle,
        fullscreen: bool,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        egui::TopBottomPanel::top("files")
            .frame(panel(12, 9))
            .show(ctx, |ui| self.header(ui, &mut actions));
        egui::TopBottomPanel::bottom("transport")
            .frame(panel(12, 10))
            .show(ctx, |ui| self.transport(ui, fullscreen, &mut actions));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BACKDROP))
            .show(ctx, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
                if let Some(frame) = frame {
                    let picture = fit(rect, frame.aspect());
                    if picture.is_positive() {
                        ui.painter().add(egui::PaintCallback {
                            rect: picture,
                            callback: Arc::new(egui_glow::CallbackFn::new(move |_, painter| {
                                preview.draw_desktop(painter.gl(), frame);
                            })),
                        });
                    }
                } else {
                    let message = if self.playback.loading {
                        Text::VideoOpening
                    } else if self.playback.loaded {
                        Text::NoVideoTrack
                    } else {
                        Text::DropVideoHere
                    };
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        self.preferences.language.text(message),
                        egui::FontId::proportional(17.0),
                        MUTED,
                    );
                }
                if response.double_clicked() {
                    actions.push(Action::ToggleFullscreen);
                }
            });
        if self.settings_open {
            egui::Window::new(self.preferences.language.text(Text::Settings))
                .id(egui::Id::new("player-settings"))
                .id(egui::Id::new("player-settings"))
                .open(&mut self.settings_open)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .collapsible(false)
                .auto_sized()
                .min_width(425.0)
                .show(ctx, |ui| {
                    settings_ui::draw(ui, &mut self.preferences, &mut self.settings_page, 1.0);
                });
        }
        actions
    }

    fn header(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let text = |key| self.preferences.language.text(key);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.media_available && !self.picker_open,
                    IconButton::new(Icon::Folder, text(Text::OpenFileShortcut)),
                )
                .clicked()
            {
                actions.push(Action::Open);
            }
            let vr_button = match self.phase {
                VrPhase::Idle | VrPhase::Probing => {
                    egui::Button::new(RichText::new(text(Text::EnterVr)).strong().color(BACKDROP))
                        .fill(ACCENT)
                }
                VrPhase::Starting | VrPhase::Waiting => egui::Button::new(text(Text::CancelVr)),
                VrPhase::Playing | VrPhase::Stopping => egui::Button::new(text(Text::ExitVr)),
            };
            if ui
                .add_enabled(self.media_available && self.phase.can_toggle(), vr_button)
                .on_hover_text("V")
                .clicked()
            {
                actions.push(Action::ToggleVr);
            }
            if ui
                .add_enabled(
                    self.phase == VrPhase::Idle,
                    egui::Button::new(text(Text::CheckHeadset)),
                )
                .clicked()
            {
                actions.push(Action::ProbeVr);
            }
            if ui
                .add(
                    IconButton::new(Icon::Settings, text(Text::Settings))
                        .selected(self.settings_open),
                )
                .clicked()
            {
                self.settings_open = !self.settings_open;
            }
            if let (Some(title), Some(path)) = (self.playback.title(), &self.playback.file) {
                ui.add(
                    egui::Label::new(RichText::new(title.as_ref()).size(15.0).strong()).truncate(),
                )
                .on_hover_text(path.to_string_lossy());
            }
        });

        ui.add_space(5.0);
        ui.horizontal_wrapped(|ui| {
            let status_color = if self.phase == VrPhase::Playing {
                ACCENT
            } else {
                MUTED
            };
            ui.label(
                RichText::new(self.phase.label(self.preferences.language))
                    .small()
                    .color(status_color),
            );
            if let Some(runtime) = &self.runtime {
                ui.label(
                    RichText::new(format!("{}  ·  {}", runtime.runtime, runtime.headset))
                        .small()
                        .color(MUTED),
                );
            }
        });
        for error in [self.vr_error.as_deref(), self.playback.error.as_deref()]
            .into_iter()
            .flatten()
        {
            ui.label(RichText::new(error).color(ERROR));
        }
    }

    fn transport(&mut self, ui: &mut egui::Ui, fullscreen: bool, actions: &mut Vec<Action>) {
        let playback = &self.playback;
        let language = self.preferences.language;
        let text = |key| language.text(key);
        let mut position = self.scrub.or(playback.position).unwrap_or(0.0);
        let duration = playback.duration.unwrap_or(0.0);
        let show_hours = duration.max(position) >= 3600.0;
        let position_label = time_label(self.scrub.or(playback.position), show_hours);
        let duration_label = time_label(playback.duration, show_hours);
        ui.horizontal(|ui| {
            ui.label(RichText::new(position_label).monospace());
            ui.spacing_mut().slider_width =
                (ui.available_width() - if show_hours { 71.0 } else { 49.0 }).max(40.0);
            let seek = ui.add_enabled(
                playback.loaded && playback.seekable && duration > 0.0,
                egui::Slider::new(&mut position, 0.0..=duration.max(0.001))
                    .show_value(false)
                    .trailing_fill(true),
            );
            if seek.is_pointer_button_down_on() {
                self.scrub = Some(position);
            } else if self.scrub.take().is_some() || seek.changed() {
                actions.push(Action::Seek(position));
            }
            ui.label(RichText::new(duration_label).monospace().color(MUTED));
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let (play_icon, play_label) = if playback.paused || playback.eof || !playback.loaded {
                (Icon::Play, text(Text::PlayShortcut))
            } else {
                (Icon::Pause, text(Text::PauseShortcut))
            };
            for (icon, label, enabled, action) in [
                (
                    Icon::Previous,
                    text(Text::PreviousVideo),
                    !playback.loading && playback.nearby.previous.is_some(),
                    Action::PreviousFile,
                ),
                (play_icon, play_label, playback.loaded, Action::TogglePause),
                (
                    Icon::Next,
                    text(Text::NextVideo),
                    !playback.loading && playback.nearby.next.is_some(),
                    Action::NextFile,
                ),
            ] {
                let mut button = IconButton::new(icon, label);
                if matches!(action, Action::TogglePause) {
                    button = button.size([48.0, 32.0]).primary();
                }
                if ui.add_enabled(enabled, button).clicked() {
                    actions.push(action);
                }
            }
            let step = self.preferences.pc_seek;
            for (sign, prefix, key) in [(-1.0, "−", "←"), (1.0, "+", "→")] {
                if ui
                    .add_enabled(
                        playback.loaded && playback.seekable,
                        egui::Button::new(format!("{prefix}{step}{}", text(Text::SecondsSuffix)))
                            .min_size(Vec2::new(46.0, 30.0)),
                    )
                    .on_hover_text(key)
                    .clicked()
                {
                    actions.push(Action::SeekBy(f64::from(step) * sign));
                }
            }
            ui.separator();
            let (icon, label) = if playback.muted {
                (Icon::Muted, text(Text::UnmuteShortcut))
            } else {
                (Icon::Volume, text(Text::MuteShortcut))
            };
            if ui
                .add_enabled(self.media_available, IconButton::new(icon, label))
                .clicked()
            {
                actions.push(Action::ToggleMute);
            }
            let mut volume = playback.volume;
            ui.spacing_mut().slider_width = 88.0;
            if ui
                .add_enabled(
                    self.media_available,
                    egui::Slider::new(&mut volume, 0.0..=100.0)
                        .integer()
                        .suffix("%"),
                )
                .on_hover_text(text(Text::VolumeShortcut))
                .changed()
            {
                actions.push(Action::Volume(volume));
            }
            ui.separator();
            let (icon, label) = if fullscreen {
                (Icon::Windowed, text(Text::ExitFullscreen))
            } else {
                (Icon::Fullscreen, text(Text::Fullscreen))
            };
            if ui.add(IconButton::new(icon, label)).clicked() {
                actions.push(Action::ToggleFullscreen);
            }
        });
    }
}

pub fn configure(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "player".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/NotoSansKR-Player.ttf"
        ))),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "player".to_owned());
    ctx.set_fonts(fonts);

    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(9.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.visuals.panel_fill = PANEL;
    style.visuals.selection.bg_fill = Color32::from_rgb(44, 74, 112);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(41, 49, 63);
    style.visuals.widgets.inactive.weak_bg_fill = style.visuals.widgets.inactive.bg_fill;
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(58, 71, 89);
    style.visuals.widgets.hovered.weak_bg_fill = style.visuals.widgets.hovered.bg_fill;
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn panel(x: i8, y: i8) -> egui::Frame {
    egui::Frame::NONE
        .fill(PANEL)
        .inner_margin(egui::Margin::symmetric(x, y))
}

fn fit(rect: egui::Rect, aspect: f32) -> egui::Rect {
    let width = rect.width().min(rect.height() * aspect);
    egui::Rect::from_center_size(rect.center(), Vec2::new(width, width / aspect))
}

pub fn time_label(seconds: Option<f64>, show_hours: bool) -> String {
    let Some(seconds) = seconds else {
        return "--:--".to_owned();
    };
    let seconds = seconds.max(0.0) as u64;
    let minutes = seconds / 60;
    if show_hours {
        format!(
            "{:02}:{:02}:{:02}",
            minutes / 60,
            minutes % 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", minutes, seconds % 60)
    }
}
