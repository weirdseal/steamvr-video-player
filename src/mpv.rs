use std::borrow::Cow;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail, ensure};
use glutin::display::{Display, GlDisplay};
use libloading::os::windows::Library;
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use winit::event_loop::EventLoopProxy;

use crate::files::NearbyVideos;
use crate::i18n::{Text, tr};
use crate::{AppEvent, executable_dir};

pub(crate) mod thumbnail;

// Subset of libmpv's client.h, render.h and render_gl.h ABI.
const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;
const EVENT_NONE: c_int = 0;
const EVENT_LOG_MESSAGE: c_int = 2;
const EVENT_GET_PROPERTY_REPLY: c_int = 3;
const EVENT_COMMAND_REPLY: c_int = 5;
const EVENT_START_FILE: c_int = 6;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_PROPERTY_CHANGE: c_int = 22;
const EVENT_QUEUE_OVERFLOW: c_int = 24;
const END_ERROR: c_int = 4;

const POSITION: u64 = 1;
const DURATION: u64 = 2;
const PAUSE: u64 = 3;
const VOLUME: u64 = 4;
const MUTE: u64 = 5;
const SEEKABLE: u64 = 6;
const WIDTH: u64 = 7;
const HEIGHT: u64 = 8;
const ROTATION: u64 = 9;
const EOF_REACHED: u64 = 10;
const LOAD_COMMAND: u64 = 1;

const PROPERTIES: &[(u64, &CStr, c_int)] = &[
    (POSITION, c"time-pos", FORMAT_DOUBLE),
    (DURATION, c"duration", FORMAT_DOUBLE),
    (PAUSE, c"pause", FORMAT_FLAG),
    (VOLUME, c"volume", FORMAT_DOUBLE),
    (MUTE, c"mute", FORMAT_FLAG),
    (SEEKABLE, c"seekable", FORMAT_FLAG),
    (WIDTH, c"dwidth", FORMAT_INT64),
    (HEIGHT, c"dheight", FORMAT_INT64),
    (ROTATION, c"video-out-params/rotate", FORMAT_INT64),
    (EOF_REACHED, c"eof-reached", FORMAT_FLAG),
];

type Handle = *mut c_void;
type Notify = unsafe extern "C" fn(*mut c_void);

#[repr(C)]
struct Event {
    id: c_int,
    error: c_int,
    user_data: u64,
    data: *const c_void,
}

#[repr(C)]
struct Property {
    name: *const c_char,
    format: c_int,
    data: *const c_void,
}

impl Property {
    fn title(&self) -> Option<Arc<str>> {
        if self.format != FORMAT_STRING || self.data.is_null() {
            return None;
        }
        let string = unsafe { self.data.cast::<*const c_char>().read() };
        if string.is_null() {
            return None;
        }
        let value = unsafe { CStr::from_ptr(string) }.to_string_lossy();
        let title = value.trim();
        (!title.is_empty()).then(|| Arc::from(title))
    }

    fn number(&self) -> Option<f64> {
        (self.format == FORMAT_DOUBLE && !self.data.is_null())
            .then(|| unsafe { self.data.cast::<f64>().read() })
            .filter(|value| value.is_finite())
    }

    fn integer(&self) -> i64 {
        if self.format == FORMAT_INT64 && !self.data.is_null() {
            unsafe { self.data.cast::<i64>().read() }
        } else {
            0
        }
    }

    fn flag(&self) -> bool {
        self.format == FORMAT_FLAG
            && !self.data.is_null()
            && unsafe { self.data.cast::<c_int>().read() != 0 }
    }
}

#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}

#[repr(C)]
struct LogMessage {
    prefix: *const c_char,
    level: *const c_char,
    text: *const c_char,
    log_level: c_int,
}

#[repr(i32)]
#[derive(Clone, Copy)]
enum ParamType {
    End = 0,
    ApiType = 1,
    OpenGlInit = 2,
    Framebuffer = 3,
    FlipY = 4,
    AdvancedControl = 10,
    NextFrameInfo = 11,
    BlockForTargetTime = 12,
    SkipRendering = 13,
    SoftwareSize = 17,
    SoftwareFormat = 18,
    SoftwareStride = 19,
    SoftwarePointer = 20,
}

#[repr(C)]
struct RenderParam {
    kind: ParamType,
    data: *mut c_void,
}

impl RenderParam {
    const END: Self = Self {
        kind: ParamType::End,
        data: ptr::null_mut(),
    };

