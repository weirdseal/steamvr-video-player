use std::ffi::CString;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use glam::{Quat, Vec3};
use glow::HasContext as _;
use glutin::context::NotCurrentContext;
use glutin::display::{Display, GlDisplay};
use glutin::prelude::*;
use glutin::surface::{PbufferSurface, Surface};
use openxr as xr;
use windows_sys::Win32::Graphics::OpenGL::{wglGetCurrentContext, wglGetCurrentDC};
use winit::event_loop::EventLoopProxy;

mod browser;
mod input;
mod panel;
mod thumbnails;

use crate::graphics::{EyeView, Projection, VideoSource, VrProgram, check_opengl};
use crate::i18n::{Text, tr};
use crate::ui::Action;
use crate::{APP_NAME, AppEvent, executable_dir};

const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;
const TURN_RATE: f32 = 85.0_f32.to_radians();
const PITCH_LIMIT: f32 = 85.0_f32.to_radians();
const FILE_SCROLL_SPEED: f32 = 1000.0;

#[derive(Clone)]
pub struct RuntimeInfo {
    pub runtime: String,
    pub headset: String,
}

pub enum Message {
    Probed(Result<RuntimeInfo, String>),
    PrepareGraphics(
        xr::opengl::Requirements,
        mpsc::SyncSender<Result<WglContext>>,
    ),
    Session(RuntimeInfo),
    Presenting,
    Playback(Action),
    ControlsUnavailable(String),
    Finished(Result<(), String>),
}

pub struct WglContext {
    pub context: NotCurrentContext,
    pub surface: Surface<PbufferSurface>,
    pub display: Display,
}

pub struct Worker {
    cancel: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Worker {
    pub fn probe(proxy: EventLoopProxy<AppEvent>) -> Result<Self> {
        Self::spawn(proxy, |_, _| {
            Message::Probed(
                Runtime::connect()
                    .map(|runtime| runtime.info)
                    .map_err(describe),
            )
        })
    }

    pub fn start(video: VideoSource, proxy: EventLoopProxy<AppEvent>) -> Result<Self> {
        Self::spawn(proxy, move |cancel, proxy| {
            Message::Finished(run(&video, cancel, proxy).map_err(describe))
        })
    }

    fn spawn(
        proxy: EventLoopProxy<AppEvent>,
        work: impl FnOnce(&AtomicBool, &EventLoopProxy<AppEvent>) -> Message + Send + 'static,
    ) -> Result<Self> {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let thread = thread::Builder::new()
            .name("openxr".to_owned())
            .spawn(move || {
                let message = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(&worker_cancel, &proxy)
                }))
                .unwrap_or_else(|_| Message::Finished(Err(tr(Text::OpenXrThreadEnded).to_owned())));
                let _ = proxy.send_event(AppEvent::Vr(message));
            })?;
        Ok(Self { cancel, thread })
    }

    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub fn join(self) {
        let _ = self.thread.join();
    }
}

fn describe(error: anyhow::Error) -> String {
    log::warn!("OpenXR: {error:#}");
    format!("{error:#}")
}

struct Runtime {
    instance: xr::Instance,
    system: xr::SystemId,
    info: RuntimeInfo,
    graphics: xr::opengl::Requirements,
    max_layers: u32,
}

