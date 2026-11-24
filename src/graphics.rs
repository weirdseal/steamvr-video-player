use std::mem;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail, ensure};
use glam::{Quat, Vec3};
use glow::HasContext as _;
use serde::{Deserialize, Serialize};

use crate::i18n::{Text, tr};
use crate::mpv::{Mpv, Playback, RenderTarget};
use crate::settings::Preferences;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    #[default]
    Flat = 0,
    Hemisphere = 1,
    Sphere = 2,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stereo {
    #[default]
    Mono,
    SideBySide,
    TopBottom,
}

impl Stereo {
    fn rect(self, eye: usize) -> [f32; 4] {
        match self {
            Self::Mono => [0.0, 0.0, 1.0, 1.0],
            Self::SideBySide => [eye as f32 * 0.5, 0.0, 0.5, 1.0],
            Self::TopBottom => [0.0, (1 - eye) as f32 * 0.5, 1.0, 0.5],
        }
    }

    fn eye_aspect(self, frame_aspect: f32, half_resolution: bool) -> f32 {
        if half_resolution {
            return frame_aspect;
        }
        let [_, _, width, height] = self.rect(0);
        frame_aspect * width / height
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct VrSettings {
    pub projection: Projection,
    pub stereo: Stereo,
    pub half_resolution: bool,
    pub screen_width: f32,
    pub screen_distance: f32,
}

impl Default for VrSettings {
    fn default() -> Self {
        Self {
            projection: Projection::Flat,
            stereo: Stereo::Mono,
            half_resolution: false,
            screen_width: 3.5,
            screen_distance: 3.0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Frame {
    pub texture: glow::Texture,
    extent: [u32; 2],
}

impl Frame {
    pub fn aspect(self) -> f32 {
        self.extent[0] as f32 / self.extent[1] as f32
    }
}

struct Fence(glow::Fence);

// GL sync objects are shared by these two WGL contexts. Access to a handle is
// transferred under the mutex and each caller uses its own current context.
unsafe impl Send for Fence {}

impl Fence {
    fn new(gl: &glow::Context) -> Result<Self> {
        let fence = unsafe { gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0) }
            .map_err(anyhow::Error::msg)?;
        ensure!(!fence.0.is_null(), tr(Text::GlFenceFailed));
        Ok(Self(fence))
    }

    fn wait(self, gl: &glow::Context) {
        unsafe { gl.wait_sync(self.0, 0, glow::TIMEOUT_IGNORED) };
        self.delete(gl);
    }

    fn delete(self, gl: &glow::Context) {
        unsafe { gl.delete_sync(self.0) };
    }
}

struct PublishedFrame {
    frame: Frame,
    slot: usize,
    ready: Option<Fence>,
}

#[derive(Default)]
pub struct SharedVideo {
    frame: Option<PublishedFrame>,
    readers: [Option<Fence>; 2],
    pub preferences: Preferences,
    pub playback: Playback,
}

pub type VideoSource = Arc<Mutex<SharedVideo>>;

impl SharedVideo {
    pub fn draw_eyes(
        &mut self,
        gl: &glow::Context,
        draw: impl FnOnce(Option<Frame>, VrSettings),
    ) -> Result<()> {
        let Some(current) = &mut self.frame else {
            draw(None, self.preferences.vr);
            unsafe { gl.flush() };
            return Ok(());
        };

        if let Some(ready) = current.ready.take() {
            ready.wait(gl);
        }
        draw(Some(current.frame), self.preferences.vr);

        let read_complete = Fence::new(gl);
        if let Some(previous) = self.readers[current.slot].take() {
            previous.delete(gl);
        }
        match read_complete {
            Ok(fence) => {
                unsafe { gl.flush() };
                self.readers[current.slot] = Some(fence);
                Ok(())
            }
            Err(error) => {
                // If the fence could not be made, finish before the slot can be reused.
                unsafe { gl.finish() };
                Err(error).context(tr(Text::VrTextureReadSyncFailed))
            }
        }
    }
}

struct VideoTarget {
    frame: Frame,
    fbo: glow::Framebuffer,
}

impl VideoTarget {
    fn new(gl: &glow::Context, extent: [u32; 2]) -> Result<Self> {
        unsafe {
            let fbo = gl.create_framebuffer().map_err(anyhow::Error::msg)?;
            let texture = match gl.create_texture() {
                Ok(texture) => texture,
                Err(error) => {
                    gl.delete_framebuffer(fbo);
                    bail!("{error}");
                }
            };
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                extent[0] as i32,
                extent[1] as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            for (parameter, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, value as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
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

            let target = Self {
                frame: Frame { texture, extent },
                fbo,
            };
            if status != glow::FRAMEBUFFER_COMPLETE {
                target.delete(gl);
                bail!(
                    "{} ({}×{}, OpenGL {status:#x})",
                    tr(Text::VideoTextureAllocationFailed),
                    extent[0],
                    extent[1]
                );
            }
            Ok(target)
        }
    }

    fn delete(self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_texture(self.frame.texture);
        }
    }

    fn mpv_target(&self) -> RenderTarget {
        RenderTarget {
            fbo: self.fbo.0.get() as i32,
            width: self.frame.extent[0] as i32,
            height: self.frame.extent[1] as i32,
            internal_format: glow::RGBA8 as i32,
        }
    }
}

pub struct Video {
    gl: Arc<glow::Context>,
    shared: VideoSource,
    slots: [Option<VideoTarget>; 2],
    last_slot: usize,
    max_extent: u32,
    sharing: bool,
}

impl Video {
    pub fn new(gl: Arc<glow::Context>) -> Self {
        let max_extent = unsafe { gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) } as u32;
        Self {
            gl,
            shared: Arc::default(),
            slots: [None, None],
            last_slot: 0,
            max_extent,
            sharing: false,
        }
    }

    pub fn frame(&self) -> Option<Frame> {
        self.shared
            .lock()
            .unwrap()
            .frame
            .as_ref()
            .map(|current| current.frame)
    }

    pub fn clear(&mut self) {
        if let Some(ready) = self
            .shared
            .lock()
            .unwrap()
            .frame
            .take()
            .and_then(|current| current.ready)
        {
            ready.delete(&self.gl);
        }
    }

    pub fn preferences(&self, preferences: Preferences) {
        self.shared.lock().unwrap().preferences = preferences;
    }

    pub fn playback(&self, playback: &Playback) {
        if self.sharing {
            self.shared.lock().unwrap().playback = playback.clone();
        }
    }

    pub fn start_sharing(&mut self, playback: &Playback) -> Result<VideoSource> {
        let mut shared = self.shared.lock().unwrap();
        shared.playback = playback.clone();
        if let Some(current) = &mut shared.frame {
            current.ready = Some(Fence::new(&self.gl)?);
            unsafe { self.gl.flush() };
        }
        self.sharing = true;
        Ok(self.shared.clone())
    }

    // Only call after the VR thread has released the shared context.
    pub fn stop_sharing(&mut self) {
        self.sharing = false;
        let mut shared = self.shared.lock().unwrap();
        if let Some(ready) = shared
            .frame
            .as_mut()
            .and_then(|current| current.ready.take())
        {
            ready.delete(&self.gl);
        }
        for reader in &mut shared.readers {
            if let Some(fence) = reader.take() {
                fence.wait(&self.gl);
            }
        }
        if let Some(unused) = self.slots[1 - self.last_slot].take() {
            unused.delete(&self.gl);
        }
    }

    pub fn render(&mut self, mpv: &Mpv, dimensions: [u32; 2]) -> Result<()> {
        let max_side = dimensions[0].max(dimensions[1]);
        let extent = if max_side > self.max_extent {
            dimensions.map(|side| {
                ((u64::from(side) * u64::from(self.max_extent) / u64::from(max_side)) as u32).max(1)
            })
        } else {
            dimensions
        };
        let slot = if self.sharing {
            1 - self.last_slot
        } else {
            self.last_slot
        };
        let wait = self.shared.lock().unwrap().readers[slot].take();
        if let Some(fence) = wait {
            fence.wait(&self.gl);
        }
        let gl = &self.gl;
        if self.slots[slot]
            .as_ref()
            .is_none_or(|target| target.frame.extent != extent)
        {
            let target = match VideoTarget::new(gl, extent) {
                Ok(target) => target,
                Err(error) => {
                    let _ = mpv.render(None);
                    return Err(error);
                }
            };
            if let Some(previous) = self.slots[slot].replace(target) {
                previous.delete(gl);
            }
        }
        let target = self.slots[slot].as_ref().unwrap();
        unsafe {
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.disable(glow::FRAMEBUFFER_SRGB);
            gl.active_texture(glow::TEXTURE0);
        }
        let rendered = mpv.render(Some(target.mpv_target()));
        unsafe { gl.bind_framebuffer(glow::FRAMEBUFFER, None) };
        rendered.context(tr(Text::MpvFrameRenderFailed))?;
        let ready = if self.sharing {
            match Fence::new(gl) {
                Ok(fence) => {
                    unsafe { gl.flush() };
                    Some(fence)
                }
                Err(error) => {
                    log::warn!("Could not synchronize video texture writes: {error:#}");
                    unsafe { gl.finish() };
                    None
                }
            }
        } else {
            None
        };
        let mut shared = self.shared.lock().unwrap();
        if let Some(previous) = shared.frame.replace(PublishedFrame {
            frame: target.frame,
            slot,
            ready,
        }) && let Some(fence) = previous.ready
        {
            fence.delete(gl);
        }
        self.last_slot = slot;
        Ok(())
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        self.stop_sharing();
        self.clear();
        for target in mem::take(&mut self.slots).into_iter().flatten() {
            target.delete(&self.gl);
        }
    }
}

const TRIANGLE_VERTEX: &str = r#"#version 330 core
out vec2 uv;
void main() {
    vec2 point = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    uv = point;
    gl_Position = vec4(point * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const DESKTOP_PIXEL: &str = r#"#version 330 core
in vec2 uv;
out vec4 color;
uniform sampler2D picture;
void main() {
    color = vec4(texture(picture, uv).rgb, 1.0);
}
"#;

const VR_CURSOR_PIXEL: &str = r#"#version 330 core
in vec2 uv;
out vec4 color;

vec3 srgb_to_linear(vec3 rgb) {
    return mix(pow((rgb + 0.055) / 1.055, vec3(2.4)), rgb / 12.92,
               lessThanEqual(rgb, vec3(0.04045)));
}

void main() {
    float radius = length(uv - 0.5);
    float alpha = 1.0 - smoothstep(0.26, 0.30, radius);
    float fill = 1.0 - smoothstep(0.20, 0.24, radius);
    vec3 border = srgb_to_linear(vec3(20.0 / 255.0));
    vec3 center = vec3(1.0);
    color = vec4(mix(border, center, fill) * alpha, alpha);
}
"#;

pub fn paint_vr_cursor(gl: &glow::Context, fbo: glow::Framebuffer, size: i32) -> Result<()> {
    let triangle = Triangle::new(gl, VR_CURSOR_PIXEL)?;
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.viewport(0, 0, size, size);
        gl.disable(glow::BLEND);
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::CULL_FACE);
        gl.enable(glow::FRAMEBUFFER_SRGB);
        gl.color_mask(true, true, true, true);
        gl.use_program(Some(triangle.program));
        gl.bind_vertex_array(Some(triangle.vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.use_program(None);
        gl.disable(glow::FRAMEBUFFER_SRGB);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.finish();
    }
    triangle.delete(gl);
    Ok(())
}

#[derive(Clone, Copy)]
pub struct Triangle {
    program: glow::Program,
    vao: glow::VertexArray,
}

impl Triangle {
    fn new(gl: &glow::Context, fragment_source: &str) -> Result<Self> {
        unsafe {
            let vertex = compile(gl, glow::VERTEX_SHADER, TRIANGLE_VERTEX)?;
            let fragment = match compile(gl, glow::FRAGMENT_SHADER, fragment_source) {
                Ok(fragment) => fragment,
                Err(error) => {
                    gl.delete_shader(vertex);
                    return Err(error);
                }
            };
            let program = gl.create_program().map_err(anyhow::Error::msg);
            if let Ok(program) = program {
                gl.attach_shader(program, vertex);
                gl.attach_shader(program, fragment);
                gl.link_program(program);
                gl.detach_shader(program, vertex);
                gl.detach_shader(program, fragment);
            }
            gl.delete_shader(vertex);
            gl.delete_shader(fragment);
            let program = program?;
            if !gl.get_program_link_status(program) {
                let error = gl.get_program_info_log(program);
                gl.delete_program(program);
                bail!("{} ({error})", tr(Text::GlShaderLinkFailed));
            }
            let vao = match gl.create_vertex_array() {
                Ok(vao) => vao,
                Err(error) => {
                    gl.delete_program(program);
                    bail!("{error}");
                }
            };
            gl.use_program(Some(program));
            let picture = gl.get_uniform_location(program, "picture");
            gl.uniform_1_i32(picture.as_ref(), 0);
            gl.use_program(None);
            Ok(Self { program, vao })
        }
    }

    pub fn desktop(gl: &glow::Context) -> Result<Self> {
        Self::new(gl, DESKTOP_PIXEL)
    }

    fn bind(self, gl: &glow::Context, frame: Frame) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_sampler(0, None);
            gl.bind_texture(glow::TEXTURE_2D, Some(frame.texture));
        }
    }

    pub fn draw_desktop(self, gl: &glow::Context, frame: Frame) {
        self.bind(gl, frame);
        unsafe {
            gl.disable(glow::FRAMEBUFFER_SRGB);
            gl.disable(glow::BLEND);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    pub fn delete(self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.vao);
            gl.delete_program(self.program);
        }
    }
}

unsafe fn compile(gl: &glow::Context, kind: u32, source: &str) -> Result<glow::Shader> {
    unsafe {
        let shader = gl.create_shader(kind).map_err(anyhow::Error::msg)?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let message = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            bail!("{} ({message})", tr(Text::GlShaderCompileFailed));
        }
        Ok(shader)
    }
}

pub struct EyeView {
    pub orientation: Quat,
    pub position: Vec3,
    pub tangents: [f32; 4],
}

pub struct VrProgram {
    triangle: Triangle,
    projection: glow::UniformLocation,
    screen: glow::UniformLocation,
    rotation: glow::UniformLocation,
    origin: glow::UniformLocation,
    tangents: glow::UniformLocation,
    region: glow::UniformLocation,
    linear_target: glow::UniformLocation,
}

impl VrProgram {
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let triangle = Triangle::new(gl, include_str!("vr.frag"))?;
        let uniform = |name| unsafe { gl.get_uniform_location(triangle.program, name) };
        let locations = [
            "projection",
            "screen",
            "rotation",
            "origin",
            "tangents",
            "region",
            "linear_target",
        ]
        .map(uniform);
        let [
            Some(projection),
            Some(screen),
            Some(rotation),
            Some(origin),
            Some(tangents),
            Some(region),
            Some(linear_target),
        ] = locations
        else {
            triangle.delete(gl);
            bail!(tr(Text::VrShaderParametersFailed));
        };
        Ok(Self {
            triangle,
            projection,
            screen,
            rotation,
            origin,
            tangents,
            region,
            linear_target,
        })
    }

    pub fn bind(
        &self,
        gl: &glow::Context,
        frame: Frame,
        settings: VrSettings,
        linear_target: bool,
    ) {
        self.triangle.bind(gl, frame);
        let aspect = settings
            .stereo
            .eye_aspect(frame.aspect(), settings.half_resolution);
        unsafe {
            gl.uniform_1_i32(Some(&self.projection), settings.projection as i32);
            gl.uniform_3_f32(
                Some(&self.screen),
                settings.screen_width,
                settings.screen_width / aspect,
                settings.screen_distance,
            );
            gl.uniform_1_i32(Some(&self.linear_target), i32::from(linear_target));
        }
    }

    pub fn draw(
        &self,
        gl: &glow::Context,
        view: &EyeView,
        stereo: Stereo,
        eye: usize,
        ray_scale: f32,
    ) {
        let tangents = view.tangents.map(|tangent| tangent * ray_scale);
        unsafe {
            gl.uniform_4_f32_slice(Some(&self.rotation), &view.orientation.to_array());
            gl.uniform_3_f32_slice(Some(&self.origin), &view.position.to_array());
            gl.uniform_4_f32_slice(Some(&self.tangents), &tangents);
            gl.uniform_4_f32_slice(Some(&self.region), &stereo.rect(eye));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    pub fn delete(self, gl: &glow::Context) {
        self.triangle.delete(gl);
    }
}

pub fn check_opengl(gl: &glow::Context) -> Result<()> {
    let version = gl.version();
    ensure!(
        !version.is_embedded && (version.major, version.minor) >= (3, 3),
        "{} ({}.{}).",
        tr(Text::DesktopGlVersionRequired),
        version.major,
        version.minor
    );
    Ok(())
}