    fn new<T>(kind: ParamType, value: &mut T) -> Self {
        Self {
            kind,
            data: ptr::from_mut(value).cast(),
        }
    }

    fn string(kind: ParamType, value: &CStr) -> Self {
        Self {
            kind,
            data: value.as_ptr().cast_mut().cast(),
        }
    }
}

#[repr(C)]
struct OpenGlInit {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    context: *mut c_void,
}

#[repr(C)]
pub struct RenderTarget {
    pub fbo: c_int,
    pub width: c_int,
    pub height: c_int,
    pub internal_format: c_int,
}

struct Api {
    api_version: unsafe extern "C" fn() -> std::ffi::c_ulong,
    create: unsafe extern "C" fn() -> Handle,
    initialize: unsafe extern "C" fn(Handle) -> c_int,
    destroy: unsafe extern "C" fn(Handle),
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    set_option: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    observe: unsafe extern "C" fn(Handle, u64, *const c_char, c_int) -> c_int,
    get_property_async: unsafe extern "C" fn(Handle, u64, *const c_char, c_int) -> c_int,
    command_async: unsafe extern "C" fn(Handle, u64, *const *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(Handle, f64) -> *const Event,
    wakeup_callback: unsafe extern "C" fn(Handle, Option<Notify>, *mut c_void),
    request_log_messages: unsafe extern "C" fn(Handle, *const c_char) -> c_int,
    render_create: unsafe extern "C" fn(*mut Handle, Handle, *mut RenderParam) -> c_int,
    render_callback: unsafe extern "C" fn(Handle, Option<Notify>, *mut c_void),
    render_update: unsafe extern "C" fn(Handle) -> u64,
    render_get_info: unsafe extern "C" fn(Handle, RenderParam) -> c_int,
    render_frame: unsafe extern "C" fn(Handle, *mut RenderParam) -> c_int,
    render_free: unsafe extern "C" fn(Handle),
    _library: Library,
}

impl Api {
    fn load() -> Result<Self> {
        let directory = executable_dir()?;
        let path = ["libmpv-2.dll", "mpv-2.dll"]
            .into_iter()
            .map(|name| directory.join(name))
            .find(|path| path.is_file())
            .context(tr(Text::MpvDllMissing))?;
        // Load the exact DLL beside the app and resolve its dependencies there or in System32.
        let library = unsafe {
            Library::load_with_flags(
                &path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        }
        .with_context(|| format!("{} ({})", tr(Text::MpvDllLoadFailed), path.display()))?;
        macro_rules! symbol {
            ($name:literal) => {
                *unsafe { library.get(concat!($name, "\0").as_bytes()) }.context($name)?
            };
        }
        Ok(Self {
            api_version: symbol!("mpv_client_api_version"),
            create: symbol!("mpv_create"),
            initialize: symbol!("mpv_initialize"),
            destroy: symbol!("mpv_terminate_destroy"),
            error_string: symbol!("mpv_error_string"),
            set_option: symbol!("mpv_set_option_string"),
            observe: symbol!("mpv_observe_property"),
            get_property_async: symbol!("mpv_get_property_async"),
            command_async: symbol!("mpv_command_async"),
            wait_event: symbol!("mpv_wait_event"),
            wakeup_callback: symbol!("mpv_set_wakeup_callback"),
            request_log_messages: symbol!("mpv_request_log_messages"),
            render_create: symbol!("mpv_render_context_create"),
            render_callback: symbol!("mpv_render_context_set_update_callback"),
            render_update: symbol!("mpv_render_context_update"),
            render_get_info: symbol!("mpv_render_context_get_info"),
            render_frame: symbol!("mpv_render_context_render"),
            render_free: symbol!("mpv_render_context_free"),
            _library: library,
        })
    }

    fn error(&self, code: c_int) -> String {
        unsafe { CStr::from_ptr((self.error_string)(code)) }
            .to_string_lossy()
            .into_owned()
    }

    fn check(&self, code: c_int) -> Result<()> {
        if code < 0 {
            bail!("{} ({code})", self.error(code));
        }
        Ok(())
    }
}

struct Core {
    api: Api,
    handle: NonNull<c_void>,
    render: Option<NonNull<c_void>>,
}

impl Core {
    fn new(options: &[(&CStr, &CStr)]) -> Result<Self> {
        let api = Api::load()?;
        let version = unsafe { (api.api_version)() };
        ensure!(version >> 16 == 2, tr(Text::MpvApiVersionRequired));
        let handle = NonNull::new(unsafe { (api.create)() }).context(tr(Text::MpvCreateFailed))?;
        let core = Self {
            api,
            handle,
            render: None,
        };
        let raw = handle.as_ptr();
        for (name, value) in options {
            core.api
                .check(unsafe { (core.api.set_option)(raw, name.as_ptr(), value.as_ptr()) })
                .with_context(|| format!("{} {}", tr(Text::MpvOption), name.to_string_lossy()))?;
        }
        core.api
            .check(unsafe { (core.api.initialize)(raw) })
            .context(tr(Text::MpvInitializeFailed))?;
        Ok(core)
    }

