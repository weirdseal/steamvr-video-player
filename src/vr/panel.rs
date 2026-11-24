use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use egui::{Align, Color32, Layout, Pos2, Rect, RichText, UiBuilder, Vec2};
use glow::HasContext as _;
use openxr as xr;
use winit::event_loop::EventLoopProxy;

use super::browser::{BrowserAction, FileBrowser};
use super::input::Hand;
use super::thumbnails::Thumbnails;
use super::{GlSwapchain, Message, orientation, position};
use crate::AppEvent;
use crate::graphics::{VideoSource, paint_vr_cursor};
use crate::i18n::{Text, tr};
use crate::icons::{Icon, IconButton};
use crate::mpv::Playback;
use crate::settings::Preferences;
use crate::settings_ui::{self, SettingsPage};
use crate::ui::{self, Action, time_label};

const WIDTH: f32 = 1.18;
const PIXEL_WIDTH: u32 = 1040;
const PLAYBACK_HEIGHT: u32 = 400;
const BROWSER_HEIGHT: u32 = 640;
const SETTINGS_HEIGHT: u32 = 520;
const PANEL_GAP: u32 = 24;
const PLAYBACK_TOP: u32 = BROWSER_HEIGHT + PANEL_GAP;
// The compositor receives both panels at the same pose with a transparent gap.
const SIZE: [u32; 2] = [PIXEL_WIDTH, PLAYBACK_TOP + PLAYBACK_HEIGHT];
const METERS_PER_PIXEL: f32 = WIDTH / PIXEL_WIDTH as f32;
const PANEL_Z: f32 = -1.30;
const PLAYBACK_Y: f32 = -0.35;
const OVERLAY_Y: f32 = PLAYBACK_Y + PLAYBACK_TOP as f32 * METERS_PER_PIXEL * 0.5;
const CURSOR_SIZE: u32 = 32;
const CURSOR_WIDTH: f32 = CURSOR_SIZE as f32 * METERS_PER_PIXEL;
const BROWSER_DRAG_THRESHOLD: f32 = 10.0;
const ACCENT: Color32 = Color32::from_rgb(143, 202, 255);

const BROWSER_RECT: Rect = Rect {
    min: Pos2::ZERO,
    max: Pos2::new(PIXEL_WIDTH as f32, BROWSER_HEIGHT as f32),
};
const SETTINGS_RECT: Rect = Rect {
    min: Pos2::new(0.0, (BROWSER_HEIGHT - SETTINGS_HEIGHT) as f32),
    max: BROWSER_RECT.max,
};
const PLAYBACK_RECT: Rect = Rect {
    min: Pos2::new(0.0, PLAYBACK_TOP as f32),
    max: Pos2::new(PIXEL_WIDTH as f32, SIZE[1] as f32),
};

fn point_at(aim: glam::Vec2) -> Pos2 {
    Pos2::new(
        SIZE[0] as f32 * 0.5 + aim.x / METERS_PER_PIXEL,
        SIZE[1] as f32 * 0.5 + (OVERLAY_Y - aim.y) / METERS_PER_PIXEL,
    )
}

fn aim_on_plane(pose: xr::Posef) -> Option<glam::Vec2> {
    let origin = position(pose);
    let direction = orientation(pose) * glam::Vec3::NEG_Z;
    if direction.z >= -0.001 {
        return None;
    }
    let distance = (PANEL_Z - origin.z) / direction.z;
    if distance <= 0.0 {
        return None;
    }
    let hit = origin + distance * direction;
    let point = glam::Vec2::new(hit.x, hit.y);
    point.is_finite().then_some(point)
}

