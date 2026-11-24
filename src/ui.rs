use std::path::PathBuf;
use std::sync::Arc;

use egui::{Align, Align2, Color32, FontFamily, Layout, RichText, Vec2};

use crate::graphics::{Frame, Triangle};
use crate::i18n::{Language, Text};
use crate::icons::{Icon, IconButton};
use crate::mpv::Playback;
use crate::settings::Preferences;
use crate::settings_ui::{self, SettingsPage};
use crate::theme;
use crate::vr::RuntimeInfo;

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

    fn label(self, language: Language) -> Option<&'static str> {
        let key = match self {
            Self::Idle => return None,
            Self::Probing => Text::OpenXrChecking,
            Self::Starting => Text::VrConnecting,
            Self::Waiting => Text::VrWaitingForHeadset,
            Self::Playing => Text::VrPlaying,
            Self::Stopping => Text::VrLeaving,
        };
        Some(language.text(key))
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
    ExitVr,
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
        let colors = &theme::COLORS;
        let mut actions = Vec::new();
        egui::TopBottomPanel::top("files")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(colors.background)
                    .inner_margin(egui::Margin::symmetric(16, 7)),
            )
            .show(ctx, |ui| self.header(ui, &mut actions));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(colors.background)
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12,
                        top: 0,
                        bottom: 12,
                    }),
            )
            .show(ctx, |ui| {
                egui::TopBottomPanel::bottom("transport")
                    .show_separator_line(false)
                    .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(15, 6)))
                    .show_inside(ui, |ui| self.transport(ui, fullscreen, &mut actions));
                self.picture(ui, frame, preview, &mut actions);
            });
        if self.settings_open {
            self.settings_dialog(ctx);
        }
        actions
    }

    fn picture(
        &self,
        ui: &mut egui::Ui,
        frame: Option<Frame>,
        preview: Triangle,
        actions: &mut Vec<Action>,
    ) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
        let corners = egui::CornerRadius {
            nw: 10,
            ne: 10,
            ..egui::CornerRadius::ZERO
        };
        ui.painter().rect_filled(rect, corners, theme::VIDEO_MATTE);
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
            let mut center = rect.center();
            if matches!(message, Text::DropVideoHere) {
                let icon_center = center - Vec2::new(0.0, 17.0);
                Icon::Film.paint(ui.painter(), icon_center, 30.0, theme::ON_VIDEO);
                center.y += 18.0;
            }
            ui.painter().text(
                center,
                Align2::CENTER_CENTER,
                self.preferences.language.text(message),
                egui::FontId::proportional(15.0),
                theme::ON_VIDEO,
            );
        }
        if response.double_clicked() {
            actions.push(Action::ToggleFullscreen);
        }
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) {
        let language = self.preferences.language;
        let colors = &theme::COLORS;
        let size = ctx.content_rect().size();
        let dialog = egui::Modal::new(egui::Id::new("player-settings"))
            .frame(
                egui::Frame::new()
                    .fill(colors.background)
                    .corner_radius(10)
                    .inner_margin(egui::Margin::same(14))
                    .shadow(ctx.style().visuals.window_shadow),
            )
            .backdrop_color(Color32::from_black_alpha(110))
            .show(ctx, |ui| {
                ui.set_width((size.x - 52.0).min(670.0));
                ui.set_height((size.y - 48.0).clamp(120.0, 340.0));
                let close = ui
                    .horizontal(|ui| {
                        ui.heading(language.text(Text::Settings));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add(IconButton::new(Icon::Close, language.text(Text::Close)))
                                .clicked()
                        })
                        .inner
                    })
                    .inner;
                ui.add_space(5.0);
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height())
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        settings_ui::draw(ui, &mut self.preferences, &mut self.settings_page, 1.0)
                    });
                close
            });
        self.settings_open = !dialog.inner && !dialog.should_close();
    }

    fn header(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let language = self.preferences.language;
        let text = |key| language.text(key);
        let colors = &theme::COLORS;
        let tool_size = [36.0, 34.0];
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), tool_size[1]),
            Layout::left_to_right(Align::Center),
            |ui| {
                if ui
                    .add_enabled(
                        self.media_available && !self.picker_open,
                        IconButton::new(Icon::Folder, text(Text::OpenFileShortcut))
                            .caption(text(Text::OpenFile))
                            .size([103.0, tool_size[1]]),
                    )
                    .clicked()
                {
                    actions.push(Action::Open);
                }
                if self.playback.file.is_some() {
                    ui.add_space(6.0);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(
                            IconButton::new(Icon::Settings, text(Text::Settings))
                                .size(tool_size)
                                .selected(self.settings_open),
                        )
                        .clicked()
                    {
                        self.settings_open = !self.settings_open;
                    }
                    let label = text(match self.phase {
                        VrPhase::Idle | VrPhase::Probing => Text::EnterVr,
                        VrPhase::Starting | VrPhase::Waiting => Text::CancelVr,
                        VrPhase::Playing | VrPhase::Stopping => Text::ExitVr,
                    });
                    let vr = IconButton::new(Icon::Headset, label)
                        .caption(label)
                        .fit_caption(ui, tool_size[1]);
                    if ui
                        .add_enabled(self.media_available && self.phase.can_toggle(), vr)
                        .on_hover_text("V")
                        .clicked()
                    {
                        actions.push(Action::ToggleVr);
                    }
                    if ui
                        .add_enabled(
                            self.phase == VrPhase::Idle,
                            IconButton::new(Icon::Info, text(Text::CheckHeadset)).size(tool_size),
                        )
                        .clicked()
                    {
                        actions.push(Action::ProbeVr);
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if let (Some(title), Some(path)) =
                            (self.playback.title(), &self.playback.file)
                        {
                            ui.add(
                                egui::Label::new(RichText::new(title.as_ref()).size(16.0).strong())
                                    .truncate(),
                            )
                            .on_hover_text(path.to_string_lossy());
                        }
                    });
                });
            },
        );

        let status = self.phase.label(language);
        if status.is_some() || self.runtime.is_some() {
            ui.horizontal_wrapped(|ui| {
                if let Some(status) = status {
                    ui.label(RichText::new(status).small().strong());
                }
                if let Some(runtime) = &self.runtime {
                    ui.label(
                        RichText::new(format!("{}  ·  {}", runtime.runtime, runtime.headset))
                            .small()
                            .color(colors.muted),
                    );
                }
            });
        }
        for error in [self.vr_error.as_deref(), self.playback.error.as_deref()]
            .into_iter()
            .flatten()
        {
            ui.label(error);
        }
    }

    fn transport(&mut self, ui: &mut egui::Ui, fullscreen: bool, actions: &mut Vec<Action>) {
        let playback = &self.playback;
        let colors = &theme::COLORS;
        let mut position = self.scrub.or(playback.position).unwrap_or(0.0);
        let duration = playback.duration.unwrap_or(0.0);
        let show_hours = duration.max(position) >= 3600.0;
        let time_width = if show_hours { 64.0 } else { 43.0 };
        let time = |seconds| {
            egui::Label::new(
                RichText::new(time_label(seconds, show_hours))
                    .monospace()
                    .size(12.0)
                    .color(colors.muted),
            )
        };
        ui.horizontal(|ui| {
            ui.add_sized([time_width, 22.0], time(self.scrub.or(playback.position)));
            ui.spacing_mut().slider_width =
                (ui.available_width() - time_width - ui.spacing().item_spacing.x).max(24.0);
            let seek = theme::slider(
                ui,
                playback.loaded && playback.seekable && duration > 0.0,
                egui::Slider::new(&mut position, 0.0..=duration.max(0.001)).show_value(false),
            );
            if seek.is_pointer_button_down_on() {
                self.scrub = Some(position);
            } else if self.scrub.take().is_some() || seek.changed() {
                actions.push(Action::Seek(position));
            }
            ui.add_sized([time_width, 22.0], time(playback.duration));
        });

        let scale = 1.0;
        let (side, center) = transport_sizes(ui, scale);
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), center.y),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.allocate_ui_with_layout(side, Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_width(side.x);
                    self.volume_controls(ui, actions);
                });
                ui.allocate_ui_with_layout(center, Layout::left_to_right(Align::Center), |ui| {
                    if let Some(action) = playback_buttons(
                        ui,
                        playback,
                        self.preferences.language,
                        self.preferences.pc_seek,
                        scale,
                        true,
                    ) {
                        actions.push(action);
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (icon, key) = if fullscreen {
                        (Icon::Windowed, Text::ExitFullscreen)
                    } else {
                        (Icon::Fullscreen, Text::Fullscreen)
                    };
                    if ui
                        .add(
                            IconButton::new(icon, self.preferences.language.text(key))
                                .size([36.0, 34.0]),
                        )
                        .clicked()
                    {
                        actions.push(Action::ToggleFullscreen);
                    }
                });
            },
        );
    }

    fn volume_controls(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let language = self.preferences.language;
        let (icon, key) = if self.playback.muted {
            (Icon::Muted, Text::UnmuteShortcut)
        } else {
            (Icon::Volume, Text::MuteShortcut)
        };
        if ui
            .add_enabled(
                self.media_available,
                IconButton::new(icon, language.text(key)).size([34.0, 32.0]),
            )
            .clicked()
        {
            actions.push(Action::ToggleMute);
        }
        let mut volume = self.playback.volume;
        ui.spacing_mut().slider_width = (ui.available_width() - 45.0).clamp(20.0, 88.0);
        if theme::slider(
            ui,
            self.media_available,
            egui::Slider::new(&mut volume, 0.0..=100.0)
                .integer()
                .suffix("%"),
        )
        .on_hover_text(language.text(Text::VolumeShortcut))
        .changed()
        {
            actions.push(Action::Volume(volume));
        }
    }
}