    fn create_renderer(&mut self, params: &mut [RenderParam]) -> Result<Handle> {
        let mut render = ptr::null_mut();
        self.api.check(unsafe {
            (self.api.render_create)(&mut render, self.handle.as_ptr(), params.as_mut_ptr())
        })?;
        self.render = Some(NonNull::new(render).context(tr(Text::MpvRenderHandleMissing))?);
        Ok(render)
    }

    fn send(&self, args: &[&str], id: u64) -> Result<()> {
        // An advanced-control render thread must use async client commands.
        let args: Vec<CString> = args
            .iter()
            .map(|arg| CString::new(*arg))
            .collect::<Result<_, _>>()?;
        let mut argv: Vec<_> = args.iter().map(|arg| arg.as_ptr()).collect();
        argv.push(ptr::null());
        self.api
            .check(unsafe { (self.api.command_async)(self.handle.as_ptr(), id, argv.as_ptr()) })
    }

    fn render_frame(&self, params: &mut [RenderParam]) -> Result<()> {
        self.api.check(unsafe {
            (self.api.render_frame)(self.render.unwrap().as_ptr(), params.as_mut_ptr())
        })
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        unsafe {
            if let Some(render) = self.render {
                (self.api.render_free)(render.as_ptr());
            }
            (self.api.destroy)(self.handle.as_ptr());
        }
    }
}

struct Wakeups {
    proxy: EventLoopProxy<AppEvent>,
    media_pending: AtomicBool,
    video_pending: AtomicBool,
}

unsafe extern "C" fn media_wakeup(data: *mut c_void) {
    let wakeups = unsafe { &*data.cast::<Wakeups>() };
    if !wakeups.media_pending.swap(true, Ordering::AcqRel) {
        let _ = wakeups.proxy.send_event(AppEvent::Media);
    }
}

unsafe extern "C" fn video_wakeup(data: *mut c_void) {
    let wakeups = unsafe { &*data.cast::<Wakeups>() };
    if !wakeups.video_pending.swap(true, Ordering::AcqRel) {
        let _ = wakeups.proxy.send_event(AppEvent::Video);
    }
}

unsafe extern "C" fn gl_proc(display: *mut c_void, name: *const c_char) -> *mut c_void {
    let display = unsafe { &*display.cast::<Display>() };
    display
        .get_proc_address(unsafe { CStr::from_ptr(name) })
        .cast_mut()
}

#[derive(Clone, PartialEq)]
pub struct Playback {
    pub file: Option<Arc<Path>>,
    pub nearby: NearbyVideos,
    pub error: Option<Arc<str>>,
    title: Option<Arc<str>>,
    generation: u64,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    pub volume: f64,
    pub muted: bool,
    pub seekable: bool,
    pub loaded: bool,
    pub loading: bool,
    pub eof: bool,
    width: i64,
    height: i64,
    rotation: i64,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            file: None,
            nearby: NearbyVideos::default(),
            error: None,
            title: None,
            generation: 0,
            position: None,
            duration: None,
            paused: false,
            volume: 75.0,
            muted: false,
            seekable: false,
            loaded: false,
            loading: false,
            eof: false,
            width: 0,
            height: 0,
            rotation: 0,
        }
    }
}

impl Playback {
    pub fn set_error(&mut self, error: String) {
        if self.error.as_deref() != Some(error.as_str()) {
            self.error = Some(Arc::from(error));
        }
    }

