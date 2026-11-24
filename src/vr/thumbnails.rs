use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::thread;
use std::time::SystemTime;

use egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions};

use crate::files::Entry;
use crate::mpv::thumbnail::{self, Thumbnail};

const CAPACITY: usize = 64;

struct FileKey {
    path: Arc<Path>,
    stamp: (Option<SystemTime>, Option<u64>),
}

struct Worker {
    requests: SyncSender<Arc<Path>>,
    results: Receiver<Option<Thumbnail>>,
    pending: Option<FileKey>,
    cancelled: Arc<AtomicBool>,
}

impl Worker {
    fn spawn() -> io::Result<Self> {
        let (requests, jobs) = mpsc::sync_channel::<Arc<Path>>(1);
        let (responses, results) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancelled);
        thread::Builder::new()
            .name("video-thumbnails".to_owned())
            .spawn(move || {
                while let Ok(path) = jobs.recv() {
                    let frame = match thumbnail::decode(&path, &stop) {
                        Ok(frame) => Some(frame),
                        Err(error) => {
                            log::debug!("Thumbnail failed for {}: {error:#}", path.display());
                            None
                        }
                    };
                    if responses.send(frame).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests,
            results,
            pending: None,
            cancelled,
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub(super) struct Thumbnails {
    worker: Option<Worker>,
    unavailable: bool,
    // Oldest access first. Missing previews are cached too so bad files do not keep decoding.
    cache: VecDeque<(FileKey, Option<TextureHandle>)>,
}

impl Thumbnails {
    pub fn poll(&mut self, context: &Context) -> bool {
        let Some(worker) = &mut self.worker else {
            return false;
        };
        let frame = match worker.results.try_recv() {
            Ok(frame) => frame,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.unavailable = true;
                self.worker = None;
                return false;
            }
        };
        if let Some(file) = worker.pending.take() {
            if self.cache.len() == CAPACITY {
                self.cache.pop_front();
            }
            let image = frame.map(|frame| {
                context.load_texture(
                    "video thumbnail",
                    ColorImage::from_rgba_unmultiplied(thumbnail::SIZE, frame.rgba()),
                    TextureOptions::LINEAR,
                )
            });
            self.cache.push_back((file, image));
        }
        // Redraw after each reply so another visible tile can request a preview.
        true
    }

    pub fn texture(&mut self, entry: &Entry) -> Option<TextureId> {
        let stamp = entry.file_stamp();
        if let Some(index) = self
            .cache
            .iter()
            .position(|(file, _)| file.path.as_ref() == entry.path.as_path() && file.stamp == stamp)
        {
            let cached = self.cache.remove(index).unwrap();
            let texture = cached.1.as_ref().map(TextureHandle::id);
            self.cache.push_back(cached);
            return texture;
        }
        if self.unavailable {
            return None;
        }
        if self.worker.is_none() {
            match Worker::spawn() {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    log::warn!("Could not start the thumbnail worker: {error}");
                    self.unavailable = true;
                }
            }
        }
        if let Some(worker) = &mut self.worker
            && worker.pending.is_none()
        {
            let path: Arc<Path> = Arc::from(entry.path.as_path());
            if worker.requests.try_send(Arc::clone(&path)).is_ok() {
                worker.pending = Some(FileKey { path, stamp });
            }
        }
        None
    }
}