fn timeline_position(rect: Rect, point: Pos2, duration: f64) -> f64 {
    let fraction = (point.x - rect.left()) / rect.width();
    duration * f64::from(fraction.clamp(0.0, 1.0))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PanelHit {
    Playback(Pos2),
    Upper(Pos2),
}

impl PanelHit {
    fn at(aim: glam::Vec2, upper_panel: Option<Rect>) -> Option<Self> {
        let point = point_at(aim);
        if PLAYBACK_RECT.contains(point) {
            return Some(Self::Playback(point));
        }
        upper_panel
            .filter(|rect| rect.contains(point))
            .map(|_| Self::Upper(point))
    }

    fn point(self) -> Pos2 {
        match self {
            Self::Playback(point) | Self::Upper(point) => point,
        }
    }
}

fn upper_panel_rect(browser_open: bool, settings_open: bool) -> Option<Rect> {
    if browser_open {
        Some(BROWSER_RECT)
    } else {
        settings_open.then_some(SETTINGS_RECT)
    }
}

struct PanelSurface<'a> {
    images: GlSwapchain<'a>,
    painter: egui_glow::Painter,
    egui: egui::Context,
    repaint_at: f64,
}

impl<'a> PanelSurface<'a> {
    fn new(
        gl: &'a Arc<glow::Context>,
        session: &xr::Session<xr::OpenGL>,
        format: u32,
    ) -> Result<Self> {
        let images = GlSwapchain::new(gl, session, SIZE, format)?;
        let painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .map_err(|error| anyhow::anyhow!("{} ({error})", tr(Text::PanelRenderingInitFailed)))?;
        let egui = egui::Context::default();
        ui::configure(&egui);
        egui.style_mut(|style| {
            style.animation_time = 0.0;
            style.spacing.item_spacing = Vec2::new(12.0, 10.0);
            style.spacing.scroll.floating = false;
        });
        Ok(Self {
            images,
            painter,
            egui,
            repaint_at: f64::INFINITY,
        })
    }

    fn draw(
        &mut self,
        time: f64,
        pointer: Option<Pos2>,
        click: Option<Pos2>,
        mut contents: impl FnMut(&mut egui::Ui),
    ) -> Result<()> {
        let mut input = egui::RawInput {
            screen_rect: Some(PLAYBACK_RECT.union(BROWSER_RECT)),
            time: Some(time),
            max_texture_side: Some(self.painter.max_texture_side()),
            ..Default::default()
        };
        input.events.push(match pointer {
            Some(point) => egui::Event::PointerMoved(point),
            None => egui::Event::PointerGone,
        });
        if let Some(pos) = click {
            for pressed in [true, false] {
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        let output = self.egui.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, &mut contents);
            if let Some(point) = pointer {
                let cursor = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("controller-cursor"),
                ));
                cursor.circle_filled(point, 8.0, Color32::from_rgb(150, 224, 255));
                cursor.circle_stroke(
                    point,
                    8.0,
                    egui::Stroke::new(2.0_f32, Color32::from_rgb(14, 21, 34)),
                );
            }
        });
        self.repaint_at = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(f64::INFINITY, |output| {
                time + output.repaint_delay.as_secs_f64()
            });
        let fbo = self.images.acquire()?;
        let gl = self.painter.gl();
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::FRAMEBUFFER_SRGB);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        let meshes = self.egui.tessellate(output.shapes, output.pixels_per_point);
        self.painter.paint_and_update_textures(
            SIZE,
            output.pixels_per_point,
            &meshes,
            &output.textures_delta,
        );
        unsafe {
            self.painter.gl().bind_framebuffer(glow::FRAMEBUFFER, None);
            self.painter.gl().flush();
        }
        self.images
            .swapchain
            .release_image()
            .context(tr(Text::PanelImageReleaseFailed))
    }

    fn layer<'b>(&'b self, view: &'b xr::Space) -> xr::CompositionLayerQuad<'b, xr::OpenGL> {
        quad(
            &self.images,
            view,
            0.0,
            OVERLAY_Y,
            WIDTH,
            SIZE[1] as f32 * METERS_PER_PIXEL,
        )
        .layer_flags(xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA)
    }
}

impl Drop for PanelSurface<'_> {
    fn drop(&mut self) {
        unsafe { self.painter.gl().finish() };
        self.painter.destroy();
    }
}