impl Runtime {
    fn connect() -> Result<Self> {
        let path = executable_dir()?.join("openxr_loader.dll");
        let entry = unsafe { xr::Entry::load_from(&path, &()) }
            .context(tr(Text::OpenXrLoaderUnavailable))?;
        let hint = tr(Text::OpenXrRuntimeUnavailable);
        let available = entry.enumerate_extensions().context(hint)?;
        let mut extensions = xr::ExtensionSet::default();
        extensions.khr_opengl_enable = available.khr_opengl_enable;
        extensions.bd_controller_interaction = available.bd_controller_interaction;
        extensions.khr_generic_controller = available.khr_generic_controller;
        let instance = entry
            .create_instance(
                &xr::ApplicationInfo {
                    application_name: APP_NAME,
                    application_version: 1,
                    ..Default::default()
                },
                &extensions,
                &[],
                &(),
            )
            .context(hint)?;
        let runtime = instance
            .properties()
            .context(tr(Text::OpenXrRuntimeInfoFailed))?
            .runtime_name;
        ensure!(
            available.khr_opengl_enable,
            "{} ({runtime})",
            tr(Text::OpenXrOpenGlUnavailable)
        );
        let system = instance
            .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
            .with_context(|| format!("{} ({runtime})", tr(Text::HeadsetUnavailable)))?;
        let properties = instance
            .system_properties(system)
            .context(tr(Text::OpenXrHeadsetInfoFailed))?;
        let headset = properties.system_name;
        let graphics = instance
            .graphics_requirements::<xr::OpenGL>(system)
            .context(tr(Text::OpenXrGlRequirementsFailed))?;
        log::info!("OpenXR runtime: {runtime}, HMD: {headset}");
        Ok(Self {
            instance,
            system,
            info: RuntimeInfo { runtime, headset },
            graphics,
            max_layers: properties.graphics_properties.max_layer_count,
        })
    }
}

fn run(video: &VideoSource, cancel: &AtomicBool, proxy: &EventLoopProxy<AppEvent>) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let runtime = Runtime::connect()?;
    let (reply, prepared) = mpsc::sync_channel(1);
    proxy
        .send_event(AppEvent::Vr(Message::PrepareGraphics(
            runtime.graphics,
            reply,
        )))
        .map_err(|_| anyhow::anyhow!(tr(Text::PlayerWindowClosed)))?;
    let wgl = loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        match prepared.recv_timeout(Duration::from_millis(25)) {
            Ok(result) => break result?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    };
    let min = runtime.graphics.min_api_version_supported;
    let max = runtime.graphics.max_api_version_supported;
    let context = wgl
        .context
        .make_current(&wgl.surface)
        .context(tr(Text::VrContextActivateFailed))?;
    let gl = Arc::new(unsafe {
        glow::Context::from_loader_function(|name| {
            wgl.display.get_proc_address(&CString::new(name).unwrap())
        })
    });
    let result = (|| {
        check_opengl(&gl)?;
        let driver = gl.version();
        let version = xr::Version::new(driver.major as u16, driver.minor as u16, 0);
        ensure!(
            version >= min && version <= max,
            "{} (OpenGL {version}, {min}–{max})",
            tr(Text::VrGlVersionUnsupported),
        );
        log::info!("OpenXR using OpenGL {version} (runtime range {min}–{max})");
        run_session(runtime, &gl, video, cancel, proxy)
    })();
    let released = context
        .make_not_current()
        .context(tr(Text::VrContextReleaseFailed));
    result.and(released.map(|_| ()))
}

