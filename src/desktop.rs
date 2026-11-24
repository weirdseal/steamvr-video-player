use std::ffi::CString;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use egui_glow::winit::EguiGlow;
use glow::HasContext as _;
use glutin::config::{Config, ConfigSurfaceTypes, ConfigTemplateBuilder};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, GlProfile, PossiblyCurrentContext, Version,
};
use glutin::display::{Display, DisplayApiPreference};
use glutin::prelude::*;
use glutin::surface::{
    PbufferSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Fullscreen, Theme, Window, WindowId};

use crate::files::{self, VIDEO_EXTENSIONS};
use crate::graphics::{Triangle, Video, check_opengl};
use crate::i18n::{Text, tr};
use crate::mpv::Mpv;
use crate::settings::Preferences;
use crate::ui::{self, Action, PlayerUi, VrPhase};
use crate::vr::{self, Worker};
use crate::{APP_NAME, AppEvent};

pub fn run(open: Option<PathBuf>, preferences: Preferences) -> Result<()> {
    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let mut app = App {
        open,
        proxy: event_loop.create_proxy(),
        desktop: None,
        startup_error: None,
        preferences,
    };
    event_loop.run_app(&mut app)?;
    app.startup_error.map_or(Ok(()), Err)
}

struct App {
    open: Option<PathBuf>,
    proxy: EventLoopProxy<AppEvent>,
    desktop: Option<Desktop>,
    startup_error: Option<anyhow::Error>,
    preferences: Preferences,
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        if self.desktop.is_some() {
            return;
        }
        match Desktop::new(events, self.proxy.clone(), self.preferences) {
            Ok(mut desktop) => {
                if let Some(path) = self.open.take() {
                    desktop.open(path);
                }
                desktop.window.set_visible(true);
                desktop.window.request_redraw();
                self.desktop = Some(desktop);
            }
            Err(error) => {
                self.startup_error = Some(error);
                events.exit();
            }
        }
    }

    fn window_event(&mut self, events: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(desktop) = &mut self.desktop
            && id == desktop.window.id()
        {
            desktop.window_event(events, event);
        }
    }

    fn user_event(&mut self, events: &ActiveEventLoop, event: AppEvent) {
        let Some(desktop) = &mut self.desktop else {
            return;
        };
        match event {
            AppEvent::Media => desktop.media_events(),
            AppEvent::Video => {
                if desktop.mpv.as_ref().is_some_and(Mpv::update_video) {
                    desktop.update_video();
                }
            }
            AppEvent::Repaint(at) => {
                desktop.repaint = Some(desktop.repaint.map_or(at, |pending| pending.min(at)));
                return;
            }
            AppEvent::FilePicked(path) => {
                desktop.ui.picker_open = false;
                if !desktop.closing
                    && let Some(path) = path
                {
                    desktop.open(path);
                }
            }
            AppEvent::NearbyVideos(request, nearby) => {
                if desktop.nearby_request == request {
                    desktop.ui.playback.nearby = nearby;
                    desktop.video.playback(&desktop.ui.playback);
                }
            }
            AppEvent::Vr(message) => {
                desktop.vr_message(message);
                if desktop.closing && desktop.vr.is_none() {
                    events.exit();
                }
            }
        }
        desktop.window.request_redraw();
    }

    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        let Some(desktop) = &mut self.desktop else {
            return;
        };
        match desktop.repaint {
            Some(at) if at > Instant::now() => events.set_control_flow(ControlFlow::WaitUntil(at)),
            Some(_) => {
                desktop.repaint = None;
                desktop.window.request_redraw();
                events.set_control_flow(ControlFlow::Wait);
            }
            None => events.set_control_flow(ControlFlow::Wait),
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.desktop.take();
    }
}