fn quad<'a>(
    images: &'a GlSwapchain<'_>,
    view: &'a xr::Space,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> xr::CompositionLayerQuad<'a, xr::OpenGL> {
    xr::CompositionLayerQuad::new()
        .space(view)
        .eye_visibility(xr::EyeVisibility::BOTH)
        .pose(xr::Posef {
            position: xr::Vector3f { x, y, z: PANEL_Z },
            ..xr::Posef::IDENTITY
        })
        .size(xr::Extent2Df { width, height })
        .sub_image(
            xr::SwapchainSubImage::new()
                .swapchain(&images.swapchain)
                .image_rect(images.rect),
        )
}

#[derive(Clone, Copy)]
struct Scrub {
    hand: usize,
    seconds: f64,
    generation: u64,
}

struct BrowserDrag {
    hand: usize,
    start: Pos2,
    last_y: f32,
    moved: bool,
}

impl BrowserDrag {
    fn new(hand: usize, start: Pos2) -> Self {
        Self {
            hand,
            start,
            last_y: start.y,
            moved: false,
        }
    }

    fn move_to(&mut self, point: Pos2) -> f32 {
        if !self.moved {
            let motion = point - self.start;
            let distance = motion.length();
            if distance <= BROWSER_DRAG_THRESHOLD {
                return 0.0;
            }
            self.last_y += motion.y * (BROWSER_DRAG_THRESHOLD / distance);
            self.moved = true;
        }
        let scroll = self.last_y - point.y;
        self.last_y = point.y;
        scroll
    }

    fn tap(&self, released_at: Pos2, entries: Rect) -> Option<Pos2> {
        (!self.moved && entries.contains(released_at)).then_some(self.start)
    }
}

enum PanelAction {
    Playback(Action),
    ToggleBrowser,
    ToggleSettings,
}

#[derive(PartialEq)]
struct PanelImage {
    playback: Playback,
    pointer: Option<Pos2>,
    scrubbing: bool,
    browser_revision: Option<u64>,
    settings_page: Option<SettingsPage>,
    preferences: Preferences,
}

#[derive(Clone, Copy)]
pub(super) struct PanelFrame {
    pub outside_pointer: Option<glam::Vec2>,
    pub browser_visible: bool,
}

pub(super) struct PlaybackPanel<'a> {
    surface: PanelSurface<'a>,
    browser: Option<FileBrowser>,
    thumbnails: Thumbnails,
    settings_page: Option<SettingsPage>,
    cursor: Option<GlSwapchain<'a>>,
    started: Instant,
    last_interaction: Option<Instant>,
    last_image: Option<PanelImage>,
    timeline: Option<Rect>,
    scrub: Option<Scrub>,
    browser_drag: Option<BrowserDrag>,
    pointing_hand: Option<usize>,
}

impl<'a> PlaybackPanel<'a> {
    pub fn new(
        gl: &'a Arc<glow::Context>,
        session: &xr::Session<xr::OpenGL>,
        format: u32,
        cursor_supported: bool,
    ) -> Result<Self> {
        let surface = PanelSurface::new(gl, session, format)?;
        let cursor = if cursor_supported {
            match create_cursor(gl, session, format) {
                Ok(images) => Some(images),
                Err(error) => {
                    log::warn!("Could not initialize the VR pointer: {error:#}");
                    None
                }
            }
        } else {
            None
        };
        Ok(Self {
            surface,
            browser: None,
            thumbnails: Thumbnails::default(),
            settings_page: None,
            cursor,
            started: Instant::now(),
            last_interaction: None,
            last_image: None,
            timeline: None,
            scrub: None,
            browser_drag: None,
            pointing_hand: None,
        })
    }

    pub fn visible(&self, now: Instant, timeout: u8) -> bool {
        self.last_image.is_none()
            || self.browser.is_some()
            || self.settings_page.is_some()
            || self.scrub.is_some()
            || self.last_interaction.is_some_and(|last| {
                now.duration_since(last) < Duration::from_secs(u64::from(timeout))
            })
    }

