//! Background work for the Home screen and project view: a small thread pool, the project scanner and the
//! thumbnail cache. Nothing here blocks the UI thread; results arrive in a shared list that [`Thumbs::poll`]
//! and [`Scans::poll`] drain once per frame.

use crate::calls::projects::{self, Project, Thumb};
use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

type Job = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Queue {
    jobs: Vec<Job>,
    closed: bool,
}

/// Worker threads taking the most recently queued job first, so what is on screen loads before what was scrolled
/// past. Threads start on first use and end when the pool is dropped.
struct Pool {
    shared: Arc<(Mutex<Queue>, Condvar)>,
    workers: usize,
    started: bool,
}

impl Pool {
    fn new(workers: usize) -> Pool {
        Pool { shared: Arc::default(), workers, started: false }
    }

    fn push(&mut self, job: Job) {
        if !self.started {
            self.started = true;
            for _ in 0..self.workers {
                let shared = self.shared.clone();
                std::thread::spawn(move || {
                    loop {
                        let job = {
                            let (lock, cv) = &*shared;
                            let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
                            loop {
                                if let Some(job) = q.jobs.pop() {
                                    break Some(job);
                                }
                                if q.closed {
                                    break None;
                                }
                                q = cv.wait(q).unwrap_or_else(|e| e.into_inner());
                            }
                        };
                        match job {
                            Some(job) => job(),
                            None => return,
                        }
                    }
                });
            }
        }
        let (lock, cv) = &*self.shared;
        lock.lock().unwrap_or_else(|e| e.into_inner()).jobs.push(job);
        cv.notify_one();
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        let (lock, cv) = &*self.shared;
        let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
        q.closed = true;
        q.jobs.clear();
        cv.notify_all();
    }
}

// --- Project scans -----------------------------------------------------------------------------------------

/// What is known about one project folder.
#[derive(Clone, Default)]
pub struct ScanEntry {
    /// The last good scan. Kept while a newer one loads, so the view does not flicker.
    pub project: Option<Arc<Project>>,
    pub error: Option<String>,
    pub loading: bool,
}

type ScanResult = (PathBuf, Result<Project, String>);

pub struct Scans {
    entries: HashMap<PathBuf, ScanEntry>,
    pool: Pool,
    done: Arc<Mutex<Vec<ScanResult>>>,
    /// Bumped by [`Scans::poll`] whenever scans arrived, so views know to refresh derived data.
    pub generation: u64,
}

impl Default for Scans {
    fn default() -> Scans {
        Scans { entries: HashMap::new(), pool: Pool::new(1), done: Arc::default(), generation: 0 }
    }
}

impl Scans {
    pub fn get(&self, folder: &Path) -> Option<&ScanEntry> {
        self.entries.get(folder)
    }

    /// The last good scan of `folder`, if any.
    pub fn project(&self, folder: &Path) -> Option<Arc<Project>> {
        self.entries.get(folder).and_then(|e| e.project.clone())
    }

    /// Scans `folder` in the background (again, if it was scanned before).
    pub fn request(&mut self, ctx: &egui::Context, folder: &Path) {
        let e = self.entries.entry(folder.to_path_buf()).or_default();
        if e.loading {
            return;
        }
        e.loading = true;
        let (folder, done, ctx) = (folder.to_path_buf(), self.done.clone(), ctx.clone());
        self.pool.push(Box::new(move || {
            let result = projects::scan_project(&folder);
            done.lock().unwrap_or_else(|e| e.into_inner()).push((folder, result));
            ctx.request_repaint();
        }));
    }

    /// Changes the cached scan of `folder` in place, so the view shows an edit before the next scan arrives.
    pub fn update(&mut self, folder: &Path, edit: impl FnOnce(&mut Project)) {
        if let Some(p) = self.entries.get_mut(folder).and_then(|e| e.project.as_mut()) {
            edit(Arc::make_mut(p));
        }
    }

    /// Scans `folder` unless it has been scanned already.
    pub fn ensure(&mut self, ctx: &egui::Context, folder: &Path) {
        if !self.entries.contains_key(folder) {
            self.request(ctx, folder);
        }
    }

    /// Some scan is still running.
    pub fn any_loading(&self) -> bool {
        self.entries.values().any(|e| e.loading)
    }

    /// Takes finished scans in. Returns whether anything arrived.
    pub fn poll(&mut self) -> bool {
        let finished: Vec<ScanResult> = std::mem::take(&mut *self.done.lock().unwrap_or_else(|e| e.into_inner()));
        let any = !finished.is_empty();
        for (folder, result) in finished {
            let e = self.entries.entry(folder).or_default();
            e.loading = false;
            match result {
                Ok(p) => (e.project, e.error) = (Some(Arc::new(p)), None),
                Err(msg) => e.error = Some(msg),
            }
        }
        if any {
            self.generation += 1;
        }
        any
    }
}

