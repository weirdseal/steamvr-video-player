use std::ffi::c_int;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

use super::{
    Core, EVENT_COMMAND_REPLY, EVENT_END_FILE, EVENT_FILE_LOADED, EVENT_NONE, LOAD_COMMAND,
    ParamType, RenderParam,
};
use crate::i18n::{Text, tr};

pub const SIZE: [usize; 2] = [256, 144];
const STRIDE: usize = SIZE[0] * 4;
const BUFFER_SIZE: usize = STRIDE * SIZE[1];
const ALIGNMENT: usize = 64;
const DECODE_TIMEOUT: Duration = Duration::from_secs(3);
const FRAME_PRESENT: u64 = 1;
const FRAME_REDRAW: u64 = 2;

pub struct Thumbnail {
    buffer: Vec<u8>,
    first_pixel: usize,
}

impl Thumbnail {
    pub fn rgba(&self) -> &[u8] {
        &self.buffer[self.first_pixel..self.first_pixel + BUFFER_SIZE]
    }
}

#[repr(C)]
#[derive(Default)]
struct FrameInfo {
    flags: u64,
    target_time: i64,
}

pub fn decode(path: &Path, cancelled: &AtomicBool) -> Result<Thumbnail> {
    ensure!(!cancelled.load(Ordering::Relaxed), "Thumbnail cancelled");
    let path = path.to_str().context(tr(Text::PathNotUtf8))?;
    // A fresh core for each request keeps queued frames and playback events file-local.
    let mut core = Core::new(&[
        (c"config", c"no"),
        (c"vo", c"libmpv"),
        (c"idle", c"yes"),
        (c"pause", c"yes"),
        (c"keep-open", c"yes"),
        (c"aid", c"no"),
        (c"sid", c"no"),
        (c"osc", c"no"),
        (c"osd-level", c"0"),
        (c"hwdec", c"no"),
        (c"vd-lavc-threads", c"2"),
        (c"hr-seek", c"no"),
        (c"start", c"10%"),
        (c"access-references", c"no"),
        (c"autoload-files", c"no"),
        (c"load-scripts", c"no"),
        (c"ytdl", c"no"),
        (c"demuxer-readahead-secs", c"0.2"),
    ])?;
    let render = core.create_renderer(&mut [
        RenderParam::string(ParamType::ApiType, c"sw"),
        RenderParam::END,
    ])?;
    core.send(&["loadfile", path, "replace"], LOAD_COMMAND)?;

    let deadline = Instant::now() + DECODE_TIMEOUT;
    let mut loaded = false;
    while !cancelled.load(Ordering::Relaxed) && Instant::now() < deadline {
        let event = unsafe { &*(core.api.wait_event)(core.handle.as_ptr(), 0.0) };
        match event.id {
            EVENT_FILE_LOADED => loaded = true,
            EVENT_END_FILE => bail!("File ended before a thumbnail frame was decoded"),
            EVENT_COMMAND_REPLY if event.user_data == LOAD_COMMAND => {
                core.api.check(event.error)?
            }
            _ => {}
        }
        if loaded {
            let mut next = FrameInfo::default();
            core.api.check(unsafe {
                (core.api.render_get_info)(
                    render,
                    RenderParam::new(ParamType::NextFrameInfo, &mut next),
                )
            })?;
            if next.flags & (FRAME_PRESENT | FRAME_REDRAW) == FRAME_PRESENT {
                return capture(&core);
            }
        }
        if event.id == EVENT_NONE {
            thread::sleep(Duration::from_millis(8));
        }
    }
    bail!("No video frame available for the thumbnail")
}

fn capture(core: &Core) -> Result<Thumbnail> {
    let mut buffer = vec![0_u8; BUFFER_SIZE + ALIGNMENT - 1];
    let first_pixel = buffer.as_mut_ptr().align_offset(ALIGNMENT);
    let pixels = &mut buffer[first_pixel..first_pixel + BUFFER_SIZE];
    let mut size = [SIZE[0] as c_int, SIZE[1] as c_int];
    let mut stride = STRIDE;
    let mut wait: c_int = 0;
    core.render_frame(&mut [
        RenderParam::new(ParamType::SoftwareSize, &mut size),
        RenderParam::string(ParamType::SoftwareFormat, c"rgb0"),
        RenderParam::new(ParamType::SoftwareStride, &mut stride),
        RenderParam {
            kind: ParamType::SoftwarePointer,
            data: pixels.as_mut_ptr().cast(),
        },
        RenderParam::new(ParamType::BlockForTargetTime, &mut wait),
        RenderParam::END,
    ])?;
    // The software renderer only defines RGB in rgb0. egui needs opaque RGBA.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel[3] = u8::MAX;
    }
    Ok(Thumbnail {
        buffer,
        first_pixel,
    })
}