    pub fn cancel_gestures(&mut self) {
        self.scrub = None;
        self.browser_drag = None;
    }

    pub fn browser_aimed(&self, hands: &[Hand; 2]) -> bool {
        self.browser.is_some()
            && self.scrub.is_none()
            && (self.browser_drag.is_some()
                || hands
                    .iter()
                    .filter_map(|hand| hand.aim.and_then(aim_on_plane))
                    .find_map(|point| PanelHit::at(point, Some(BROWSER_RECT)))
                    .is_some_and(|hit| matches!(hit, PanelHit::Upper(_))))
    }

    pub fn update(
        &mut self,
        hands: [Hand; 2],
        video: &VideoSource,
        now: Instant,
        scroll: f32,
        preferences: Preferences,
        proxy: &EventLoopProxy<AppEvent>,
    ) -> Result<Option<PanelFrame>> {
        let visible = self.visible(now, preferences.panel_timeout);
        if !visible && !hands.iter().any(|hand| hand.pressed) {
            return Ok(None);
        }

        let upper_panel = upper_panel_rect(self.browser.is_some(), self.settings_page.is_some());
        let entries_rect = self.browser.as_mut().and_then(|browser| {
            browser.preferences(preferences);
            browser.entries_rect()
        });
        let aim = hands.map(|hand| hand.aim.and_then(aim_on_plane));
        let hits = aim.map(|point| point.and_then(|point| PanelHit::at(point, upper_panel)));
        let mut pointing_hand = hits
            .iter()
            .position(Option::is_some)
            .or(self.pointing_hand.filter(|&hand| aim[hand].is_some()))
            .or_else(|| aim.iter().position(Option::is_some));
        let mut playback = video.lock().unwrap().playback.clone();
        if self
            .scrub
            .is_some_and(|scrub| scrub.generation != playback.generation())
        {
            self.scrub = None;
        }
        let seek_range = self.timeline.zip(
            playback
                .duration
                .filter(|duration| playback.loaded && playback.seekable && *duration > 0.0),
        );
        let mut click = None;
        let mut committed = None;
        let was_dragging_browser = self.browser_drag.is_some();

        if let Some(drag) = self.scrub.take() {
            let hand = hands[drag.hand];
            pointing_hand = Some(drag.hand);
            // The controller that grabbed the timeline owns the gesture until release.
            if hand.active
                && let Some(point) = aim[drag.hand]
                && let Some((timeline, duration)) = seek_range
            {
                let seconds = timeline_position(timeline, point_at(point), duration);
                if hand.down {
                    self.scrub = Some(Scrub { seconds, ..drag });
                } else {
                    committed = Some(seconds);
                }
            }
        } else if let Some(mut drag) = self.browser_drag.take() {
            let hand = hands[drag.hand];
            pointing_hand = Some(drag.hand);
            if hand.active
                && let Some(point) = aim[drag.hand].map(point_at)
                && let Some(entries) = entries_rect
                && let Some(browser) = &mut self.browser
            {
                browser.scroll_by(drag.move_to(point));
                if hand.down {
                    self.browser_drag = Some(drag);
                } else {
                    click = drag.tap(point, entries);
                }
            }
        } else if !visible {
            pointing_hand = hands.iter().position(|hand| hand.pressed);
        } else if let Some((hand, hit)) = hands
            .iter()
            .zip(hits)
            .enumerate()
            .find_map(|(index, (hand, hit))| hit.filter(|_| hand.pressed).map(|hit| (index, hit)))
        {
            pointing_hand = Some(hand);
            if let PanelHit::Playback(point) = hit
                && let Some((timeline, duration)) = seek_range
                && timeline.contains(point)
            {
                self.scrub = Some(Scrub {
                    hand,
                    seconds: timeline_position(timeline, point, duration),
                    generation: playback.generation(),
                });
            } else if let PanelHit::Upper(point) = hit
                && entries_rect.is_some_and(|entries| entries.contains(point))
            {
                self.browser_drag = Some(BrowserDrag::new(hand, point));
            } else {
                click = Some(hit.point());
            }
        } else if let Some(hand) = hands
            .iter()
            .position(|hand| hand.pressed && hand.aim.is_some())
        {
            if upper_panel.is_some() {
                self.browser = None;
                self.settings_page = None;
                pointing_hand = Some(hand);
            } else {
                self.last_interaction = None;
                return Ok(None);
            }
        }

        self.pointing_hand = pointing_hand.or(self.pointing_hand);
        let aimed = pointing_hand.and_then(|hand| aim[hand]);
        let hit = aimed.and_then(|point| {
            PanelHit::at(
                point,
                upper_panel_rect(self.browser.is_some(), self.settings_page.is_some()),
            )
        });
        let pointer = hit
            .map(PanelHit::point)
            .map(|point| Pos2::new(point.x.floor(), point.y.floor()));
        let scrubbing = self.scrub.is_some();
        if self.last_image.is_none()
            || !visible
            || hit.is_some()
            || upper_panel.is_some()
            || scrubbing
            || committed.is_some()
        {
            self.last_interaction = Some(now);
        }
        playback.position = self
            .scrub
            .map(|drag| drag.seconds)
            .or(committed)
            .or(playback.position.map(f64::floor));
        playback.volume = playback.volume.round();
        if let Some(browser) = &mut self.browser {
            browser.poll();
            if scroll != 0.0 && !was_dragging_browser && self.browser_drag.is_none() {
                browser.scroll_by(scroll);
            }
        }
        let display = PanelImage {
            playback,
            pointer,
            scrubbing,
            browser_revision: self.browser.as_ref().map(FileBrowser::revision),
            settings_page: self.settings_page,
            preferences,
        };
        let browser_visible = display.browser_revision.is_some();
        let thumbnails_ready = browser_visible
            && preferences.browser_view == crate::settings::BrowserView::Grid
            && self.thumbnails.poll(&self.surface.egui);
        let displayed_upper_panel =
            upper_panel_rect(browser_visible, display.settings_page.is_some());
        let time = now.duration_since(self.started).as_secs_f64();
        if click.is_some()
            || committed.is_some()
            || thumbnails_ready
            || self.last_image.as_ref() != Some(&display)
            || time >= self.surface.repaint_at
        {
            let mut action = committed.map(|seconds| PanelAction::Playback(Action::Seek(seconds)));
            let mut timeline = None;
            let mut browser_action = None;
            let mut settings_page = self.settings_page;
            let mut edited = preferences;
            let browser = &mut self.browser;
            self.surface.draw(time, pointer, click, |ui| {
                if let Some(browser) = browser {
                    draw_section(ui, "browser", BROWSER_RECT, |ui| {
                        browser_action =
                            browser.draw(ui, preferences.language, &mut self.thumbnails);
                    });
                } else if let Some(page) = &mut settings_page {
                    let mut close = false;
                    draw_section(ui, "settings", SETTINGS_RECT, |ui| {
                        close = draw_settings(ui, &mut edited, page);
                    });
                    if close {
                        settings_page = None;
                    }
                }
                draw_section(ui, "playback", PLAYBACK_RECT, |ui| {
                    timeline = Some(draw_playback(ui, &display, &mut action));
                });
            })?;
            self.timeline = timeline;
            self.settings_page = settings_page;
            if let Some(action) = action {
                match action {
                    PanelAction::ToggleBrowser => {
                        self.settings_page = None;
                        self.browser = if browser_visible {
                            None
                        } else {
                            Some(FileBrowser::new(
                                display.playback.file.as_deref(),
                                preferences,
                            ))
                        };
                    }
                    PanelAction::ToggleSettings => {
                        self.browser = None;
                        self.settings_page =
                            self.settings_page.is_none().then(SettingsPage::default);
                    }
                    PanelAction::Playback(action) => {
                        let _ = proxy.send_event(AppEvent::Vr(Message::Playback(action)));
                    }
                }
            }
            if let (Some(browser), Some(action)) = (&mut self.browser, browser_action) {
                match action {
                    BrowserAction::Browse(location) => browser.browse(location),
                    BrowserAction::View(view) => {
                        edited.browser_view = view;
                    }
                    BrowserAction::Sort(sort) => edited.sort = sort,
                    BrowserAction::Close => self.browser = None,
                    BrowserAction::Open(path) => {
                        self.browser = None;
                        let _ = proxy
                            .send_event(AppEvent::Vr(Message::Playback(Action::OpenFile(path))));
                    }
                }
            }
            if edited != preferences {
                let action = Action::Preferences {
                    before: preferences,
                    edited,
                };
                let _ = proxy.send_event(AppEvent::Vr(Message::Playback(action)));
            }
            self.last_image = Some(display);
        }
        Ok(Some(PanelFrame {
            outside_pointer: aimed
                .filter(|&point| PanelHit::at(point, displayed_upper_panel).is_none()),
            browser_visible,
        }))
    }

