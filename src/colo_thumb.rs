//! A small worker pool that fetches 16colo.rs's pre-rendered thumbnail PNGs off the
//! UI thread. It mirrors [`crate::thumb::ThumbBuilder`] — a LIFO stack (so the most
//! recently scrolled-into-view piece downloads first), per-path dedup, and results
//! over an `mpsc` channel — but the job is an HTTPS GET + PNG decode instead of a
//! local-file decode. Results are keyed by the piece's *virtual* display path, so the
//! grid/table upload them into `thumb_tex` exactly like a locally-decoded thumbnail.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

pub struct RemoteThumbResult {
    pub path: PathBuf, // the piece's virtual display path (the cache key)
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>, // width * height * 4
}

struct Job {
    path: PathBuf,
    url: String,
    target: u32,
}

pub struct RemoteThumbs {
    queue: Arc<(Mutex<Vec<Job>>, Condvar)>,
    results: Receiver<RemoteThumbResult>,
    requested: HashSet<PathBuf>,
}

impl RemoteThumbs {
    pub fn new(workers: usize) -> Self {
        let queue: Arc<(Mutex<Vec<Job>>, Condvar)> =
            Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let (tx, rx): (Sender<RemoteThumbResult>, Receiver<RemoteThumbResult>) = channel();

        for _ in 0..workers.max(1) {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            std::thread::spawn(move || loop {
                let job = {
                    let (lock, cvar) = &*queue;
                    let mut q = lock.lock().unwrap();
                    while q.is_empty() {
                        q = cvar.wait(q).unwrap();
                    }
                    // LIFO: the most-recently-requested (visible) thumbnail first.
                    q.pop().unwrap()
                };
                if let Some(res) = fetch(&job) {
                    let _ = tx.send(res);
                }
            });
        }

        Self {
            queue,
            results: rx,
            requested: HashSet::new(),
        }
    }

    /// Enqueue once per path. Cheap to call every frame for visible rows.
    pub fn request(&mut self, path: &Path, url: &str, target: u32) {
        if self.requested.insert(path.to_path_buf()) {
            let (lock, cvar) = &*self.queue;
            lock.lock().unwrap().push(Job {
                path: path.to_path_buf(),
                url: url.to_string(),
                target,
            });
            cvar.notify_one();
        }
    }

    pub fn drain(&self) -> Vec<RemoteThumbResult> {
        self.results.try_iter().collect()
    }
}

/// Download + decode one thumbnail PNG, area-downscaling if it's bigger than `target`.
/// The PNG is immutable (a pre-rendered preview), so it goes through the persistent
/// disk cache — re-browsing a pack/artist doesn't re-fetch it.
fn fetch(job: &Job) -> Option<RemoteThumbResult> {
    let buf = crate::cache::get_bytes(&job.url, None).ok()?;
    let img = image::load_from_memory(&buf).ok()?.to_rgba8();
    let (sw, sh) = (img.width() as usize, img.height() as usize);
    if sw == 0 || sh == 0 {
        return None;
    }
    let rgba = img.into_raw();
    let target = job.target.max(1) as usize;
    // The `tn` previews are ~180px wide, usually already ≤ target; only downscale a
    // larger render. Box-average (not nearest) so a 50% dither isn't aliased to noise.
    let (w, h, rgba) = if sw.max(sh) > target {
        let scale = target as f32 / sw.max(sh) as f32;
        let dw = ((sw as f32 * scale).round() as usize).max(1);
        let dh = ((sh as f32 * scale).round() as usize).max(1);
        (dw, dh, crate::thumb::box_downscale(&rgba, sw, sh, dw, dh))
    } else {
        (sw, sh, rgba)
    };
    Some(RemoteThumbResult {
        path: job.path.clone(),
        width: w,
        height: h,
        rgba,
    })
}