    pub fn dismiss_error(&mut self, error: &Arc<str>) -> bool {
        if self
            .error
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, error))
        {
            self.error = None;
            true
        } else {
            false
        }
    }

    pub fn begin_load(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.title = None;
        self.loaded = false;
        self.loading = true;
        self.position = None;
        self.duration = None;
        self.eof = false;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn title(&self) -> Option<Cow<'_, str>> {
        self.title.as_deref().map(Cow::Borrowed).or_else(|| {
            let file = self.file.as_deref()?;
            Some(
                file.file_name()
                    .unwrap_or(file.as_os_str())
                    .to_string_lossy(),
            )
        })
    }

    pub fn dimensions(&self) -> Option<[u32; 2]> {
        if !self.loaded || self.width <= 0 || self.height <= 0 {
            return None;
        }
        let mut extent = [
            u32::try_from(self.width).ok()?,
            u32::try_from(self.height).ok()?,
        ];
        if self.rotation.rem_euclid(180) == 90 {
            extent.swap(0, 1);
        }
        Some(extent)
    }
}

#[derive(Default)]
pub struct MediaChange {
    pub clear_frame: bool,
    pub issue: Option<String>,
}

pub struct Mpv {
    core: Core,
    wakeups: Box<Wakeups>,
    display: Box<Display>,
}

impl Mpv {
    // The desktop WGL context must be current during construction, rendering and drop.
    pub fn new(
        display: Display,
        proxy: EventLoopProxy<AppEvent>,
        initial_volume: u8,
    ) -> Result<Self> {
        let volume = CString::new(initial_volume.to_string())?;
        let core = Core::new(&[
            (c"config", c"no"),
            (c"vo", c"libmpv"),
            (c"idle", c"yes"),
            (c"keep-open", c"yes"),
            (c"hwdec", c"auto"),
            (c"osc", c"no"),
            (c"osd-level", c"0"),
            (c"volume", volume.as_c_str()),
            (c"volume-max", c"100"),
            (c"video-timing-offset", c"0.005"),
            (c"target-prim", c"bt.709"),
            (c"target-trc", c"srgb"),
        ])?;
        let mut mpv = Self {
            core,
            wakeups: Box::new(Wakeups {
                proxy,
                media_pending: AtomicBool::new(false),
                video_pending: AtomicBool::new(false),
            }),
            display: Box::new(display),
        };
        let raw = mpv.core.handle.as_ptr();
        let api = &mpv.core.api;
        let log_level = if log::log_enabled!(log::Level::Debug) {
            c"v"
        } else {
            c"warn"
        };
        api.check(unsafe { (api.request_log_messages)(raw, log_level.as_ptr()) })?;
        for &(id, name, format) in PROPERTIES {
            api.check(unsafe { (api.observe)(raw, id, name.as_ptr(), format) })
                .with_context(|| format!("{} {}", tr(Text::MpvProperty), name.to_string_lossy()))?;
        }

        let mut gl_init = OpenGlInit {
            get_proc_address: gl_proc,
            context: ptr::from_mut(mpv.display.as_mut()).cast(),
        };
        let mut advanced: c_int = 1;
        let render = mpv
            .core
            .create_renderer(&mut [
                RenderParam::string(ParamType::ApiType, c"opengl"),
                RenderParam::new(ParamType::OpenGlInit, &mut gl_init),
                RenderParam::new(ParamType::AdvancedControl, &mut advanced),
                RenderParam::END,
            ])
            .context(tr(Text::MpvGlRendererInitFailed))?;
        let wakeups = ptr::from_mut(mpv.wakeups.as_mut()).cast();
        unsafe {
            (mpv.core.api.wakeup_callback)(raw, Some(media_wakeup), wakeups);
            (mpv.core.api.render_callback)(render, Some(video_wakeup), wakeups);
        }
        Ok(mpv)
    }

    pub fn open(&self, path: &Path) -> Result<()> {
        let path = path.to_str().context(tr(Text::PathNotUtf8))?;
        self.core.send(&["loadfile", path, "replace"], LOAD_COMMAND)
    }

    pub fn command(&self, args: &[&str]) -> Result<()> {
        self.core.send(args, 0)
    }

