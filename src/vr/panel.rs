use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use egui::{Align, Layout, Pos2, Rect, RichText, UiBuilder, Vec2};
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
use crate::theme;
use crate::ui::{self, Action, time_label};

const WIDTH: f32 = 1.18;
const PIXEL_WIDTH: u32 = 1040;
const PLAYBACK_HEIGHT: u32 = 206;
const PLAYBACK_ERROR_HEIGHT: u32 = 250;
const BROWSER_HEIGHT: u32 = 640;
const SETTINGS_WIDTH: u32 = 860;
const SETTINGS_HEIGHT: u32 = 520;
const PANEL_GAP: u32 = 24;
const PLAYBACK_TOP: u32 = BROWSER_HEIGHT + PANEL_GAP;
// The compositor receives both panels at the same pose with a transparent gap.
const SIZE: [u32; 2] = [PIXEL_WIDTH, PLAYBACK_TOP + PLAYBACK_ERROR_HEIGHT];
const METERS_PER_PIXEL: f32 = WIDTH / PIXEL_WIDTH as f32;
const PANEL_Z: f32 = -1.30;
const PLAYBACK_TOP_Y: f32 = -0.123;
const OVERLAY_Y: f32 =
    PLAYBACK_TOP_Y + (PLAYBACK_TOP as f32 - SIZE[1] as f32 * 0.5) * METERS_PER_PIXEL;
const CURSOR_SIZE: u32 = 32;
const CURSOR_WIDTH: f32 = CURSOR_SIZE as f32 * METERS_PER_PIXEL;
const HOVER_REDRAW_INTERVAL: f64 = 1.0 / 45.0;
const BROWSER_DRAG_THRESHOLD: f32 = 10.0;

const BROWSER_RECT: Rect = Rect {
    min: Pos2::ZERO,
    max: Pos2::new(PIXEL_WIDTH as f32, BROWSER_HEIGHT as f32),
};
const SETTINGS_RECT: Rect = Rect {
    min: Pos2::new(
        ((PIXEL_WIDTH - SETTINGS_WIDTH) / 2) as f32,
        (BROWSER_HEIGHT - SETTINGS_HEIGHT) as f32,
    ),
    max: Pos2::new(
        ((PIXEL_WIDTH + SETTINGS_WIDTH) / 2) as f32,
        BROWSER_HEIGHT as f32,
    ),
};
const PLAYBACK_RECT: Rect = Rect {
    min: Pos2::new(0.0, PLAYBACK_TOP as f32),
    max: Pos2::new(PIXEL_WIDTH as f32, (PLAYBACK_TOP + PLAYBACK_HEIGHT) as f32),
};
const PLAYBACK_ERROR_RECT: Rect = Rect {
    min: PLAYBACK_RECT.min,
    max: Pos2::new(PIXEL_WIDTH as f32, SIZE[1] as f32),
};
const SURFACE_RECT: Rect = Rect {
    min: Pos2::ZERO,
    max: PLAYBACK_ERROR_RECT.max,
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
    fn at(aim: glam::Vec2, playback_panel: Rect, upper_panel: Option<Rect>) -> Option<Self> {
        let point = point_at(aim);
        if playback_panel.contains(point) {
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

fn preferred_hand(
    current: Option<usize>,
    aim: [Option<glam::Vec2>; 2],
    hits: [Option<PanelHit>; 2],
) -> Option<usize> {
    current
        .filter(|&hand| aim[hand].is_some())
        .or_else(|| hits.iter().position(Option::is_some))
        .or_else(|| aim.iter().position(Option::is_some))
}

fn playback_panel_rect(playback: &Playback) -> Rect {
    if playback.error.is_some() {
        PLAYBACK_ERROR_RECT
    } else {
        PLAYBACK_RECT
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
    hover_pointer: Option<Pos2>,
    hover_at: f64,
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
            style.spacing.scroll.bar_width = 12.0;
        });
        Ok(Self {
            images,
            painter,
            egui,
            repaint_at: f64::INFINITY,
            hover_pointer: None,
            hover_at: 0.0,
        })
    }

    fn hover_changed(&self, pointer: Option<Pos2>, time: f64) -> bool {
        self.hover_pointer != pointer
            && (time >= self.hover_at || self.hover_pointer.is_none() || pointer.is_none())
    }

    fn draw(
        &mut self,
        time: f64,
        pointer: Option<Pos2>,
        raster_cursor: Option<Pos2>,
        click: Option<Pos2>,
        mut contents: impl FnMut(&mut egui::Ui),
    ) -> Result<()> {
        let mut input = egui::RawInput {
            screen_rect: Some(SURFACE_RECT),
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
            if let Some(point) = raster_cursor {
                let cursor = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("controller-cursor"),
                ));
                cursor.circle_filled(point, 8.0, egui::Color32::WHITE);
                cursor.circle_stroke(point, 8.0, egui::Stroke::new(2.0_f32, egui::Color32::BLACK));
            }
        });
        self.repaint_at = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(f64::INFINITY, |output| {
                time + output.repaint_delay.as_secs_f64()
            });
        self.hover_pointer = pointer;
        self.hover_at = time + HOVER_REDRAW_INTERVAL;
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
    raster_cursor: Option<Pos2>,
    scrubbing: bool,
    browser_revision: Option<u64>,
    settings_page: Option<SettingsPage>,
    preferences: Preferences,
}

