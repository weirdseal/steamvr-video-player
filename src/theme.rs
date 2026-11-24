use egui::{Color32, FontId, Response, Slider, Stroke, Style, TextStyle, Ui, Visuals, vec2};

pub struct Colors {
    pub background: Color32,
    pub control: Color32,
    pub field: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub disabled: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub track: Color32,
}

pub const COLORS: Colors = Colors {
    background: Color32::from_gray(11),
    control: Color32::from_gray(33),
    field: Color32::from_gray(17),
    hover: Color32::from_gray(35),
    pressed: Color32::from_gray(47),
    text: Color32::from_gray(245),
    muted: Color32::from_gray(190),
    disabled: Color32::from_gray(121),
    accent: Color32::from_gray(226),
    on_accent: Color32::from_gray(24),
    track: Color32::from_gray(77),
};

pub const VIDEO_MATTE: Color32 = Color32::BLACK;
pub const ON_VIDEO: Color32 = Color32::from_gray(207);

pub fn style() -> Style {
    let colors = &COLORS;
    let mut style = Style {
        visuals: Visuals::dark(),
        ..Default::default()
    };
    for (role, size) in [
        (TextStyle::Body, 14.0),
        (TextStyle::Button, 14.0),
        (TextStyle::Small, 12.0),
        (TextStyle::Heading, 23.0),
    ] {
        style.text_styles.insert(role, FontId::proportional(size));
    }
    style.spacing.item_spacing = vec2(8.0, 7.0);
    style.spacing.button_padding = vec2(10.0, 4.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.slider_rail_height = 4.0;
    style.spacing.scroll.bar_width = 6.0;

    let visuals = &mut style.visuals;
    visuals.panel_fill = colors.background;
    visuals.window_fill = colors.background;
    visuals.window_corner_radius = 10.into();
    visuals.window_stroke = Stroke::NONE;
    visuals.menu_corner_radius = 8.into();
    visuals.extreme_bg_color = colors.field;
    visuals.faint_bg_color = Color32::TRANSPARENT;
    visuals.weak_text_color = Some(colors.muted);
    visuals.hyperlink_color = colors.accent;
    visuals.warn_fg_color = colors.text;
    visuals.error_fg_color = colors.text;
    visuals.text_cursor.stroke.color = colors.accent;
    visuals.selection.bg_fill = colors.pressed;
    visuals.selection.stroke = Stroke::new(1.0_f32, colors.text);
    visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, colors.text);

    for (widget, fill) in [
        (&mut visuals.widgets.inactive, colors.control),
        (&mut visuals.widgets.hovered, colors.hover),
        (&mut visuals.widgets.active, colors.pressed),
        (&mut visuals.widgets.open, colors.hover),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::NONE;
        widget.fg_stroke = Stroke::new(1.0_f32, colors.text);
        widget.corner_radius = 5.into();
        widget.expansion = 0.0;
    }
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    style
}

pub fn slider(ui: &mut Ui, enabled: bool, slider: Slider<'_>) -> Response {
    let colors = &COLORS;
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 22.0;
        let visuals = ui.visuals_mut();
        visuals.selection.bg_fill = colors.accent;
        visuals.widgets.inactive.bg_fill = colors.track;
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            widget.expansion = -1.5;
            widget.fg_stroke = Stroke::new(2.5_f32, colors.accent);
        }
        visuals.widgets.noninteractive.expansion = -1.5;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.5_f32, colors.disabled);
        ui.add_enabled(enabled, slider.trailing_fill(true))
    })
    .inner
}