struct Desktop {
    mpv: Option<Mpv>,
    video: Video,
    egui: EguiGlow,
    preview: Triangle,
    ui: PlayerUi,
    nearby_request: u64,
    proxy: EventLoopProxy<AppEvent>,
    vr: Option<Worker>,
    repaint: Option<Instant>,
    modifiers: ModifiersState,
    closing: bool,
    gl: Arc<glow::Context>,
    display: Display,
    config: Config,
    context: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    window: Window,
}

impl Desktop {
    fn new(
        events: &ActiveEventLoop,
        proxy: EventLoopProxy<AppEvent>,
        preferences: Preferences,
    ) -> Result<Self> {
        let window = events.create_window(
            Window::default_attributes()
                .with_title(APP_NAME)
                .with_inner_size(LogicalSize::new(1080, 720))
                .with_min_inner_size(LogicalSize::new(570, 390))
                .with_theme(Some(Theme::Dark))
                .with_visible(false),
        )?;
        let handle = window.window_handle()?.as_raw();
        let display = unsafe {
            Display::new(
                window.display_handle()?.as_raw(),
                DisplayApiPreference::Wgl(Some(handle)),
            )
        }
        .context(tr(Text::WglDisplayInitFailed))?;
        let config = select_config(&display, handle)?;
        let create_context = |version| {
            let attrs = context_attributes(version).build(Some(handle));
            unsafe { display.create_context(&config, &attrs) }
        };
        // SteamVR commonly requires OpenGL 4.3. Matching the desktop and VR versions
        // avoids driver restrictions when sharing contexts.
        let context = create_context(Version::new(4, 3))
            .or_else(|_| create_context(Version::new(3, 3)))
            .context(tr(Text::DesktopContextCreateFailed))?;
        let size = window.inner_size();
        let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            handle,
            nonzero(size.width),
            nonzero(size.height),
        );
        let surface = unsafe { display.create_window_surface(&config, &attrs) }
            .context(tr(Text::DesktopSurfaceCreateFailed))?;
        let context = context
            .make_current(&surface)
            .context(tr(Text::DesktopContextActivateFailed))?;
        if let Err(error) = surface.set_swap_interval(&context, SwapInterval::DontWait) {
            log::debug!("WGL swap interval: {error}");
        }
        let gl = Arc::new(unsafe {
            glow::Context::from_loader_function(|name| {
                display.get_proc_address(&CString::new(name).unwrap())
            })
        });
        check_opengl(&gl)?;
        log::info!("OpenGL {:?} / {}", gl.version(), unsafe {
            gl.get_parameter_string(glow::RENDERER)
        });
        let preview = Triangle::desktop(&gl)?;
        let egui = EguiGlow::new(
            events,
            gl.clone(),
            None,
            Some(window.scale_factor() as f32),
            false,
        );
        ui::configure(&egui.egui_ctx);
        let repaint_proxy = proxy.clone();
        egui.egui_ctx.set_request_repaint_callback(move |request| {
            if let Some(at) = Instant::now().checked_add(request.delay) {
                let _ = repaint_proxy.send_event(AppEvent::Repaint(at));
            }
        });