#[derive(Clone, Copy)]
pub(super) struct PanelFrame {
    pub cursor: Option<glam::Vec2>,
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
        self.pointing_hand = None;
    }

    pub fn browser_aimed(&self, hands: &[Hand; 2]) -> bool {
        if self.browser.is_none() || self.scrub.is_some() {
            return false;
        }
        if self.browser_drag.is_some() {
            return true;
        }
        let playback_panel = self
            .last_image
            .as_ref()
            .map_or(PLAYBACK_RECT, |image| playback_panel_rect(&image.playback));
        let aim = hands.map(|hand| hand.aim.and_then(aim_on_plane));
        let hits = aim.map(|point| {
            point.and_then(|point| PanelHit::at(point, playback_panel, Some(BROWSER_RECT)))
        });
        preferred_hand(self.pointing_hand, aim, hits)
            .is_some_and(|hand| matches!(hits[hand], Some(PanelHit::Upper(_))))
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
        let mut playback = video.lock().unwrap().playback.clone();
        let playback_panel = playback_panel_rect(&playback);
        let aim = hands.map(|hand| hand.aim.and_then(aim_on_plane));
        let hits = aim
            .map(|point| point.and_then(|point| PanelHit::at(point, playback_panel, upper_panel)));
        let mut pointing_hand = preferred_hand(self.pointing_hand, aim, hits);
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
                playback_panel,
                upper_panel_rect(self.browser.is_some(), self.settings_page.is_some()),
            )
        });
        let pointer = hit.map(PanelHit::point);
        // Keep the compositor cursor independent of the panel redraw rate.
        let raster_cursor = if self.cursor.is_none() {
            aimed
                .map(point_at)
                .filter(|point| SURFACE_RECT.contains(*point))
        } else {
            None
        };
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
            raster_cursor,
            scrubbing,
            browser_revision: self.browser.as_ref().map(FileBrowser::revision),
            settings_page: self.settings_page,
            preferences,
        };
        let browser_visible = display.browser_revision.is_some();
        let thumbnails_ready = browser_visible
            && preferences.browser_view == crate::settings::BrowserView::Grid
            && self.thumbnails.poll(&self.surface.egui);
        let time = now.duration_since(self.started).as_secs_f64();
        if click.is_some()
            || committed.is_some()
            || thumbnails_ready
            || self.last_image.as_ref() != Some(&display)
            || self.surface.hover_changed(pointer, time)
            || time >= self.surface.repaint_at
        {
            let mut action = committed.map(|seconds| PanelAction::Playback(Action::Seek(seconds)));
            let mut timeline = None;
            let mut browser_action = None;
            let mut settings_page = self.settings_page;
            let mut edited = preferences;
            let browser = &mut self.browser;
            self.surface
                .draw(time, pointer, raster_cursor, click, |ui| {
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
                    draw_section(ui, "playback", playback_panel, |ui| {
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
            cursor: aimed.filter(|_| self.cursor.is_some()),
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
        .rect_filled(bounds, 14, theme::COLORS.background);
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
                .size(28.0)
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
    let language = display.preferences.language;
    let colors = &theme::COLORS;
    let title = playback.title();
    let title = if playback.loading {
        language.text(Text::VideoOpening)
    } else {
        title.as_deref().unwrap_or_default()
    };
    ui.add_sized(
        [ui.available_width(), 35.0],
        egui::Label::new(RichText::new(title).size(28.0).strong())
            .halign(Align::Min)
            .truncate(),
    );

    let hours = playback
        .duration
        .unwrap_or(0.0)
        .max(playback.position.unwrap_or(0.0))
        >= 3600.0;
    let time_width = if hours { 116.0 } else { 78.0 };
    let time = |seconds| {
        egui::Label::new(
            RichText::new(time_label(seconds, hours))
                .size(21.0)
                .color(colors.muted),
        )
    };
    let timeline = ui
        .horizontal(|ui| {
            ui.add_sized([time_width, 40.0], time(playback.position));
            let width = ui.available_width() - time_width - ui.spacing().item_spacing.x;
            let (timeline, _) =
                ui.allocate_exact_size(Vec2::new(width, 40.0), egui::Sense::hover());
            ui.add_sized([time_width, 40.0], time(playback.duration));
            timeline
        })
        .inner;
    let track = timeline.shrink2(Vec2::new(0.0, 17.0));
    let fraction = playback
        .duration
        .filter(|duration| *duration > 0.0)
        .map_or(0.0, |duration| playback.position.unwrap_or(0.0) / duration);
    let x = egui::lerp(track.x_range(), fraction.clamp(0.0, 1.0) as f32);
    ui.painter().rect_filled(track, 3, colors.track);
    if playback.loaded && playback.seekable && playback.duration.is_some() {
        ui.painter()
            .rect_filled(track.with_max_x(x), 3, colors.accent);
        let center = Pos2::new(x, track.center().y);
        let radius = if display.scrubbing { 12.0 } else { 10.0 };
        ui.painter().circle_filled(center, radius, colors.accent);
        ui.painter()
            .circle_filled(center, radius - 3.0, colors.background);
    }

    let scale = 1.5;
    let (side, center) = ui::transport_sizes(ui, scale);
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), center.y),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.allocate_ui_with_layout(side, Layout::left_to_right(Align::Center), |ui| {
                ui.set_min_width(side.x);
                draw_volume(ui, playback, display.preferences, action);
            });
            ui.allocate_ui_with_layout(center, Layout::left_to_right(Align::Center), |ui| {
                if let Some(command) = ui::playback_buttons(
                    ui,
                    playback,
                    language,
                    display.preferences.vr_seek,
                    scale,
                    false,
                ) {
                    *action = Some(PanelAction::Playback(command));
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button_size = [52.0, 49.0];
                if ui
                    .add(
                        IconButton::new(Icon::Settings, language.text(Text::Settings))
                            .selected(display.settings_page.is_some())
                            .size(button_size),
                    )
                    .clicked()
                {
                    *action = Some(PanelAction::ToggleSettings);
                }
                let browser_open = display.browser_revision.is_some();
                let label = language.text(if browser_open {
                    Text::CloseFileBrowser
                } else {
                    Text::OpenFileBrowser
                });
                if ui
                    .add(
                        IconButton::new(Icon::Folder, label)
                            .selected(browser_open)
                            .size(button_size),
                    )
                    .clicked()
                {
                    *action = Some(PanelAction::ToggleBrowser);
                }
                let exit = language.text(Text::ExitVr);
                if ui
                    .add(
                        IconButton::new(Icon::Close, exit)
                            .caption(exit)
                            .size([132.0, 49.0]),
                    )
                    .clicked()
                {
                    *action = Some(PanelAction::Playback(Action::ExitVr));
                }
            });
        },
    );
    if let Some(error) = playback.error.as_deref() {
        ui.add(egui::Label::new(RichText::new(error).size(22.0).color(colors.text)).truncate());
    }
    timeline
}

fn draw_volume(
    ui: &mut egui::Ui,
    playback: &Playback,
    preferences: Preferences,
    action: &mut Option<PanelAction>,
) {
    let colors = &theme::COLORS;
    let (icon, key) = if playback.muted {
        (Icon::Muted, Text::Unmute)
    } else {
        (Icon::Volume, Text::Mute)
    };
    let size = Vec2::new(46.0, 45.0);
    let step = f64::from(preferences.volume_step);
    ui.spacing_mut().item_spacing.x = 4.0;
    if ui
        .add(IconButton::new(icon, preferences.language.text(key)).size(size))
        .clicked()
    {
        *action = Some(PanelAction::Playback(Action::ToggleMute));
    }
    if ui
        .add(IconButton::new(Icon::Minus, &format!("−{step:.0}%")).size(size))
        .clicked()
    {
        *action = Some(PanelAction::Playback(Action::VolumeBy(-step)));
    }

    let (rect, response) = ui.allocate_exact_size(Vec2::new(78.0, size.y), egui::Sense::hover());
    response.on_hover_text(preferences.language.text(Text::Volume));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{:.0}%", playback.volume),
        egui::FontId::proportional(21.0),
        colors.text,
    );

    if ui
        .add(IconButton::new(Icon::Plus, &format!("+{step:.0}%")).size(size))
        .clicked()
    {
        *action = Some(PanelAction::Playback(Action::VolumeBy(step)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_vr_button_requests_a_stop() {
        let ctx = egui::Context::default();
        ui::configure(&ctx);
        let display = PanelImage {
            playback: Playback::default(),
            raster_cursor: None,
            scrubbing: false,
            browser_revision: None,
            settings_page: None,
            preferences: Preferences::default(),
        };
        let mut action = None;
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                draw_section(ui, "playback", PLAYBACK_RECT, |ui| {
                    draw_playback(ui, &display, &mut action);
                });
            });
        };
        let mut input = egui::RawInput {
            screen_rect: Some(SURFACE_RECT),
            ..Default::default()
        };
        let output = ctx.run(input.clone(), &mut draw);
        let caption = display.preferences.language.text(Text::ExitVr);
        let button_point = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == caption => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("The VR exit caption is visible");
        assert!(PLAYBACK_RECT.contains(button_point));
        input.events.push(egui::Event::PointerMoved(button_point));
        for pressed in [true, false] {
            input.events.push(egui::Event::PointerButton {
                pos: button_point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }

        let _ = ctx.run(input, draw);
        assert!(matches!(
            action,
            Some(PanelAction::Playback(Action::ExitVr))
        ));
    }

    #[test]
    fn pointer_keeps_its_hand_across_the_panel_edge() {
        let aim = [Some(glam::Vec2::ZERO), Some(glam::Vec2::new(-0.4, -0.3))];
        let hits = |aim: [Option<glam::Vec2>; 2]| {
            aim.map(|point| {
                point.and_then(|point| PanelHit::at(point, PLAYBACK_RECT, Some(BROWSER_RECT)))
            })
        };
        let both_aimed = hits(aim);
        assert!(both_aimed.iter().all(Option::is_some));
        assert_eq!(preferred_hand(Some(1), aim, both_aimed), Some(1));

        let left_beyond_edge = [aim[0], Some(glam::Vec2::new(-0.65, -0.3))];
        let only_right_aimed = hits(left_beyond_edge);
        assert!(only_right_aimed[1].is_none());
        assert_eq!(
            preferred_hand(Some(1), left_beyond_edge, only_right_aimed),
            Some(1)
        );
        assert_eq!(
            preferred_hand(None, left_beyond_edge, only_right_aimed),
            Some(0)
        );
        let left_disconnected = [aim[0], None];
        assert_eq!(
            preferred_hand(Some(1), left_disconnected, hits(left_disconnected)),
            Some(0)
        );
    }

    #[test]
    fn aim_distinguishes_the_two_panels_and_the_gap() {
        let aim_at_y = |y| {
            glam::Vec2::new(
                0.0,
                OVERLAY_Y + (SIZE[1] as f32 * 0.5 - y) * METERS_PER_PIXEL,
            )
        };
        let playback = aim_at_y(PLAYBACK_RECT.center().y);
        let browser = aim_at_y(BROWSER_RECT.center().y);
        let gap = aim_at_y((BROWSER_RECT.bottom() + PLAYBACK_RECT.top()) * 0.5);
        assert!(
            matches!(PanelHit::at(playback, PLAYBACK_RECT, Some(BROWSER_RECT)), Some(PanelHit::Playback(point))
            if point.distance(PLAYBACK_RECT.center()) < 0.001)
        );
        assert!(
            matches!(PanelHit::at(browser, PLAYBACK_RECT, Some(BROWSER_RECT)), Some(PanelHit::Upper(point))
            if point.distance(BROWSER_RECT.center()) < 0.001)
        );
        assert_eq!(PanelHit::at(browser, PLAYBACK_RECT, None), None);
        assert_eq!(PanelHit::at(gap, PLAYBACK_RECT, Some(BROWSER_RECT)), None);

        let above_settings = aim_at_y(SETTINGS_RECT.top() * 0.5);
        assert!(matches!(
            PanelHit::at(above_settings, PLAYBACK_RECT, upper_panel_rect(true, false)),
            Some(PanelHit::Upper(_))
        ));
        assert_eq!(
            PanelHit::at(above_settings, PLAYBACK_RECT, upper_panel_rect(false, true)),
            None
        );

        let footer = aim_at_y((PLAYBACK_RECT.bottom() + PLAYBACK_ERROR_RECT.bottom()) * 0.5);
        let mut state = Playback::default();
        assert_eq!(
            PanelHit::at(footer, playback_panel_rect(&state), None),
            None
        );
        state.set_error("Example playback error".to_owned());
        assert!(matches!(
            PanelHit::at(footer, playback_panel_rect(&state), None),
            Some(PanelHit::Playback(_))
        ));
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