const ADJACENT_WIDTH: f32 = 32.0;
const SKIP_WIDTH: f32 = 49.0;
const PLAY_SIZE: f32 = 44.0;
const TRANSPORT_GAP: f32 = 4.0;

pub fn transport_sizes(ui: &egui::Ui, scale: f32) -> (Vec2, Vec2) {
    let controls = (2.0 * (ADJACENT_WIDTH + SKIP_WIDTH) + PLAY_SIZE + 4.0 * TRANSPORT_GAP) * scale;
    let side = (ui.available_width() - controls - 2.0 * ui.spacing().item_spacing.x).max(0.0) * 0.5;
    (
        Vec2::new(side, PLAY_SIZE * scale),
        Vec2::new(controls, PLAY_SIZE * scale),
    )
}

pub fn playback_buttons(
    ui: &mut egui::Ui,
    playback: &Playback,
    language: Language,
    interval: u8,
    scale: f32,
    shortcuts: bool,
) -> Option<Action> {
    let text = |key| language.text(key);
    let (play_icon, play_key) = if playback.paused || playback.eof || !playback.loaded {
        (
            Icon::Play,
            if shortcuts {
                Text::PlayShortcut
            } else {
                Text::Play
            },
        )
    } else {
        (
            Icon::Pause,
            if shortcuts {
                Text::PauseShortcut
            } else {
                Text::Pause
            },
        )
    };
    let seek = f64::from(interval);
    let can_seek = playback.loaded && playback.seekable;
    let mut action = None;
    ui.spacing_mut().item_spacing.x = TRANSPORT_GAP * scale;
    ui.spacing_mut().button_padding.x = 5.0 * scale;
    for (icon, command, enabled) in [
        (
            Some((Icon::Previous, Text::PreviousVideo)),
            Action::PreviousFile,
            !playback.loading && playback.nearby.previous.is_some(),
        ),
        (None, Action::SeekBy(-seek), can_seek),
        (
            Some((play_icon, play_key)),
            Action::TogglePause,
            playback.loaded,
        ),
        (None, Action::SeekBy(seek), can_seek),
        (
            Some((Icon::Next, Text::NextVideo)),
            Action::NextFile,
            !playback.loading && playback.nearby.next.is_some(),
        ),
    ] {
        let response = if let Some((icon, key)) = icon {
            let button = IconButton::new(icon, text(key));
            let button = if matches!(command, Action::TogglePause) {
                button.size(Vec2::splat(PLAY_SIZE * scale))
            } else {
                button.size(Vec2::new(ADJACENT_WIDTH, 34.0) * scale)
            };
            ui.add_enabled(enabled, button)
        } else if let Action::SeekBy(delta) = &command {
            let (sign, shortcut) = if *delta < 0.0 {
                ("−", "←")
            } else {
                ("+", "→")
            };
            let label = format!("{sign}{interval}{}", text(Text::SecondsSuffix));
            let response = ui.add_enabled(
                enabled,
                egui::Button::new(RichText::new(label).size(13.0 * scale))
                    .frame_when_inactive(false)
                    .min_size(Vec2::new(SKIP_WIDTH, 34.0) * scale),
            );
            if shortcuts {
                response.on_hover_text(shortcut)
            } else {
                response
            }
        } else {
            continue;
        };
        if response.clicked() {
            action = Some(command);
        }
    }
    action
}

