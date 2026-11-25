use egui::{Align, Button, Layout, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::files::{SortField, SortOrder};
use crate::graphics::{Projection, Stereo};
use crate::i18n::{Language, Text};
use crate::icons::{Icon, IconButton};
use crate::settings::{BrowserView, Preferences, SEEK_INTERVAL};
use crate::theme;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum SettingsPage {
    #[default]
    General,
    Playback,
    Vr,
}

pub fn draw(ui: &mut Ui, preferences: &mut Preferences, page: &mut SettingsPage, scale: f32) {
    let language = preferences.language;
    let colors = &theme::COLORS;
    ui.horizontal_top(|ui| {
        let nav_size = Vec2::new(110.0, 34.0) * scale;
        ui.allocate_ui_with_layout(nav_size, Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 3.0 * scale;
            for (tab, icon, label) in [
                (
                    SettingsPage::General,
                    Icon::Settings,
                    language.text(Text::General),
                ),
                (
                    SettingsPage::Playback,
                    Icon::Film,
                    language.text(Text::Playback),
                ),
                (SettingsPage::Vr, Icon::Headset, "VR"),
            ] {
                let active = *page == tab;
                let response = ui.add(IconButton::new(icon, label).caption(label).size(nav_size));
                if response.clicked() {
                    *page = tab;
                }
                if active {
                    let marker = egui::Rect::from_center_size(
                        response.rect.left_center() + Vec2::new(3.0 * scale, 0.0),
                        Vec2::new(3.0, 16.0) * scale,
                    );
                    ui.painter().rect_filled(marker, 2, colors.accent);
                }
            }
        });
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 5.0 * scale;
            draw_page(ui, preferences, *page, scale);
        });
    });
}

fn draw_page(ui: &mut Ui, preferences: &mut Preferences, page: SettingsPage, scale: f32) {
    let language = preferences.language;
    let text = |key| language.text(key);
    match page {
        SettingsPage::General => {
            row(ui, text(Text::Language), scale, |ui| {
                choices(
                    ui,
                    &mut preferences.language,
                    &[
                        (Language::Korean, text(Text::LanguageKorean)),
                        (Language::English, text(Text::LanguageEnglish)),
                    ],
                    scale,
                );
            });
            row(ui, text(Text::FileOrder), scale, |ui| {
                sort_controls(ui, &mut preferences.sort, language, scale);
            });
            row(ui, text(Text::BrowserView), scale, |ui| {
                choices(
                    ui,
                    &mut preferences.browser_view,
                    &[
                        (BrowserView::List, text(Text::List)),
                        (BrowserView::Grid, text(Text::Grid)),
                    ],
                    scale,
                );
            });
        }
        SettingsPage::Playback => {
            let unit = text(Text::SecondsSuffix);
            let presets = [5, 10, 15, 30].map(|seconds| (seconds, format!("{seconds}{unit}")));
            let presets = presets
                .each_ref()
                .map(|(seconds, label)| (*seconds, label.as_str()));
            for (label, interval) in [
                (Text::PcSeekStep, &mut preferences.pc_seek),
                (Text::VrSeekStep, &mut preferences.vr_seek),
            ] {
                row(ui, text(label), scale, |ui| {
                    integer(ui, interval, SEEK_INTERVAL, 1, unit, language, scale);
                    choices(ui, interval, &presets, scale);
                });
            }
            row(ui, text(Text::StartupVolume), scale, |ui| {
                integer(
                    ui,
                    &mut preferences.initial_volume,
                    0..=100,
                    5,
                    "%",
                    language,
                    scale,
                );
            });
            row(ui, text(Text::VolumeStep), scale, |ui| {
                integer(
                    ui,
                    &mut preferences.volume_step,
                    1..=25,
                    1,
                    "%",
                    language,
                    scale,
                );
            });
            row(ui, text(Text::OnFileOpen), scale, |ui| {
                toggle(
                    ui,
                    &mut preferences.play_on_open,
                    text(Text::PlayAutomatically),
                    scale,
                );
            });
        }
        SettingsPage::Vr => {
            let vr = &mut preferences.vr;
            row(ui, text(Text::Projection), scale, |ui| {
                choices(
                    ui,
                    &mut vr.projection,
                    &[
                        (Projection::Flat, text(Text::Flat)),
                        (Projection::Hemisphere, "180°"),
                        (Projection::Sphere, "360°"),
                    ],
                    scale,
                );
            });
            row(ui, text(Text::StereoFormat), scale, |ui| {
                choices(
                    ui,
                    &mut vr.stereo,
                    &[
                        (Stereo::Mono, text(Text::Mono)),
                        (Stereo::SideBySide, "SBS"),
                        (Stereo::TopBottom, "Top/Bottom"),
                    ],
                    scale,
                );
            });
            if vr.projection == Projection::Flat {
                row(ui, text(Text::ScreenWidth), scale, |ui| {
                    meters(ui, &mut vr.screen_width, language, scale);
                });
                row(ui, text(Text::ScreenDistance), scale, |ui| {
                    meters(ui, &mut vr.screen_distance, language, scale);
                });
                if vr.stereo != Stereo::Mono {
                    row(ui, text(Text::AspectRatio), scale, |ui| {
                        toggle(
                            ui,
                            &mut vr.half_resolution,
                            text(Text::CorrectHalfStereo),
                            scale,
                        );
                    });
                }
            }
            row(ui, text(Text::PanelTimeout), scale, |ui| {
                integer(
                    ui,
                    &mut preferences.panel_timeout,
                    3..=30,
                    1,
                    text(Text::SecondsSuffix),
                    language,
                    scale,
                );
            });
        }
    }
}

