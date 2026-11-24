use egui::{Button, RichText, Ui, Vec2};

use crate::files::{SortField, SortOrder};
use crate::graphics::{Projection, Stereo};
use crate::i18n::{Language, Text};
use crate::icons::{Icon, IconButton};
use crate::settings::{BrowserView, Preferences, SEEK_INTERVAL};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum SettingsPage {
    #[default]
    General,
    Playback,
    Vr,
}

pub fn draw(ui: &mut Ui, preferences: &mut Preferences, page: &mut SettingsPage, scale: f32) {
    let language = preferences.language;
    let text = |key| language.text(key);
    ui.horizontal_wrapped(|ui| {
        choices(
            ui,
            page,
            &[
                (SettingsPage::General, text(Text::General)),
                (SettingsPage::Playback, text(Text::Playback)),
                (SettingsPage::Vr, "VR"),
            ],
            scale,
        );
    });
    ui.add_space(10.0 * scale);
    egui::Grid::new("preferences")
        .num_columns(2)
        .spacing(Vec2::new(18.0 * scale, 8.0 * scale))
        .show(ui, |ui| match *page {
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
                        ui.separator();
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
                    ui.checkbox(
                        &mut preferences.play_on_open,
                        caption(text(Text::PlayAutomatically), scale),
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
                            ui.checkbox(
                                &mut vr.half_resolution,
                                caption(text(Text::CorrectHalfStereo), scale),
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
        });
}

pub fn sort_controls(ui: &mut Ui, order: &mut SortOrder, language: Language, scale: f32) {
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
        .add(IconButton::new(icon, label).size(Vec2::splat(30.0 * scale)))
        .clicked()
    {
        order.descending = !order.descending;
    }
}

fn row(ui: &mut Ui, label: &str, scale: f32, contents: impl FnOnce(&mut Ui)) {
    ui.label(caption(label, scale));
    ui.horizontal(contents);
    ui.end_row();
}

fn caption(text: &str, scale: f32) -> RichText {
    RichText::new(text).size(15.0 * scale)
}

fn choices<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, options: &[(T, &str)], scale: f32) {
    for &(choice, label) in options {
        if ui
            .add(
                Button::new(caption(label, scale))
                    .min_size(Vec2::new(42.0 * scale, 30.0 * scale))
                    .selected(*value == choice),
            )
            .clicked()
        {
            *value = choice;
        }
    }
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
    let size = Vec2::splat(30.0 * scale);
    let decrease = ui
        .add_enabled(
            can_decrease,
            IconButton::new(Icon::Minus, language.text(Text::Decrease)).size(size),
        )
        .clicked();
    ui.add_sized(
        Vec2::new(66.0 * scale, size.y),
        egui::Label::new(caption(value, scale).monospace()),
    );
    let increase = ui
        .add_enabled(
            can_increase,
            IconButton::new(Icon::Plus, language.text(Text::Increase)).size(size),
        )
        .clicked();
    i8::from(increase) - i8::from(decrease)
}