// --- Thumbnails --------------------------------------------------------------------------------------------

/// A thumbnail is identified by its file and the file's state, so a saved canvas gets a fresh one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThumbKey {
    pub path: PathBuf,
    pub modified: u64,
    pub size: u64,
}

impl ThumbKey {
    /// The key for a file as it is on disk now.
    pub fn of(path: &Path) -> ThumbKey {
        ThumbKey {
            path: path.to_path_buf(),
            modified: projects::modified_secs(path),
            size: path.metadata().map_or(0, |m| m.len()),
        }
    }
}

pub enum ThumbState {
    Loading,
    Ready { texture: egui::TextureHandle, width: u32, height: u32 },
    Failed,
}

struct ThumbEntry {
    state: ThumbState,
    last_used: u64,
}

/// The largest side requested from the core. Cards show it at most this big at 1x.
pub const THUMB_SIDE: u32 = 256;
/// Textures kept; the least recently drawn go first.
const CACHE_LIMIT: usize = 700;
const CACHE_TRIM_TO: usize = 500;
/// Textures uploaded per frame, so a burst of finished thumbnails does not stall one frame.
const UPLOADS_PER_FRAME: usize = 24;

type ThumbResult = (ThumbKey, Result<Thumb, String>);

pub struct Thumbs {
    entries: HashMap<ThumbKey, ThumbEntry>,
    pool: Pool,
    done: Arc<Mutex<Vec<ThumbResult>>>,
    clock: u64,
}

impl Default for Thumbs {
    fn default() -> Thumbs {
        Thumbs { entries: HashMap::new(), pool: Pool::new(3), done: Arc::default(), clock: 0 }
    }
}

impl Thumbs {
    /// The state of a thumbnail, starting its load the first time it is asked for. Call it only for what is
    /// drawn: it also marks the entry as recently used.
    pub fn get(&mut self, ctx: &egui::Context, key: &ThumbKey) -> &ThumbState {
        let clock = self.clock;
        if !self.entries.contains_key(key) {
            let (job_key, done, ctx) = (key.clone(), self.done.clone(), ctx.clone());
            self.pool.push(Box::new(move || {
                let result = projects::file_thumbnail(&job_key.path, THUMB_SIDE);
                done.lock().unwrap_or_else(|e| e.into_inner()).push((job_key, result));
                ctx.request_repaint();
            }));
            self.entries.insert(key.clone(), ThumbEntry { state: ThumbState::Loading, last_used: clock });
        }
        let e = self.entries.get_mut(key).expect("inserted above");
        e.last_used = clock;
        &e.state
    }

    /// Uploads finished thumbnails as textures and trims the cache. Call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.clock += 1;
        let mut finished = self.done.lock().unwrap_or_else(|e| e.into_inner());
        let take = finished.len().min(UPLOADS_PER_FRAME);
        let batch: Vec<ThumbResult> = finished.drain(..take).collect();
        let more = !finished.is_empty();
        drop(finished);
        for (key, result) in batch {
            let Some(entry) = self.entries.get_mut(&key) else { continue };
            entry.state = match result {
                Ok(t) if t.width > 0 && t.height > 0 && t.rgba.len() == (t.width * t.height * 4) as usize => {
                    let size = [t.width as usize, t.height as usize];
                    ThumbState::Ready {
                        texture: ctx.load_texture(
                            "project-thumb",
                            egui::ColorImage::from_rgba_unmultiplied(size, &t.rgba),
                            egui::TextureOptions::NEAREST,
                        ),
                        width: t.width,
                        height: t.height,
                    }
                }
                _ => ThumbState::Failed,
            };
        }
        if more {
            ctx.request_repaint();
        }
        if self.entries.len() > CACHE_LIMIT {
            let mut by_age: Vec<(u64, ThumbKey)> = self
                .entries
                .iter()
                .filter(|(_, e)| !matches!(e.state, ThumbState::Loading))
                .map(|(k, e)| (e.last_used, k.clone()))
                .collect();
            by_age.sort_by_key(|(used, _)| *used);
            let drop_count = self.entries.len().saturating_sub(CACHE_TRIM_TO);
            for (_, key) in by_age.into_iter().take(drop_count) {
                self.entries.remove(&key);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Thumbnails still being produced.
    pub fn loading(&self) -> usize {
        self.entries.values().filter(|e| matches!(e.state, ThumbState::Loading)).count()
    }
}