    pub fn layer<'b>(&'b self, view: &'b xr::Space) -> xr::CompositionLayerQuad<'b, xr::OpenGL> {
        self.surface.layer(view)
    }

    pub fn cursor_layer<'b>(
        &'b self,
        view: &'b xr::Space,
        point: glam::Vec2,
    ) -> Option<xr::CompositionLayerQuad<'b, xr::OpenGL>> {
        let images = self.cursor.as_ref()?;
        Some(
            quad(images, view, point.x, point.y, CURSOR_WIDTH, CURSOR_WIDTH)
                .layer_flags(xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA),
        )
    }

    pub fn disable_cursor(&mut self) {
        self.cursor = None;
    }
}

fn create_cursor<'a>(
    gl: &'a glow::Context,
    session: &xr::Session<xr::OpenGL>,
    format: u32,
) -> Result<GlSwapchain<'a>> {
    let mut cursor = GlSwapchain::with_flags(
        gl,
        session,
        [CURSOR_SIZE; 2],
        format,
        xr::SwapchainCreateFlags::STATIC_IMAGE,
    )?;
    let fbo = cursor.acquire()?;
    let painted = paint_vr_cursor(gl, fbo, CURSOR_SIZE as i32);
    let released = cursor.swapchain.release_image();
    painted?;
    released.context(tr(Text::PointerImageReleaseFailed))?;
    Ok(cursor)
}