pub fn configure(ctx: &egui::Context) {
    static SEGOE_UI: std::sync::OnceLock<Option<Arc<egui::FontData>>> = std::sync::OnceLock::new();
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "player".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/NotoSansKR-Player.ttf"
        ))),
    );
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "player".to_owned());
    if let Some(segoe) = SEGOE_UI.get_or_init(|| {
        let windows = PathBuf::from(std::env::var_os("WINDIR")?);
        let data = std::fs::read(windows.join("Fonts").join("segoeui.ttf")).ok()?;
        Some(Arc::new(egui::FontData::from_owned(data)))
    }) {
        fonts.font_data.insert("segoe".to_owned(), segoe.clone());
        proportional.insert(0, "segoe".to_owned());
    }
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("player".to_owned());
    ctx.set_fonts(fonts);
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_style_of(egui::Theme::Dark, theme::style());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_dialog_keeps_its_height_across_pages() {
        let id = egui::Id::new("player-settings");
        for size in [
            Vec2::new(570.0, 300.0),
            Vec2::new(570.0, 390.0),
            Vec2::new(1080.0, 720.0),
        ] {
            let ctx = egui::Context::default();
            configure(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let input = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            let mut player = PlayerUi::new(true, None, Preferences::default());
            player.settings_open = true;
            player.preferences.vr.stereo = crate::graphics::Stereo::SideBySide;
            let mut height = None;
            for page in [
                SettingsPage::General,
                SettingsPage::Playback,
                SettingsPage::Vr,
            ] {
                player.settings_page = page;
                for _ in 0..2 {
                    let _ = ctx.run(input.clone(), |ctx| player.settings_dialog(ctx));
                }
                let rect = ctx.memory(|memory| memory.area_rect(id)).unwrap();
                let expected_height = *height.get_or_insert(rect.height());
                assert!((rect.height() - expected_height).abs() <= 1.0);
                assert!(screen.contains_rect(rect));
            }
        }
    }
}