        let (mpv, issue) =
            match Mpv::new(display.clone(), proxy.clone(), preferences.initial_volume) {
                Ok(mpv) => (Some(mpv), None),
                Err(error) => {
                    log::error!("{error:#}");
                    (
                        None,
                        Some(format!("{} ({error:#})", tr(Text::MediaEngineInitFailed))),
                    )
                }
            };
        let ui = PlayerUi::new(mpv.is_some(), issue, preferences);
        let video = Video::new(gl.clone());
        video.preferences(preferences);
        Ok(Self {
            mpv,
            video,
            egui,
            preview,
            ui,
            nearby_request: 0,
            proxy,
            vr: None,
            repaint: None,
            modifiers: ModifiersState::empty(),
            closing: false,
            gl,
            display,
            config,
            context,
            surface,
            window,
        })
    }

    fn window_event(&mut self, events: &ActiveEventLoop, event: WindowEvent) {
        let response = self.egui.on_window_event(&self.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                self.closing = true;
                if let Some(worker) = &self.vr {
                    self.ui.phase = VrPhase::Stopping;
                    worker.stop();
                    self.window.request_redraw();
                } else {
                    events.exit();
                }
            }
            WindowEvent::Resized(PhysicalSize { width, height }) if width != 0 && height != 0 => {
                self.surface
                    .resize(&self.context, nonzero(width), nonzero(height));
                self.window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::DroppedFile(path) if !self.closing => self.open(path),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } if event.state.is_pressed() && !self.closing => {
                if let PhysicalKey::Code(key) = event.physical_key
                    && let Some(action) = self.shortcut(key, !event.repeat, response.consumed)
                {
                    self.action(action);
                    self.window.request_redraw();
                }
            }
            _ => {}
        }
        if response.repaint {
            self.window.request_redraw();
        }
    }

    fn shortcut(&self, key: KeyCode, first_press: bool, consumed: bool) -> Option<Action> {
        if self.modifiers.control_key() && key == KeyCode::KeyO && first_press {
            return Some(Action::Open);
        }
        if key == KeyCode::F11 && first_press {
            return Some(Action::ToggleFullscreen);
        }
        if consumed
            || self
                .modifiers
                .intersects(ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SUPER)
        {
            return None;
        }
        let seek = f64::from(self.ui.preferences.pc_seek);
        let volume = f64::from(self.ui.preferences.volume_step);
        match key {
            KeyCode::Space if first_press => Some(Action::TogglePause),
            KeyCode::ArrowLeft => Some(Action::SeekBy(-seek)),
            KeyCode::ArrowRight => Some(Action::SeekBy(seek)),
            KeyCode::ArrowUp => Some(Action::VolumeBy(volume)),
            KeyCode::ArrowDown => Some(Action::VolumeBy(-volume)),
            KeyCode::KeyM if first_press => Some(Action::ToggleMute),
            KeyCode::KeyV if first_press => Some(Action::ToggleVr),
            KeyCode::KeyF if first_press => Some(Action::ToggleFullscreen),
            KeyCode::Escape if first_press && self.window.fullscreen().is_some() => {
                Some(Action::ToggleFullscreen)
            }
            _ => None,
        }
    }

    fn redraw(&mut self) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 || self.window.is_minimized().unwrap_or(false) {
            return;
        }
        let previous_settings = self.ui.preferences;
        let frame = self.video.frame();
        let fullscreen = self.window.fullscreen().is_some();
        let mut actions = Vec::new();
        self.egui.run(&self.window, |ctx| {
            actions.extend(self.ui.draw(ctx, frame, self.preview, fullscreen));
        });
        self.preferences_changed(previous_settings);
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            self.gl.disable(glow::SCISSOR_TEST);
            self.gl.disable(glow::FRAMEBUFFER_SRGB);
            self.gl.clear_color(0.0, 0.0, 0.0, 1.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
        self.egui.paint(&self.window);
        self.window.pre_present_notify();
        if let Err(error) = self.surface.swap_buffers(&self.context) {
            log::warn!("PC OpenGL swap: {error}");
        }
        if !self.closing {
            for action in actions {
                self.action(action);
            }
        }
    }

    fn open(&mut self, path: PathBuf) {
        let Some(mpv) = &self.mpv else { return };
        let result = (|| -> Result<PathBuf> {
            let path = std::path::absolute(path)?;
            ensure!(
                path.is_file(),
                "{} ({})",
                tr(Text::VideoFileNotFound),
                path.display()
            );
            mpv.open(&path)?;
            Ok(path)
        })();
        match result {
            Ok(path) => {
                let path: Arc<Path> = Arc::from(path);
                self.video.clear();
                self.ui.reset_scrub();
                self.ui.playback.begin_load();
                self.ui.playback.file = Some(path);
                self.ui.playback.error = None;
                self.find_nearby();
                let paused = if self.ui.preferences.play_on_open {
                    "no"
                } else {
                    "yes"
                };
                self.command(&["set", "pause", paused]);
            }
            Err(error) => self.media_error(format!("{} ({error:#})", tr(Text::VideoOpenFailed))),
        }
        self.window.request_redraw();
    }

    fn find_nearby(&mut self) {
        self.nearby_request = self.nearby_request.wrapping_add(1);
        self.ui.playback.nearby = Default::default();
        self.video.playback(&self.ui.playback);
        let Some(file) = self.ui.playback.file.clone() else {
            return;
        };
        let request = self.nearby_request;
        let sort = self.ui.preferences.sort;
        let proxy = self.proxy.clone();
        if let Err(error) = thread::Builder::new()
            .name("nearby-videos".to_owned())
            .spawn(move || match files::nearby(&file, sort) {
                Ok(nearby) => {
                    let _ = proxy.send_event(AppEvent::NearbyVideos(request, nearby));
                }
                Err(error) => log::debug!("Could not list neighboring videos: {error}"),
            })
        {
            log::warn!("Could not start neighboring video scan: {error}");
        }
    }

    fn preferences_changed(&mut self, previous: Preferences) {
        let preferences = self.ui.preferences;
        if previous == preferences {
            return;
        }
        preferences.language.set_current();
        self.video.preferences(preferences);
        if previous.sort != preferences.sort {
            self.find_nearby();
        }
        if let Err(error) = preferences.save() {
            self.media_error(format!("{} ({error:#})", tr(Text::SettingsSaveFailed)));
        }
        self.window.request_redraw();
    }

    fn choose_file(&mut self) {
        if self.ui.picker_open || self.mpv.is_none() {
            return;
        }
        let dialog = rfd::FileDialog::new()
            .set_parent(&self.window)
            .set_title(tr(Text::OpenVideoDialogTitle))
            .add_filter(tr(Text::VideoFilesFilter), VIDEO_EXTENSIONS)
            .add_filter(tr(Text::AllFilesFilter), &["*"]);
        let proxy = self.proxy.clone();
        let spawn = thread::Builder::new()
            .name("file-dialog".to_owned())
            .spawn(move || {
                let _ = proxy.send_event(AppEvent::FilePicked(dialog.pick_file()));
            });
        match spawn {
            Ok(_) => self.ui.picker_open = true,
            Err(error) => self.media_error(format!("{} ({error})", tr(Text::FilePickerFailed))),
        }
        self.window.request_redraw();
    }

    fn command(&mut self, args: &[&str]) {
        if let Some(mpv) = &self.mpv
            && let Err(error) = mpv.command(args)
        {
            self.media_error(format!("{} ({error:#})", tr(Text::PlaybackCommandFailed)));
        }
    }

    fn media_error(&mut self, error: String) {
        self.ui.playback.set_error(error);
        self.video.playback(&self.ui.playback);
    }

    fn media_events(&mut self) {
        let Some(mpv) = &self.mpv else { return };
        let before = self.ui.playback.dimensions();
        let was_paused = self.ui.playback.paused;
        let change = mpv.poll(&mut self.ui.playback);
        if was_paused != self.ui.playback.paused {
            log::info!(
                "Playback paused={} position={:?}",
                self.ui.playback.paused,
                self.ui.playback.position
            );
        }
        let after = self.ui.playback.dimensions();
        if change.clear_frame || (before.is_some() && after.is_none()) {
            self.ui.reset_scrub();
            self.video.clear();
        }
        if let Some(issue) = change.issue {
            self.ui.playback.set_error(issue);
        }
        self.video.playback(&self.ui.playback);
        if after.is_some() && (before != after || change.clear_frame) {
            self.update_video();
        }
    }

    fn update_video(&mut self) {
        let Some(mpv) = &self.mpv else { return };
        let result = if let Some(dimensions) = self.ui.playback.dimensions() {
            self.video.render(mpv, dimensions)
        } else {
            mpv.render(None)
        };
        if let Err(error) = result {
            self.video.clear();
            self.media_error(format!("{} ({error:#})", tr(Text::VideoDisplayFailed)));
        }
    }

    fn action(&mut self, action: Action) {
        let playback = &self.ui.playback;
        match action {
            Action::Preferences { before, edited } => {
                let previous = self.ui.preferences;
                self.ui.preferences.apply_edit(before, edited);
                self.preferences_changed(previous);
            }
            Action::Open => self.choose_file(),
            Action::OpenFile(path) => self.open(path),
            Action::PreviousFile | Action::NextFile if !playback.loading => {
                let file = if matches!(action, Action::PreviousFile) {
                    &playback.nearby.previous
                } else {
                    &playback.nearby.next
                };
                if let Some(file) = file {
                    self.open(file.to_path_buf());
                }
            }
            Action::TogglePause if playback.loaded => {
                if playback.eof && playback.seekable {
                    self.command(&["seek", "0", "absolute+exact"]);
                    self.command(&["set", "pause", "no"]);
                } else {
                    self.command(&["cycle", "pause"]);
                }
            }
            Action::Seek(position) if playback.loaded && playback.seekable => {
                self.command(&["seek", &position.to_string(), "absolute+exact"]);
            }
            Action::SeekBy(delta) if playback.loaded && playback.seekable => {
                self.command(&["seek", &delta.to_string(), "relative+exact"]);
            }
            Action::Volume(volume) => self.command(&["set", "volume", &volume.to_string()]),
            Action::VolumeBy(delta) => self.command(&["add", "volume", &delta.to_string()]),
            Action::ToggleMute => self.command(&["cycle", "mute"]),
            Action::ToggleFullscreen => {
                let fullscreen = self
                    .window
                    .fullscreen()
                    .is_none()
                    .then(|| Fullscreen::Borderless(self.window.current_monitor()));
                self.window.set_fullscreen(fullscreen);
            }
            Action::ProbeVr if self.vr.is_none() => self.probe_vr(),
            Action::ToggleVr if self.mpv.is_some() && self.ui.phase.can_toggle() => {
                self.toggle_vr()
            }
            Action::ExitVr => self.stop_vr(),
            _ => {}
        }
    }

    fn probe_vr(&mut self) {
        self.ui.vr_error = None;
        self.ui.runtime = None;
        match Worker::probe(self.proxy.clone()) {
            Ok(worker) => {
                self.vr = Some(worker);
                self.ui.phase = VrPhase::Probing;
            }
            Err(error) => {
                self.ui.vr_error = Some(format!("{} ({error:#})", tr(Text::OpenXrProbeStartFailed)))
            }
        }
        self.window.request_redraw();
    }

    fn toggle_vr(&mut self) {
        if self.vr.is_some() {
            self.stop_vr();
            return;
        }
        self.ui.vr_error = None;
        self.ui.runtime = None;
        let start = self
            .video
            .start_sharing(&self.ui.playback)
            .and_then(|source| Worker::start(source, self.proxy.clone()));
        match start {
            Ok(worker) => {
                self.vr = Some(worker);
                self.ui.phase = VrPhase::Starting;
                log::info!(
                    "VR start requested: position={:?}, paused={}",
                    self.ui.playback.position,
                    self.ui.playback.paused
                );
            }
            Err(error) => {
                self.video.stop_sharing();
                self.ui.vr_error = Some(format!("{} ({error:#})", tr(Text::VrStartFailed)));
            }
        }
        self.window.request_redraw();
    }

    fn stop_vr(&mut self) {
        if let Some(worker) = &self.vr {
            worker.stop();
            self.ui.phase = VrPhase::Stopping;
            self.window.request_redraw();
        }
    }

    fn shared_context(&self, requirements: openxr::opengl::Requirements) -> Result<vr::WglContext> {
        let min = requirements
            .min_api_version_supported
            .max(openxr::Version::new(3, 3, 0));
        let max = requirements.max_api_version_supported;
        ensure!(min <= max, tr(Text::RuntimeGlVersionUnsupported));
        let desktop = self.gl.version();
        let requested =
            openxr::Version::new(desktop.major as u16, desktop.minor as u16, 0).clamp(min, max);
        let attrs = SurfaceAttributesBuilder::<PbufferSurface>::new().build(nonzero(1), nonzero(1));
        let surface = unsafe { self.display.create_pbuffer_surface(&self.config, &attrs) }
            .context(tr(Text::VrSurfaceCreateFailed))?;
        let attrs = context_attributes(Version::new(
            requested.major() as u8,
            requested.minor() as u8,
        ))
        .with_sharing(&self.context)
        .build(None);
        let context = unsafe { self.display.create_context(&self.config, &attrs) }
            .with_context(|| format!("{} ({requested})", tr(Text::VrSharedContextCreateFailed)))?;
        Ok(vr::WglContext {
            context,
            surface,
            display: self.display.clone(),
        })
    }

    fn vr_message(&mut self, message: vr::Message) {
        match message {
            vr::Message::Probed(result) => {
                if let Some(worker) = self.vr.take() {
                    worker.join();
                }
                self.ui.phase = VrPhase::Idle;
                match result {
                    Ok(info) => self.ui.runtime = Some(info),
                    Err(error) => self.ui.vr_error = Some(error),
                }
            }
            vr::Message::PrepareGraphics(requirements, reply) => {
                if !self.closing && self.ui.phase == VrPhase::Starting {
                    let _ = reply.send(self.shared_context(requirements));
                }
            }
            vr::Message::Session(info) => {
                self.ui.runtime = Some(info);
                if self.ui.phase == VrPhase::Starting {
                    self.ui.phase = VrPhase::Waiting;
                }
            }
            vr::Message::Presenting if self.ui.phase == VrPhase::Waiting => {
                self.ui.phase = VrPhase::Playing
            }
            vr::Message::Presenting => {}
            vr::Message::Playback(action) => {
                if !self.closing && self.ui.phase == VrPhase::Playing {
                    log::info!("OpenXR playback action: {action:?}");
                    self.action(action);
                }
            }
            vr::Message::ControlsUnavailable(error) => self.ui.vr_error = Some(error),
            vr::Message::Finished(result) => {
                if let Some(worker) = self.vr.take() {
                    worker.join();
                }
                self.video.stop_sharing();
                if let Err(error) = result {
                    self.ui.runtime = None;
                    self.ui.vr_error = Some(error);
                }
                self.ui.phase = VrPhase::Idle;
                log::info!(
                    "VR ended: position={:?}, paused={}",
                    self.ui.playback.position,
                    self.ui.playback.paused
                );
            }
        }
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        if let Some(worker) = self.vr.take() {
            worker.stop();
            worker.join();
        }
        self.mpv.take();
        self.egui.destroy();
        self.preview.delete(&self.gl);
        // Video textures are dropped while the desktop context is still current.
    }
}

fn select_config(display: &Display, handle: RawWindowHandle) -> Result<Config> {
    let query = |surfaces| -> Result<Option<Config>> {
        let template = ConfigTemplateBuilder::new()
            .compatible_with_native_window(handle)
            .with_surface_type(surfaces)
            .prefer_hardware_accelerated(Some(true))
            .build();
        Ok(unsafe { display.find_configs(template) }?.next())
    };
    if let Ok(Some(config)) = query(ConfigSurfaceTypes::WINDOW | ConfigSurfaceTypes::PBUFFER) {
        return Ok(config);
    }
    query(ConfigSurfaceTypes::WINDOW)?.context(tr(Text::WglFormatUnavailable))
}

fn context_attributes(version: Version) -> ContextAttributesBuilder {
    ContextAttributesBuilder::new()
        .with_profile(GlProfile::Core)
        .with_context_api(ContextApi::OpenGl(Some(version)))
}

fn nonzero(size: u32) -> NonZeroU32 {
    NonZeroU32::new(size.max(1)).unwrap()
}