fn run_session(
    runtime: Runtime,
    gl: &Arc<glow::Context>,
    video: &VideoSource,
    cancel: &AtomicBool,
    proxy: &EventLoopProxy<AppEvent>,
) -> Result<()> {
    let Runtime {
        instance,
        system,
        info,
        max_layers,
        ..
    } = runtime;
    let (hdc, hglrc) = unsafe { (wglGetCurrentDC(), wglGetCurrentContext()) };
    ensure!(
        !hdc.is_null() && !hglrc.is_null(),
        tr(Text::VrWglBindingUnavailable)
    );
    // OpenXR only sees this thread's offscreen context. The desktop context stays on the UI thread.
    let (session, mut waiter, mut stream) = unsafe {
        instance.create_session::<xr::OpenGL>(
            system,
            &xr::opengl::SessionCreateInfo::Windows {
                h_dc: hdc as isize,
                h_glrc: hglrc as isize,
            },
        )
    }
    .with_context(|| format!("{} ({})", tr(Text::VrSessionCreateFailed), info.runtime))?;
    let space = session
        .create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)
        .context(tr(Text::OpenXrReferenceSpaceFailed))?;
    let blend_modes = instance
        .enumerate_environment_blend_modes(system, VIEW_TYPE)
        .context(tr(Text::OpenXrBlendModesFailed))?;
    let blend = [
        xr::EnvironmentBlendMode::OPAQUE,
        xr::EnvironmentBlendMode::ALPHA_BLEND,
        xr::EnvironmentBlendMode::ADDITIVE,
    ]
    .into_iter()
    .find(|mode| blend_modes.contains(mode))
    .context(tr(Text::OpenXrBlendModeUnavailable))?;
    let mut scene = Scene::new(gl, &instance, system, &session)?;
    let mut controllers = match input::Controllers::new(&session) {
        Ok(controllers) => Some(controllers),
        Err(error) => {
            report_controls_error(proxy, tr(Text::VrControllersInitFailed), error);
            None
        }
    };
    let mut panel = controllers.as_ref().and_then(|_| {
        let created = if max_layers >= 2 {
            panel::PlaybackPanel::new(gl, &session, scene.color_format, max_layers >= 3)
        } else {
            Err(anyhow::anyhow!(tr(Text::VrPanelCompositionUnavailable)))
        };
        match created {
            Ok(panel) => Some(panel),
            Err(error) => {
                report_controls_error(proxy, tr(Text::VrPlaybackPanelInitFailed), error);
                None
            }
        }
    });
    let _ = proxy.send_event(AppEvent::Vr(Message::Session(info)));

    let mut events = xr::EventDataBuffer::new();
    let mut running = false;
    let mut focused = false;
    let mut exit_requested = false;
    let mut recenter_at = None;
    let mut submitted = 0_u64;
    let mut controls_submitted = false;
    let mut browser_submitted = false;
    let created = Instant::now();
    loop {
        while let Some(event) = instance
            .poll_event(&mut events)
            .context(tr(Text::OpenXrEventsFailed))?
        {
            match event {
                xr::Event::InstanceLossPending(_) => bail!(tr(Text::OpenXrRuntimeDisconnected)),
                xr::Event::ReferenceSpaceChangePending(event)
                    if event.session() == session.as_raw()
                        && event.reference_space_type() == xr::ReferenceSpaceType::LOCAL =>
                {
                    recenter_at = Some(event.change_time());
                }
                xr::Event::InteractionProfileChanged(event)
                    if event.session() == session.as_raw() =>
                {
                    if let Some(controllers) = &controllers {
                        controllers.log_profiles(&session);
                    }
                }
                xr::Event::SessionStateChanged(event) if event.session() == session.as_raw() => {
                    log::info!("OpenXR session state: {:?}", event.state());
                    focused = event.state() == xr::SessionState::FOCUSED;
                    if !focused {
                        if let Some(controllers) = &mut controllers {
                            controllers.reset_input();
                        }
                        if let Some(panel) = &mut panel {
                            panel.cancel_gestures();
                        }
                    }
                    match event.state() {
                        xr::SessionState::READY if !running && !cancel.load(Ordering::Acquire) => {
                            session
                                .begin(VIEW_TYPE)
                                .context(tr(Text::OpenXrSessionBeginFailed))?;
                            running = true;
                        }
                        xr::SessionState::STOPPING if running => {
                            session.end().context(tr(Text::OpenXrSessionEndFailed))?;
                            log::info!("OpenXR projection frames submitted: {submitted}");
                            return Ok(());
                        }
                        xr::SessionState::EXITING => return Ok(()),
                        xr::SessionState::LOSS_PENDING => {
                            bail!(tr(Text::OpenXrSessionDisconnected))
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if cancel.load(Ordering::Acquire) && !exit_requested {
            if !running {
                return Ok(());
            }
            session
                .request_exit()
                .context(tr(Text::OpenXrSessionExitFailed))?;
            exit_requested = true;
        }
        if !running {
            ensure!(
                created.elapsed() < Duration::from_secs(15),
                tr(Text::OpenXrSessionTimeout)
            );
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        let state = waiter.wait().context(tr(Text::OpenXrFrameWaitFailed))?;
        stream.begin().context(tr(Text::OpenXrFrameBeginFailed))?;
        let time = state.predicted_display_time;
        if recenter_at.is_some_and(|change: xr::Time| time.as_nanos() >= change.as_nanos()) {
            scene.anchor = None;
            recenter_at = None;
        }
        let now = Instant::now();
        let preferences = video.lock().unwrap().preferences;
        let should_render = state.should_render && !exit_requested;
        let mut input = input::FrameInput::default();
        if should_render && let Some(devices) = &mut controllers {
            let locate_aim = panel
                .as_ref()
                .is_some_and(|panel| panel.visible(now, preferences.panel_timeout));
            match devices.read(&session, time, focused, locate_aim) {
                Ok(frame) => input = frame,
                Err(error) => {
                    report_controls_error(proxy, tr(Text::VrControllersReadFailed), error);
                    controllers = None;
                    panel = None;
                }
            }
        }
        let scroll = if input.motion.zoom != 0.0
            && panel
                .as_ref()
                .is_some_and(|panel| panel.browser_aimed(&input.hands))
        {
            let seconds =
                (state.predicted_display_period.as_nanos() as f32 * 1e-9).clamp(0.0, 0.05);
            -std::mem::take(&mut input.motion.zoom) * FILE_SCROLL_SPEED * seconds
        } else {
            0.0
        };
        if let Some(direction) = input.seek_direction {
            let seconds = direction * f64::from(preferences.vr_seek);
            let _ = proxy.send_event(AppEvent::Vr(Message::Playback(Action::SeekBy(seconds))));
        }
        scene.adjust_view(&input.motion, state.predicted_display_period);

        let rendered = if should_render {
            scene.render(&session, &space, time, video)
        } else {
            Ok(None)
        };
        if let Ok(Some((views, has_video))) = rendered {
            let projection_views = std::array::from_fn::<_, 2, _>(|eye| {
                xr::CompositionLayerProjectionView::new()
                    .pose(views[eye].pose)
                    .fov(views[eye].fov)
                    .sub_image(
                        xr::SwapchainSubImage::new()
                            .swapchain(&scene.eyes[eye].swapchain)
                            .image_rect(scene.eyes[eye].rect),
                    )
            });
            let layer = xr::CompositionLayerProjection::new()
                .space(&space)
                .views(&projection_views);
            let panel_frame = match panel
                .as_mut()
                .map(|panel| panel.update(input.hands, video, now, scroll, preferences, proxy))
            {
                Some(Ok(frame)) => frame,
                Some(Err(error)) => {
                    report_controls_error(proxy, tr(Text::VrPlaybackPanelUpdateFailed), error);
                    panel = None;
                    None
                }
                None => None,
            };
            let (end, show_cursor) = if let (Some(panel), Some(controllers), Some(frame)) =
                (&panel, &controllers, panel_frame)
            {
                let view = &controllers.view;
                let panel_layer = panel.layer(view);
                if let Some(cursor_layer) = frame
                    .outside_pointer
                    .and_then(|point| panel.cursor_layer(view, point))
                {
                    (
                        stream.end(time, blend, &[&layer, &panel_layer, &cursor_layer]),
                        true,
                    )
                } else {
                    (stream.end(time, blend, &[&layer, &panel_layer]), false)
                }
            } else {
                (stream.end(time, blend, &[&layer]), false)
            };
            if panel_frame.is_some()
                && let Err(
                    error @ (xr::sys::Result::ERROR_LAYER_LIMIT_EXCEEDED
                    | xr::sys::Result::ERROR_LAYER_INVALID),
                ) = end
            {
                if show_cursor && let Some(panel) = &mut panel {
                    log::warn!("Could not submit the VR pointer: {error}");
                    panel.disable_cursor();
                } else {
                    report_controls_error(proxy, tr(Text::VrPlaybackPanelSubmitFailed), error);
                    panel = None;
                }
                continue;
            }
            end.context(tr(Text::OpenXrStereoFrameSubmitFailed))?;
            if panel_frame.is_some() && !controls_submitted {
                controls_submitted = true;
                log::info!("OpenXR playback panel submitted");
            }
            if panel_frame.is_some_and(|frame| frame.browser_visible) && !browser_submitted {
                browser_submitted = true;
                log::info!("OpenXR file browser panel submitted");
            }
            submitted += 1;
            if submitted == 1 {
                log::info!("OpenXR first projection layer submitted (video={has_video})");
                let _ = proxy.send_event(AppEvent::Vr(Message::Presenting));
            }
        } else {
            let end = stream
                .end(time, blend, &[])
                .context(tr(Text::OpenXrFrameEndFailed));
            rendered?;
            end?;
        }
    }
}

fn report_controls_error(
    proxy: &EventLoopProxy<AppEvent>,
    context: &str,
    error: impl std::fmt::Display,
) {
    let message = format!("{context} ({error:#})");
    log::warn!("{message}");
    let _ = proxy.send_event(AppEvent::Vr(Message::ControlsUnavailable(message)));
}

struct GlSwapchain<'a> {
    gl: &'a glow::Context,
    swapchain: xr::Swapchain<xr::OpenGL>,
    framebuffers: Vec<glow::Framebuffer>,
    rect: xr::Rect2Di,
}

impl<'a> GlSwapchain<'a> {
    fn new(
        gl: &'a glow::Context,
        session: &xr::Session<xr::OpenGL>,
        size: [u32; 2],
        format: u32,
    ) -> Result<Self> {
        Self::with_flags(gl, session, size, format, xr::SwapchainCreateFlags::EMPTY)
    }

    fn with_flags(
        gl: &'a glow::Context,
        session: &xr::Session<xr::OpenGL>,
        [width, height]: [u32; 2],
        format: u32,
        create_flags: xr::SwapchainCreateFlags,
    ) -> Result<Self> {
        ensure!(
            width > 0
                && height > 0
                && i32::try_from(width).is_ok()
                && i32::try_from(height).is_ok(),
            tr(Text::OpenXrSwapchainSizeInvalid)
        );
        let swapchain = session
            .create_swapchain(&xr::SwapchainCreateInfo {
                create_flags,
                usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT,
                format,
                sample_count: 1,
                width,
                height,
                face_count: 1,
                array_size: 1,
                mip_count: 1,
            })
            .context(tr(Text::OpenXrSwapchainCreateFailed))?;
        let images = swapchain
            .enumerate_images()
            .context(tr(Text::OpenXrSwapchainImagesFailed))?;
        ensure!(!images.is_empty(), tr(Text::OpenXrSwapchainEmpty));
        let mut images_set = Self {
            gl,
            swapchain,
            framebuffers: Vec::with_capacity(images.len()),
            rect: xr::Rect2Di {
                offset: xr::Offset2Di { x: 0, y: 0 },
                extent: xr::Extent2Di {
                    width: width as i32,
                    height: height as i32,
                },
            },
        };
        for name in images {
            let texture = glow::NativeTexture(
                NonZeroU32::new(name).context(tr(Text::OpenXrSwapchainTextureMissing))?,
            );
            unsafe {
                let fbo = gl.create_framebuffer().map_err(anyhow::Error::msg)?;
                images_set.framebuffers.push(fbo);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(texture),
                    0,
                );
                let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                ensure!(
                    status == glow::FRAMEBUFFER_COMPLETE,
                    "{} (OpenGL {status:#x})",
                    tr(Text::OpenXrFramebufferIncomplete)
                );
            }
        }
        Ok(images_set)
    }

    fn acquire(&mut self) -> Result<glow::Framebuffer> {
        let image = self
            .swapchain
            .acquire_image()
            .context(tr(Text::OpenXrImageAcquireFailed))? as usize;
        let fbo = self
            .framebuffers
            .get(image)
            .copied()
            .context(tr(Text::OpenXrImageIndexInvalid))?;
        self.swapchain
            .wait_image(xr::Duration::INFINITE)
            .context(tr(Text::OpenXrImageWaitFailed))?;
        Ok(fbo)
    }
}

impl Drop for GlSwapchain<'_> {
    fn drop(&mut self) {
        for fbo in self.framebuffers.drain(..) {
            unsafe { self.gl.delete_framebuffer(fbo) };
        }
    }
}

struct Anchor {
    origin: Vec3,
    reference: Quat,
}

impl Anchor {
    fn new(views: &[xr::View; 2], center_pitch: bool) -> Self {
        let origin = (position(views[0].pose) + position(views[1].pose)) * 0.5;
        let forward =
            orientation(views[0].pose).slerp(orientation(views[1].pose), 0.5) * Vec3::NEG_Z;
        let yaw = (-forward.x).atan2(-forward.z);
        let pitch = if center_pitch {
            forward.y.clamp(-1.0, 1.0).asin()
        } else {
            0.0
        };
        Self {
            origin,
            reference: (Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch)).conjugate(),
        }
    }

    fn eye(&self, view: &xr::View, turn: Quat) -> EyeView {
        let rotation = turn * self.reference;
        EyeView {
            position: rotation * (position(view.pose) - self.origin),
            orientation: rotation * orientation(view.pose),
            tangents: [
                view.fov.angle_left.tan(),
                view.fov.angle_right.tan(),
                view.fov.angle_down.tan(),
                view.fov.angle_up.tan(),
            ],
        }
    }
}

fn orientation(pose: xr::Posef) -> Quat {
    let q = pose.orientation;
    Quat::from_xyzw(q.x, q.y, q.z, q.w)
}

fn position(pose: xr::Posef) -> Vec3 {
    let v = pose.position;
    Vec3::new(v.x, v.y, v.z)
}

struct ViewAdjustment {
    zoom: f32,
    yaw: f32,
    pitch: f32,
}

impl Default for ViewAdjustment {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

struct Scene<'a> {
    gl: &'a glow::Context,
    eyes: [GlSwapchain<'a>; 2],
    program: Option<VrProgram>,
    anchor: Option<Anchor>,
    adjustment: ViewAdjustment,
    center_pitch: bool,
    color_format: u32,
}

impl<'a> Scene<'a> {
    fn new(
        gl: &'a glow::Context,
        instance: &xr::Instance,
        system: xr::SystemId,
        session: &xr::Session<xr::OpenGL>,
    ) -> Result<Self> {
        let formats = session
            .enumerate_swapchain_formats()
            .context(tr(Text::OpenXrSwapchainFormatsFailed))?;
        let format = [
            glow::SRGB8_ALPHA8,
            glow::RGBA16F,
            glow::RGB10_A2,
            glow::RGBA8,
        ]
        .into_iter()
        .find(|format| formats.contains(format))
        .context(tr(Text::OpenXrSwapchainFormatUnavailable))?;
        let views = instance
            .enumerate_view_configuration_views(system, VIEW_TYPE)
            .context(tr(Text::OpenXrStereoConfigFailed))?;
        let [left, right] = views.as_slice() else {
            bail!(tr(Text::OpenXrStereoRequired))
        };
        let eye_size = |view: &xr::ViewConfigurationView| {
            [
                view.recommended_image_rect_width,
                view.recommended_image_rect_height,
            ]
        };
        let eyes = [
            GlSwapchain::new(gl, session, eye_size(left), format)?,
            GlSwapchain::new(gl, session, eye_size(right), format)?,
        ];
        log::info!(
            "OpenXR swapchains: {:?} and {:?}, format {format:#x}",
            eyes[0].rect.extent,
            eyes[1].rect.extent
        );
        Ok(Self {
            gl,
            eyes,
            program: Some(VrProgram::new(gl)?),
            anchor: None,
            adjustment: ViewAdjustment::default(),
            center_pitch: false,
            color_format: format,
        })
    }

    fn adjust_view(&mut self, motion: &input::ViewMotion, period: xr::Duration) {
        if motion.reset {
            self.anchor = None;
            self.adjustment = ViewAdjustment::default();
            self.center_pitch = true;
            log::info!("OpenXR view recentered");
            return;
        }
        let seconds = (period.as_nanos() as f32 * 1e-9).clamp(0.0, 0.05);
        if motion.zoom != 0.0 {
            self.adjustment.zoom =
                (self.adjustment.zoom * (motion.zoom * seconds).exp()).clamp(0.5, 3.0);
        }
        if motion.turn != glam::Vec2::ZERO {
            self.adjustment.yaw =
                (self.adjustment.yaw - motion.turn.x * TURN_RATE * seconds) % std::f32::consts::TAU;
            self.adjustment.pitch = (self.adjustment.pitch + motion.turn.y * TURN_RATE * seconds)
                .clamp(-PITCH_LIMIT, PITCH_LIMIT);
        }
    }

    fn render(
        &mut self,
        session: &xr::Session<xr::OpenGL>,
        space: &xr::Space,
        time: xr::Time,
        video: &VideoSource,
    ) -> Result<Option<([xr::View; 2], bool)>> {
        let (valid, views) = session
            .locate_views(VIEW_TYPE, time, space)
            .context(tr(Text::OpenXrHeadsetLocateFailed))?;
        let required = xr::ViewStateFlags::ORIENTATION_VALID | xr::ViewStateFlags::POSITION_VALID;
        if !valid.contains(required) {
            return Ok(None);
        }
        let views: [xr::View; 2] = views
            .try_into()
            .map_err(|_| anyhow::anyhow!(tr(Text::OpenXrEyePosesUnavailable)))?;
        let anchor = self
            .anchor
            .get_or_insert_with(|| Anchor::new(&views, self.center_pitch));
        let turn = Quat::from_rotation_y(self.adjustment.yaw)
            * Quat::from_rotation_x(self.adjustment.pitch);
        let eye_views = views.map(|view| anchor.eye(&view, turn));
        let [left, right] = &mut self.eyes;
        let left_fbo = left.acquire()?;
        let right_fbo = match right.acquire() {
            Ok(fbo) => fbo,
            Err(error) => {
                let _ = left.swapchain.release_image();
                return Err(error);
            }
        };
        let gl = self.gl;
        let program = self.program.as_ref().unwrap();
        let mut has_video = false;
        let draw = video.lock().unwrap().draw_eyes(gl, |frame, mut settings| {
            has_video = frame.is_some();
            unsafe {
                gl.disable(glow::BLEND);
                gl.disable(glow::DEPTH_TEST);
                gl.disable(glow::SCISSOR_TEST);
                gl.disable(glow::CULL_FACE);
                gl.disable(glow::FRAMEBUFFER_SRGB);
                gl.color_mask(true, true, true, true);
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
            }
            let ray_scale = if settings.projection == Projection::Flat {
                settings.screen_width *= self.adjustment.zoom;
                1.0
            } else {
                self.adjustment.zoom.recip()
            };
            if let Some(frame) = frame {
                program.bind(gl, frame, settings, self.color_format != glow::SRGB8_ALPHA8);
            }
            for (eye, (fbo, rect)) in [(left_fbo, left.rect), (right_fbo, right.rect)]
                .into_iter()
                .enumerate()
            {
                unsafe {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                    gl.viewport(0, 0, rect.extent.width, rect.extent.height);
                }
                if frame.is_some() {
                    program.draw(gl, &eye_views[eye], settings.stereo, eye, ray_scale);
                } else {
                    unsafe { gl.clear(glow::COLOR_BUFFER_BIT) };
                }
            }
            unsafe { gl.bind_framebuffer(glow::FRAMEBUFFER, None) };
        });
        let left_released = left.swapchain.release_image();
        let right_released = right.swapchain.release_image();
        draw?;
        left_released.context(tr(Text::OpenXrLeftImageReleaseFailed))?;
        right_released.context(tr(Text::OpenXrRightImageReleaseFailed))?;
        Ok(Some((views, has_video)))
    }
}

impl Drop for Scene<'_> {
    fn drop(&mut self) {
        // Swapchains are destroyed after the outstanding eye draws have finished.
        unsafe { self.gl.finish() };
        if let Some(program) = self.program.take() {
            program.delete(self.gl);
        }
    }
}