pub fn sort_controls(ui: &mut Ui, order: &mut SortOrder, language: Language, scale: f32) {
    ui.horizontal(|ui| {
        choices(
            ui,
            &mut order.field,
            &[
                (SortField::Name, language.text(Text::SortName)),
                (SortField::Modified, language.text(Text::SortModified)),
                (SortField::Size, language.text(Text::SortSize)),
            ],
            scale,
        );
        let (icon, label) = if order.descending {
            (Icon::Descending, language.text(Text::SortDescending))
        } else {
            (Icon::Ascending, language.text(Text::SortAscending))
        };
        if ui
            .add(IconButton::new(icon, label).size(Vec2::splat(28.0 * scale)))
            .clicked()
        {
            order.descending = !order.descending;
        }
    });
}

fn row(ui: &mut Ui, label: &str, scale: f32, contents: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(
            (9.0 * scale) as i8,
            (3.0 * scale) as i8,
        ))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(122.0, 29.0) * scale,
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_min_width(122.0 * scale);
                        ui.add(egui::Label::new(caption(label, scale)).truncate());
                    },
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0 * scale;
                    contents(ui);
                });
            });
        });
}

fn caption(text: &str, scale: f32) -> RichText {
    RichText::new(text).size(14.0 * scale)
}

fn choices<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, options: &[(T, &str)], scale: f32) {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 28.0 * scale),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0 * scale;
            let mut selected_rect = None;
            for &(choice, label) in options {
                let response = ui
                    .add(Button::new(caption(label, scale)).min_size(Vec2::new(38.0, 27.0) * scale))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.clicked() {
                    *value = choice;
                }
                if *value == choice {
                    selected_rect = Some(response.rect);
                }
            }
            if let Some(rect) = selected_rect {
                theme::selection_mark(ui.painter(), rect);
            }
        },
    );
}

fn toggle(ui: &mut Ui, value: &mut bool, label: &str, scale: f32) {
    let colors = &theme::COLORS;
    let switch_width = 36.0 * scale;
    let gap = 7.0 * scale;
    let galley = egui::WidgetText::from(caption(label, scale)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (ui.available_width() - switch_width - gap).max(0.0),
        egui::TextStyle::Body,
    );
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(switch_width + gap + galley.size().x, 29.0 * scale),
        Sense::click(),
    );
    let mut response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *value, label)
    });

    let height = 18.0 * scale;
    let switch = egui::Rect::from_center_size(
        rect.left_center() + Vec2::new(switch_width * 0.5, 0.0),
        Vec2::new(switch_width, height),
    );
    let fill = if *value {
        colors.accent
    } else if response.hovered() {
        colors.hover
    } else {
        colors.control
    };
    let focus = if response.has_focus() {
        Stroke::new(1.0_f32, colors.accent)
    } else {
        Stroke::NONE
    };
    let x = if *value {
        switch.right() - height * 0.5
    } else {
        switch.left() + height * 0.5
    };
    ui.painter()
        .rect(switch, height * 0.5, fill, focus, StrokeKind::Inside);
    ui.painter().circle_filled(
        egui::pos2(x, switch.center().y),
        5.5 * scale,
        if *value {
            colors.on_accent
        } else {
            colors.muted
        },
    );
    ui.painter().galley(
        egui::pos2(
            switch.right() + gap,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        colors.text,
    );
}

fn integer(
    ui: &mut Ui,
    value: &mut u8,
    range: std::ops::RangeInclusive<u8>,
    step: u8,
    unit: &str,
    language: Language,
    scale: f32,
) {
    let delta = stepper(
        ui,
        &format!("{}{unit}", *value),
        *value > *range.start(),
        *value < *range.end(),
        language,
        scale,
    );
    if delta < 0 {
        *value = value
            .saturating_sub(step)
            .clamp(*range.start(), *range.end());
    } else if delta > 0 {
        *value = value
            .saturating_add(step)
            .clamp(*range.start(), *range.end());
    }
}

fn meters(ui: &mut Ui, value: &mut f32, language: Language, scale: f32) {
    let delta = stepper(
        ui,
        &format!("{:.2} m", *value),
        *value > 0.5,
        *value < 15.0,
        language,
        scale,
    );
    *value = (*value + f32::from(delta) * 0.25).clamp(0.5, 15.0);
}

fn stepper(
    ui: &mut Ui,
    value: &str,
    can_decrease: bool,
    can_increase: bool,
    language: Language,
    scale: f32,
) -> i8 {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 28.0 * scale),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 3.0 * scale;
            let size = Vec2::splat(28.0 * scale);
            let decrease = ui
                .add_enabled(
                    can_decrease,
                    IconButton::new(Icon::Minus, language.text(Text::Decrease)).size(size),
                )
                .clicked();
            ui.add_sized(
                Vec2::new(62.0 * scale, size.y),
                egui::Label::new(caption(value, scale)),
            );
            let increase = ui
                .add_enabled(
                    can_increase,
                    IconButton::new(Icon::Plus, language.text(Text::Increase)).size(size),
                )
                .clicked();
            i8::from(increase) - i8::from(decrease)
        },
    )
    .inner
}
