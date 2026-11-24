use egui::{
    Color32, Pos2, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, Widget, WidgetInfo,
    WidgetType, vec2,
};

#[derive(Clone, Copy)]
pub enum Icon {
    Folder,
    Play,
    Pause,
    Previous,
    Next,
    Volume,
    Muted,
    Fullscreen,
    Windowed,
    Settings,
    Close,
    Up,
    Drive,
    List,
    Grid,
    Ascending,
    Descending,
    Minus,
    Plus,
}

pub struct IconButton<'a> {
    icon: Icon,
    label: &'a str,
    size: Vec2,
    selected: bool,
    primary: bool,
}

impl<'a> IconButton<'a> {
    pub fn new(icon: Icon, label: &'a str) -> Self {
        Self {
            icon,
            label,
            size: vec2(36.0, 30.0),
            selected: false,
            primary: false,
        }
    }

    pub fn size(mut self, size: impl Into<Vec2>) -> Self {
        self.size = size.into();
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
}

impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(self.size, Sense::click());
        response
            .widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), self.label));
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact_selectable(&response, self.selected);
            let (fill, color) = if self.primary && response.enabled() {
                let fill = if response.hovered() {
                    Color32::from_rgb(175, 217, 255)
                } else {
                    Color32::from_rgb(139, 195, 255)
                };
                (fill, Color32::from_rgb(8, 18, 31))
            } else {
                (visuals.weak_bg_fill, visuals.text_color())
            };
            let painter = ui.painter();
            painter.rect(rect, 5.0, fill, visuals.bg_stroke, StrokeKind::Inside);
            self.icon.paint(
                painter,
                rect.center(),
                rect.width().min(rect.height()) * 0.56,
                color,
                fill,
            );
        }
        response.on_hover_text(self.label)
    }
}

impl Icon {
    fn paint(
        self,
        painter: &egui::Painter,
        center: Pos2,
        size: f32,
        color: Color32,
        background: Color32,
    ) {
        let point = |x, y| center + size * vec2(x, y);
        let stroke = Stroke::new((size * 0.095).max(1.4), color);
        let line = |a: (f32, f32), b: (f32, f32)| {
            painter.line_segment([point(a.0, a.1), point(b.0, b.1)], stroke);
        };
        let path = |points: &[(f32, f32)]| {
            painter.add(Shape::line(
                points.iter().map(|&(x, y)| point(x, y)).collect(),
                stroke,
            ));
        };
        let filled = |points: &[(f32, f32)]| {
            painter.add(Shape::convex_polygon(
                points.iter().map(|&(x, y)| point(x, y)).collect(),
                color,
                Stroke::NONE,
            ));
        };
        let triangle = |offset: f32, sign: f32| {
            filled(&[
                (offset - 0.30 * sign, -0.39),
                (offset - 0.30 * sign, 0.39),
                (offset + 0.39 * sign, 0.0),
            ]);
        };
        let block = |x, y, w, h| {
            painter.rect_filled(
                egui::Rect::from_min_size(point(x, y), vec2(w, h) * size),
                size * 0.035,
                color,
            );
        };

        match self {
            Self::Folder => {
                path(&[
                    (-0.47, 0.35),
                    (-0.47, -0.34),
                    (-0.12, -0.34),
                    (0.03, -0.20),
                    (0.45, -0.20),
                ]);
                path(&[
                    (-0.47, 0.35),
                    (0.35, 0.35),
                    (0.54, -0.08),
                    (-0.30, -0.08),
                    (-0.47, 0.35),
                ]);
            }
            Self::Play => triangle(0.04, 1.0),
            Self::Pause => {
                block(-0.29, -0.39, 0.20, 0.78);
                block(0.09, -0.39, 0.20, 0.78);
            }
            Self::Previous | Self::Next => {
                let side = if matches!(self, Self::Next) {
                    1.0
                } else {
                    -1.0
                };
                triangle(-0.10 * side, side);
                line((0.43 * side, -0.38), (0.43 * side, 0.38));
            }
            Self::Volume | Self::Muted => {
                filled(&[
                    (-0.49, -0.15),
                    (-0.29, -0.15),
                    (-0.03, -0.37),
                    (-0.03, 0.37),
                    (-0.29, 0.15),
                    (-0.49, 0.15),
                ]);
                if matches!(self, Self::Muted) {
                    line((0.16, -0.15), (0.44, 0.15));
                    line((0.16, 0.15), (0.44, -0.15));
                } else {
                    path(&[
                        (0.14, -0.18),
                        (0.23, -0.10),
                        (0.27, 0.0),
                        (0.23, 0.10),
                        (0.14, 0.18),
                    ]);
                    path(&[
                        (0.27, -0.32),
                        (0.40, -0.19),
                        (0.46, 0.0),
                        (0.40, 0.19),
                        (0.27, 0.32),
                    ]);
                }
            }
            Self::Fullscreen | Self::Windowed => {
                let (outer, inner) = if matches!(self, Self::Fullscreen) {
                    (0.43, 0.14)
                } else {
                    (0.14, 0.43)
                };
                for x in [-1.0, 1.0] {
                    for y in [-1.0, 1.0] {
                        path(&[
                            (inner * x, outer * y),
                            (outer * x, outer * y),
                            (outer * x, inner * y),
                        ]);
                    }
                }
            }
            Self::Settings => {
                for tooth in 0..8 {
                    let angle = tooth as f32 * std::f32::consts::FRAC_PI_4;
                    let direction = Vec2::angled(angle);
                    painter.line_segment(
                        [
                            center + direction * size * 0.28,
                            center + direction * size * 0.47,
                        ],
                        Stroke::new(size * 0.15, color),
                    );
                }
                painter.circle_filled(center, size * 0.36, color);
                painter.circle_filled(center, size * 0.16, background);
            }
            Self::Close => {
                line((-0.32, -0.32), (0.32, 0.32));
                line((-0.32, 0.32), (0.32, -0.32));
            }
            Self::Up => {
                line((0.0, 0.40), (0.0, -0.41));
                path(&[(-0.31, -0.12), (0.0, -0.41), (0.31, -0.12)]);
            }
            Self::Drive => {
                let rect = egui::Rect::from_min_max(point(-0.46, -0.28), point(0.46, 0.28));
                painter.rect_stroke(rect, 3.0, stroke, StrokeKind::Inside);
                line((-0.38, 0.05), (0.38, 0.05));
                painter.circle_filled(point(0.23, 0.16), size * 0.038, color);
            }
            Self::List => {
                for y in [-0.33, 0.0, 0.33] {
                    painter.circle_filled(point(-0.38, y), size * 0.065, color);
                    line((-0.18, y), (0.45, y));
                }
            }
            Self::Grid => {
                for x in [-0.42, 0.08] {
                    for y in [-0.42, 0.08] {
                        block(x, y, 0.34, 0.34);
                    }
                }
            }
            Self::Ascending | Self::Descending => {
                let end = if matches!(self, Self::Ascending) {
                    -1.0
                } else {
                    1.0
                };
                line((-0.25, -0.39), (-0.25, 0.39));
                path(&[
                    (-0.48, 0.15 * end),
                    (-0.25, 0.39 * end),
                    (-0.02, 0.15 * end),
                ]);
                for (y, right) in [(-0.32, 0.48), (0.0, 0.39), (0.32, 0.30)] {
                    line((0.12, y), (right, y));
                }
            }
            Self::Minus | Self::Plus => {
                line((-0.37, 0.0), (0.37, 0.0));
                if matches!(self, Self::Plus) {
                    line((0.0, -0.37), (0.0, 0.37));
                }
            }
        }
    }
}