fn draw_section(ui: &mut egui::Ui, id: &str, bounds: Rect, contents: impl FnOnce(&mut egui::Ui)) {
    ui.painter()
        .rect_filled(bounds, 0.0, Color32::from_rgb(23, 30, 42));
    ui.scope_builder(
        UiBuilder::new()
            .id_salt(id)
            .max_rect(bounds.shrink2(Vec2::new(26.0, 18.0)))
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_clip_rect(bounds);
            contents(ui);
        },
    );
}

fn draw_settings(
    ui: &mut egui::Ui,
    preferences: &mut Preferences,
    page: &mut SettingsPage,
) -> bool {
    let language = preferences.language;
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(language.text(Text::Settings))
                .size(29.0)
                .strong(),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            close = ui
                .add(IconButton::new(Icon::Close, language.text(Text::Close)).size([52.0, 44.0]))
                .clicked();
        });
    });
    settings_ui::draw(ui, preferences, page, 1.5);
    close
}

fn draw_playback(
    ui: &mut egui::Ui,
    display: &PanelImage,
    action: &mut Option<PanelAction>,
) -> Rect {
    let playback = &display.playback;
    let scrubbing = display.scrubbing;
    let browser_open = display.browser_revision.is_some();
    let text = |key| display.preferences.language.text(key);
    ui.horizontal(|ui| {
        let button_size = [58.0, 48.0];
        let title_width =
            ui.available_width() - 2.0 * (button_size[0] + ui.spacing().item_spacing.x);
        ui.allocate_ui_with_layout(
            Vec2::new(title_width, 48.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_min_width(title_width);
                if let Some(title) = playback.title() {
                    ui.add(
                        egui::Label::new(RichText::new(title.as_ref()).size(33.0).strong())
                            .truncate(),
                    );
                }
            },
        );
        let label = if browser_open {
            text(Text::CloseFileBrowser)
        } else {
            text(Text::OpenFileBrowser)
        };
        if ui
            .add(
                IconButton::new(Icon::Folder, label)
                    .size(button_size)
                    .selected(browser_open),
            )
            .clicked()
        {
            *action = Some(PanelAction::ToggleBrowser);
        }
        if ui
            .add(
                IconButton::new(Icon::Settings, text(Text::Settings))
                    .size(button_size)
                    .selected(display.settings_page.is_some()),
            )
            .clicked()
        {
            *action = Some(PanelAction::ToggleSettings);
        }
    });
    let hours = playback
        .duration
        .unwrap_or(0.0)
        .max(playback.position.unwrap_or(0.0))
        >= 3600.0;
    let time = if playback.loading {
        text(Text::VideoOpening).to_owned()
    } else {
        format!(
            "{}  /  {}",
            time_label(playback.position, hours),
            time_label(playback.duration, hours),
        )
    };
    ui.label(RichText::new(time).size(30.0).strong());

    let (timeline, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), egui::Sense::hover());
    let fraction = playback
        .duration
        .filter(|duration| *duration > 0.0)
        .map_or(0.0, |duration| playback.position.unwrap_or(0.0) / duration);
    let track = timeline.shrink2(Vec2::new(0.0, 13.0));
    let cursor_x = egui::lerp(track.x_range(), fraction.clamp(0.0, 1.0) as f32);
    let seekable = playback.loaded && playback.seekable;
    let can_scrub = seekable && playback.duration.is_some();
    let fill = if can_scrub {
        ACCENT
    } else {
        Color32::from_rgb(78, 91, 113)
    };
    ui.painter()
        .rect_filled(track, 5.0, Color32::from_rgb(48, 58, 75));
    ui.painter()
        .rect_filled(track.with_max_x(cursor_x), 5.0, fill);
    if can_scrub {
        ui.painter().circle_filled(
            Pos2::new(cursor_x, track.center().y),
            if scrubbing { 14.0 } else { 11.0 },
            ACCENT,
        );
    }

    ui.horizontal(|ui| {
        let (play_icon, play_label) = if playback.paused || playback.eof || !playback.loaded {
            (Icon::Play, text(Text::Play))
        } else {
            (Icon::Pause, text(Text::Pause))
        };
        let width = (ui.available_width() - 4.0 * ui.spacing().item_spacing.x) / 5.0;
        let size = [width, 60.0];
        let amount = display.preferences.vr_seek;
        for (icon, label, enabled, command) in [
            (
                Some(Icon::Previous),
                text(Text::PreviousVideo).to_owned(),
                !playback.loading && playback.nearby.previous.is_some(),
                Action::PreviousFile,
            ),
            (
                None,
                format!("−{amount}{}", text(Text::SecondsSuffix)),
                seekable,
                Action::SeekBy(-f64::from(amount)),
            ),
            (
                Some(play_icon),
                play_label.to_owned(),
                playback.loaded,
                Action::TogglePause,
            ),
            (
                None,
                format!("+{amount}{}", text(Text::SecondsSuffix)),
                seekable,
                Action::SeekBy(f64::from(amount)),
            ),
            (
                Some(Icon::Next),
                text(Text::NextVideo).to_owned(),
                !playback.loading && playback.nearby.next.is_some(),
                Action::NextFile,
            ),
        ] {
            let response = if let Some(icon) = icon {
                let button = IconButton::new(icon, &label).size(size);
                let button = if matches!(command, Action::TogglePause) {
                    button.primary()
                } else {
                    button
                };
                ui.add_enabled(enabled, button)
            } else {
                ui.add_enabled(
                    enabled,
                    egui::Button::new(RichText::new(label).size(29.0)).min_size(size.into()),
                )
            };
            if response.clicked() {
                *action = Some(PanelAction::Playback(command));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{}  {:3.0}%", text(Text::Volume), playback.volume))
                .size(29.0)
                .strong(),
        );
        let amount = display.preferences.volume_step;
        for (prefix, sign) in [("−", -1.0), ("+", 1.0)] {
            if ui
                .add_sized(
                    [154.0, 53.0],
                    egui::Button::new(RichText::new(format!("{prefix}{amount}%")).size(27.0)),
                )
                .clicked()
            {
                *action = Some(PanelAction::Playback(Action::VolumeBy(
                    f64::from(amount) * sign,
                )));
            }
        }
        let (icon, label) = if playback.muted {
            (Icon::Muted, text(Text::Unmute))
        } else {
            (Icon::Volume, text(Text::Mute))
        };
        if ui
            .add(IconButton::new(icon, label).size([86.0, 53.0]))
            .clicked()
        {
            *action = Some(PanelAction::Playback(Action::ToggleMute));
        }
    });
    if let Some(error) = playback.error.as_deref() {
        ui.add(
            egui::Label::new(
                RichText::new(error)
                    .size(23.0)
                    .color(Color32::from_rgb(255, 173, 157)),
            )
            .truncate(),
        );
    }
    timeline
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aim_distinguishes_the_two_panels_and_the_gap() {
        let aim_at_y = |y| {
            glam::Vec2::new(
                0.0,
                OVERLAY_Y + (SIZE[1] as f32 * 0.5 - y) * METERS_PER_PIXEL,
            )
        };
        let playback = glam::Vec2::new(0.0, PLAYBACK_Y);
        let browser = aim_at_y(BROWSER_RECT.center().y);
        let gap = aim_at_y((BROWSER_RECT.bottom() + PLAYBACK_RECT.top()) * 0.5);
        assert!(
            matches!(PanelHit::at(playback, Some(BROWSER_RECT)), Some(PanelHit::Playback(point))
            if point.distance(PLAYBACK_RECT.center()) < 0.001)
        );
        assert!(
            matches!(PanelHit::at(browser, Some(BROWSER_RECT)), Some(PanelHit::Upper(point))
            if point.distance(BROWSER_RECT.center()) < 0.001)
        );
        assert_eq!(PanelHit::at(browser, None), None);
        assert_eq!(PanelHit::at(gap, Some(BROWSER_RECT)), None);

        let above_settings = aim_at_y(SETTINGS_RECT.top() * 0.5);
        assert!(matches!(
            PanelHit::at(above_settings, upper_panel_rect(true, false)),
            Some(PanelHit::Upper(_))
        ));
        assert_eq!(
            PanelHit::at(above_settings, upper_panel_rect(false, true)),
            None
        );
    }

    #[test]
    fn browser_tap_selects_where_the_press_started() {
        let entries = Rect::from_min_max(Pos2::new(100.0, 100.0), Pos2::new(900.0, 430.0));
        let start = Pos2::new(103.0, 200.0);
        let mut drag = BrowserDrag::new(0, start);

        let inside = Pos2::new(105.0, 197.0);
        assert_eq!(drag.move_to(inside), 0.0);
        assert_eq!(drag.tap(inside, entries), Some(start));

        let outside = Pos2::new(99.0, 200.0);
        assert_eq!(drag.move_to(outside), 0.0);
        assert_eq!(drag.tap(outside, entries), None);
    }

    #[test]
    fn browser_drag_scrolls_both_ways_without_selecting_an_entry() {
        let entries = BROWSER_RECT;
        let start = Pos2::new(200.0, 200.0);
        let mut drag = BrowserDrag::new(1, start);

        assert_eq!(drag.move_to(Pos2::new(200.0, 170.0)), 20.0);
        assert_eq!(drag.move_to(Pos2::new(200.0, 150.0)), 20.0);
        assert_eq!(drag.move_to(start), -50.0);
        assert_eq!(drag.tap(start, entries), None);
    }
}