    pub fn poll(&self, playback: &mut Playback) -> MediaChange {
        self.wakeups.media_pending.store(false, Ordering::Release);
        let mut change = MediaChange::default();
        let api = &self.core.api;
        let handle = self.core.handle.as_ptr();
        loop {
            // A zero timeout is safe on the libmpv render thread.
            let event = unsafe { &*(api.wait_event)(handle, 0.0) };
            match event.id {
                EVENT_NONE => break,
                EVENT_START_FILE => {
                    change.clear_frame = true;
                    playback.begin_load();
                    log::info!("libmpv START_FILE");
                }
                EVENT_FILE_LOADED => {
                    playback.loading = false;
                    playback.loaded = true;
                    log::info!("libmpv FILE_LOADED");
                    // Query each file's title even if two consecutive files have the same title.
                    unsafe {
                        (api.get_property_async)(
                            handle,
                            playback.generation,
                            c"media-title".as_ptr(),
                            FORMAT_STRING,
                        );
                    }
                }
                EVENT_GET_PROPERTY_REPLY
                    if event.user_data == playback.generation
                        && playback.loaded
                        && event.error >= 0
                        && !event.data.is_null() =>
                {
                    let property = unsafe { &*event.data.cast::<Property>() };
                    playback.title = property.title();
                }
                EVENT_END_FILE if !event.data.is_null() => {
                    let end = unsafe { &*event.data.cast::<EndFile>() };
                    if end.reason == END_ERROR {
                        playback.loading = false;
                        playback.loaded = false;
                        change.clear_frame = true;
                        change.issue = Some(format!(
                            "{} ({})",
                            tr(Text::VideoPlaybackFailed),
                            api.error(end.error)
                        ));
                    }
                }
                EVENT_PROPERTY_CHANGE if !event.data.is_null() => {
                    let property = unsafe { &*event.data.cast::<Property>() };
                    match event.user_data {
                        POSITION => playback.position = property.number(),
                        DURATION => {
                            playback.duration = property.number().filter(|value| *value > 0.0)
                        }
                        PAUSE => playback.paused = property.flag(),
                        VOLUME => playback.volume = property.number().unwrap_or(playback.volume),
                        MUTE => playback.muted = property.flag(),
                        SEEKABLE => playback.seekable = property.flag(),
                        WIDTH => playback.width = property.integer(),
                        HEIGHT => playback.height = property.integer(),
                        ROTATION => playback.rotation = property.integer(),
                        EOF_REACHED => playback.eof = property.flag(),
                        _ => {}
                    }
                }
                EVENT_COMMAND_REPLY if event.error < 0 => {
                    if event.user_data == LOAD_COMMAND {
                        playback.loading = false;
                    }
                    change.issue = Some(format!(
                        "{} ({})",
                        tr(Text::PlaybackCommandFailed),
                        api.error(event.error)
                    ));
                }
                EVENT_LOG_MESSAGE if !event.data.is_null() => {
                    let message = unsafe { &*event.data.cast::<LogMessage>() };
                    let prefix = unsafe { CStr::from_ptr(message.prefix) }.to_string_lossy();
                    let text = unsafe { CStr::from_ptr(message.text) }.to_string_lossy();
                    if message.log_level <= 30 {
                        log::warn!("libmpv [{prefix}] {}", text.trim_end());
                    } else {
                        log::debug!("libmpv [{prefix}] {}", text.trim_end());
                    }
                }
                EVENT_QUEUE_OVERFLOW => log::warn!("libmpv event queue overflow"),
                _ => {}
            }
        }
        change
    }

    pub fn update_video(&self) -> bool {
        self.wakeups.video_pending.store(false, Ordering::Release);
        let render = self.core.render.unwrap().as_ptr();
        unsafe { (self.core.api.render_update)(render) & 1 != 0 }
    }

    pub fn render(&self, target: Option<RenderTarget>) -> Result<()> {
        let mut flip_y: c_int = 1;
        let mut wait: c_int = 0;
        let mut skip: c_int = c_int::from(target.is_none());
        let mut target = target.unwrap_or(RenderTarget {
            fbo: 0,
            width: 0,
            height: 0,
            internal_format: 0,
        });
        self.core.render_frame(&mut [
            RenderParam::new(ParamType::Framebuffer, &mut target),
            RenderParam::new(ParamType::FlipY, &mut flip_y),
            RenderParam::new(ParamType::BlockForTargetTime, &mut wait),
            RenderParam::new(ParamType::SkipRendering, &mut skip),
            RenderParam::END,
        ])
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        unsafe {
            (self.core.api.wakeup_callback)(self.core.handle.as_ptr(), None, ptr::null_mut());
            if let Some(render) = self.core.render {
                (self.core.api.render_callback)(render.as_ptr(), None, ptr::null_mut());
            }
        }
    }
}
