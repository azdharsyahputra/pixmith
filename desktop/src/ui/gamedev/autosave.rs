//! Autosave and crash recovery (`Autosaver` in `FileActions.swift`).
//!
//! While a document has unsaved changes, a copy goes to the Recovery folder every minute
//! (`PIXMITH_AUTOSAVE_INTERVAL` overrides the seconds): `<id>.pxmt` plus a JSON sidecar with the original
//! path, the tab's name and the date. Every open tab has its own copy. A clean exit, a save or closing the tab
//! removes the copy, so anything found there at launch is left over from a crash, and the app offers to
//! restore it.
//!
//! The copy is written on a worker thread from a snapshot, so a big document never stalls the window, and
//! through a temporary file, so a crash mid-write keeps the previous copy.

use super::paths;
use crate::app::PixmithApp;
use crate::doc::Doc;
use crate::ui::widgets::{self, DialogAction};
use eframe::egui;
use pixmith_core::formats::json::{self, Value};
use pixmith_core::formats::sheet::json_str;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// One tab's recovery bookkeeping.
struct Slot {
    /// The tab's document (the address of the core document).
    key: usize,
    /// Names the recovery files.
    id: String,
    /// Revision whose content is in the recovery file, if any.
    saved_revision: Option<u64>,
}

/// A recovery copy left behind by an earlier run.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub id: String,
    pub file: PathBuf,
    pub original: Option<String>,
    pub name: String,
    pub modified: SystemTime,
}

struct Written {
    id: String,
    revision: u64,
    error: Option<String>,
}

#[derive(Default)]
pub struct Autosaver {
    /// The recovery folder; `None` keeps autosave off (the default, so tests never write to it).
    dir: Option<PathBuf>,
    interval: Duration,
    next: Option<Instant>,
    slots: Vec<Slot>,
    in_flight: HashSet<String>,
    warned: bool,
    done: Arc<Mutex<Vec<Written>>>,
    /// Leftover copies to offer at launch, until the user answers.
    offer: Option<Vec<Candidate>>,
}

/// A fresh recovery id: unique per run and per tab, without needing a random source.
fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{nanos:x}-{:x}-{:x}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// "2026-10-24T15:45:09Z" for `t` (UTC), like `ISO8601DateFormatter`.
fn iso8601(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian calendar.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// How long ago a copy was written, for the restore question.
fn age(t: SystemTime) -> String {
    let secs = SystemTime::now().duration_since(t).map_or(0, |d| d.as_secs());
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => {
            let (y, m, d) = civil_from_days(t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64 / 86_400));
            const MONTHS: [&str; 12] =
                ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            format!("{} {d}, {y}", MONTHS[m as usize - 1])
        }
    }
}

impl Autosaver {
    /// Turns autosave on for the real app: recovery folder, timer, and a scan for copies a crash left behind.
    pub fn start(&mut self) {
        self.start_in(paths::recovery_dir());
    }

    pub fn start_in(&mut self, dir: PathBuf) {
        let seconds =
            std::env::var("PIXMITH_AUTOSAVE_INTERVAL").ok().and_then(|s| s.parse::<f64>().ok()).unwrap_or(60.0);
        self.interval = Duration::from_secs_f64(seconds.max(1.0));
        self.next = Some(Instant::now() + self.interval);
        let found = candidates(&dir, &self.slots.iter().map(|s| s.id.clone()).collect::<Vec<_>>());
        self.offer = (!found.is_empty()).then_some(found);
        self.dir = Some(dir);
    }

    pub fn offer(&self) -> Option<&[Candidate]> {
        self.offer.as_deref()
    }

    /// The recovery ids in use, in tab order.
    pub fn ids(&self) -> Vec<String> {
        self.slots.iter().map(|s| s.id.clone()).collect()
    }

    fn file(&self, id: &str, ext: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("{id}.{ext}")))
    }

    fn remove(&self, id: &str) {
        for ext in ["pxmt", "aseprite", "json"] {
            if let Some(f) = self.file(id, ext) {
                let _ = std::fs::remove_file(f);
            }
        }
    }

    /// Clean exit: no tab leaves a copy behind (`discardAll`).
    pub fn discard_all(&mut self) {
        for i in 0..self.slots.len() {
            let id = self.slots[i].id.clone();
            self.remove(&id);
            self.slots[i].saved_revision = None;
        }
    }

    /// One pass: keeps the slots in step with the tabs, drops copies of saved or closed documents, collects
    /// finished writes and, when the timer fires, starts the writes of tabs that changed since their copy.
    pub fn tick(app: &mut PixmithApp, ctx: &egui::Context) {
        if app.gamedev.autosave.dir.is_none() {
            return;
        }
        let keys: Vec<usize> = app.tabs.iter().map(|t| t.doc.raw() as usize).collect();
        let a = &mut app.gamedev.autosave;
        // Closed tabs take their copy with them.
        let gone: Vec<String> = a.slots.iter().filter(|s| !keys.contains(&s.key)).map(|s| s.id.clone()).collect();
        a.slots.retain(|s| keys.contains(&s.key));
        for id in gone {
            a.remove(&id);
        }
        for key in &keys {
            if !a.slots.iter().any(|s| s.key == *key) {
                a.slots.push(Slot { key: *key, id: new_id(), saved_revision: None });
            }
        }
        // Finished writes.
        let finished: Vec<Written> = std::mem::take(&mut *a.done.lock().unwrap_or_else(|e| e.into_inner()));
        for w in finished {
            a.in_flight.remove(&w.id);
            let tab = app.tabs.iter().find(|t| a.slots.iter().any(|s| s.id == w.id && s.key == t.doc.raw() as usize));
            match tab {
                // The tab was closed, replaced or saved while the copy was being written: it is stale now.
                Some(t) if t.doc.is_dirty() => {
                    if let Some(err) = w.error {
                        a.warn(&err, &mut app.error);
                    } else if let Some(s) = a.slots.iter_mut().find(|s| s.id == w.id) {
                        s.saved_revision = Some(w.revision);
                        a.warned = false;
                    }
                }
                _ => a.remove(&w.id),
            }
        }
        // A document that is clean again (saved, or undone back to the saved state) needs no copy.
        for tab in &app.tabs {
            let key = tab.doc.raw() as usize;
            if let Some(slot) = a.slots.iter_mut().find(|s| s.key == key)
                && !tab.doc.is_dirty()
                && slot.saved_revision.take().is_some()
            {
                let id = slot.id.clone();
                a.remove(&id);
            }
        }
        let Some(next) = a.next else { return };
        let now = Instant::now();
        if now >= next {
            a.next = Some(now + a.interval);
            let names: Vec<String> = (0..app.tabs.len()).map(|i| app.tab_name(i)).collect();
            let a = &mut app.gamedev.autosave;
            for (i, tab) in app.tabs.iter().enumerate() {
                let key = tab.doc.raw() as usize;
                let Some(slot) = a.slots.iter().find(|s| s.key == key) else { continue };
                if !tab.doc.is_dirty() || slot.saved_revision == Some(tab.revision) || a.in_flight.contains(&slot.id) {
                    continue;
                }
                let id = slot.id.clone();
                a.write(&id, &tab.doc, tab.revision, &names[i]);
            }
        }
        if let Some(next) = app.gamedev.autosave.next {
            ctx.request_repaint_after(next.saturating_duration_since(Instant::now()).max(Duration::from_millis(50)));
        }
    }

    /// Starts writing a copy of `doc` on a worker thread.
    fn write(&mut self, id: &str, doc: &Doc, revision: u64, name: &str) {
        let (Some(dir), Some(snapshot)) = (self.dir.clone(), doc.snapshot()) else { return };
        let (file, meta) = (self.file(id, "pxmt").unwrap_or_default(), self.file(id, "json").unwrap_or_default());
        let original = doc.path.as_ref().map_or("null".to_string(), |p| json_str(&p.to_string_lossy()));
        let sidecar = format!(
            "{{\"path\":{original},\"name\":{},\"date\":{}}}",
            json_str(name),
            json_str(&iso8601(SystemTime::now()))
        );
        let (id, done) = (id.to_string(), self.done.clone());
        self.in_flight.insert(id.clone());
        std::thread::spawn(move || {
            let result = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|()| snapshot.save(&file));
            if result.is_ok() {
                let tmp = meta.with_extension("json.tmp");
                if std::fs::write(&tmp, sidecar).is_ok() {
                    let _ = std::fs::rename(&tmp, &meta);
                }
            }
            drop(snapshot);
            done.lock().unwrap_or_else(|e| e.into_inner()).push(Written { id, revision, error: result.err() });
        });
    }

    /// Tells the user once that recovery copies cannot be written, so they do not rely on them.
    fn warn(&mut self, reason: &str, error: &mut Option<String>) {
        if self.warned {
            return;
        }
        self.warned = true;
        let folder = self.dir.as_ref().and_then(|d| d.parent()).map_or_else(String::new, |p| p.display().to_string());
        *error = Some(format!(
            "Autosave failed: {reason}\n\nUnsaved changes are not protected against a crash until this is fixed. \
             Free some disk space or check the permissions of {folder}, or save your work now."
        ));
    }
}

/// Recovery copies in `dir` that no open tab uses, newest first.
pub fn candidates(dir: &Path, open: &[String]) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<Candidate> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let file = e.path();
            // Copies from before `.pxmt` are `.aseprite`; both open.
            if !matches!(file.extension()?.to_str()?, "pxmt" | "aseprite") {
                return None;
            }
            let id = file.file_stem()?.to_string_lossy().into_owned();
            if open.contains(&id) {
                return None;
            }
            let meta = std::fs::read_to_string(file.with_extension("json")).ok().and_then(|t| json::parse(&t).ok());
            let field = |k: &str| meta.as_ref().and_then(|m| m.get(k)).and_then(Value::as_str).map(str::to_string);
            Some(Candidate {
                modified: e.metadata().and_then(|m| m.modified()).unwrap_or_else(|_| SystemTime::now()),
                original: field("path"),
                name: field("name").unwrap_or_else(|| "Untitled".into()),
                file,
                id,
            })
        })
        .collect();
    found.sort_by_key(|c| std::cmp::Reverse(c.modified));
    found
}

pub fn tick(app: &mut PixmithApp, ctx: &egui::Context) {
    Autosaver::tick(app, ctx);
}

/// The restore question at launch (`offerRecovery`). Returns whether it is showing.
pub fn show_offer(app: &mut PixmithApp, ctx: &egui::Context) -> bool {
    let Some(found) = app.gamedev.autosave.offer.clone() else { return false };
    let Some(newest) = found.first() else {
        app.gamedev.autosave.offer = None;
        return false;
    };
    let many = found.len() > 1;
    let title = if many {
        format!("Pixmith quit unexpectedly. Restore unsaved work in {} documents?", found.len())
    } else {
        format!("Pixmith quit unexpectedly. Restore the unsaved changes to \u{201C}{}\u{201D}?", newest.name)
    };
    let mut lines: Vec<String> =
        found.iter().take(8).map(|c| format!("{} \u{2013} {}", c.name, age(c.modified))).collect();
    if found.len() > 8 {
        lines.push("\u{2026}".into());
    }
    let action = widgets::dialog(ctx, "recovery", 420.0, &title, Some(&lines.join("\n")), |ui| {
        widgets::dialog_buttons(
            ui,
            Some(if many { "Discard All" } else { "Discard" }),
            Some("Not Now"),
            if many { "Restore All" } else { "Restore" },
            true,
        )
    });
    match action {
        DialogAction::Primary => {
            app.gamedev.autosave.offer = None;
            // Oldest first, so the most recent document ends up as the active tab.
            for c in found.iter().rev() {
                app.restore_recovered(c);
            }
        }
        DialogAction::Destructive => {
            app.gamedev.autosave.offer = None;
            for c in &found {
                app.gamedev.autosave.remove(&c.id);
            }
        }
        DialogAction::Cancel => app.gamedev.autosave.offer = None,
        DialogAction::None => {}
    }
    true
}

impl PixmithApp {
    /// Opens a recovery copy as a tab: unsaved, under its original file name when that file still exists.
    pub fn restore_recovered(&mut self, c: &Candidate) {
        let mut doc = match Doc::open(&c.file) {
            Ok(d) => d,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        doc.path = c.original.as_ref().map(PathBuf::from).filter(|p| p.exists());
        doc.touch();
        self.present(doc);
        // The tab keeps using the recovered copy, so there is no unprotected gap until the next autosave.
        let key = self.tab().doc.raw() as usize;
        let a = &mut self.gamedev.autosave;
        if a.dir.is_some() {
            match a.slots.iter_mut().find(|s| s.key == key) {
                Some(s) => (s.id, s.saved_revision) = (c.id.clone(), None),
                None => a.slots.push(Slot { key, id: c.id.clone(), saved_revision: None }),
            }
        }
    }

    /// Clean exit: removes every recovery copy (`terminateReply`).
    pub fn discard_recovery_copies(&mut self) {
        self.gamedev.autosave.discard_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pixmith-autosave-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn app_with_autosave(dir: &Path) -> PixmithApp {
        let mut app = PixmithApp::default();
        app.gamedev.autosave.start_in(dir.to_path_buf());
        app.gamedev.autosave.interval = Duration::from_millis(1);
        app
    }

    /// Runs `tick` until no write is in flight.
    fn settle(app: &mut PixmithApp) {
        let ctx = egui::Context::default();
        for _ in 0..400 {
            app.gamedev.autosave.next = Some(Instant::now());
            Autosaver::tick(app, &ctx);
            if app.gamedev.autosave.in_flight.is_empty() {
                // One more pass collects the result.
                Autosaver::tick(app, &ctx);
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("autosave did not finish");
    }

    fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|d| {
                d.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == ext)).collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn iso_dates() {
        assert_eq!(iso8601(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(UNIX_EPOCH + Duration::from_secs(1_791_042_309)), "2026-10-03T15:45:09Z");
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn clean_documents_are_not_written() {
        let dir = scratch("clean");
        let mut app = app_with_autosave(&dir);
        settle(&mut app);
        assert!(files(&dir, "pxmt").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_dirty_document_gets_a_copy_and_a_sidecar_that_go_when_it_is_saved() {
        let dir = scratch("dirty");
        let mut app = app_with_autosave(&dir);
        app.tab_mut().doc.touch();
        app.tab_mut().changed();
        settle(&mut app);
        let copies = files(&dir, "pxmt");
        assert_eq!(copies.len(), 1);
        let meta = std::fs::read_to_string(copies[0].with_extension("json")).unwrap();
        assert!(meta.contains("\"name\":\"Untitled\"") && meta.contains("\"path\":null"), "{meta}");
        // The copy is a real sprite.
        assert_eq!(Doc::open(&copies[0]).unwrap().width(), 32);
        // Unchanged since the copy: nothing is written again.
        let before = std::fs::metadata(&copies[0]).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        settle(&mut app);
        assert_eq!(std::fs::metadata(&copies[0]).unwrap().modified().unwrap(), before);
        // Saving removes the copy.
        app.tab_mut().doc.mark_saved();
        settle(&mut app);
        assert!(files(&dir, "pxmt").is_empty() && files(&dir, "json").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn closing_a_tab_removes_its_copy_and_a_clean_exit_removes_the_rest() {
        let dir = scratch("close");
        let mut app = app_with_autosave(&dir);
        app.tab_mut().doc.touch();
        app.new_sprite(8, 8);
        app.tab_mut().doc.touch();
        settle(&mut app);
        assert_eq!(files(&dir, "pxmt").len(), 2);
        app.request_close(0);
        app.answer_prompt(crate::ui::dialogs::Answer::Discard);
        settle(&mut app);
        assert_eq!(files(&dir, "pxmt").len(), 1);
        app.discard_recovery_copies();
        assert!(files(&dir, "pxmt").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn leftover_copies_are_found_and_restored() {
        let work = scratch("restore");
        let dir = work.join("Recovery");
        let original = work.join("art.aseprite");
        let mut first = app_with_autosave(&dir);
        first.tab_mut().doc.save_marked(&original).unwrap();
        first.tab_mut().doc.touch();
        first.new_sprite(8, 8);
        first.tab_mut().doc.touch();
        settle(&mut first);
        assert_eq!(files(&dir, "pxmt").len(), 2);
        // The first run "crashes": nothing is cleaned up. A new run finds the copies.
        let mut second = PixmithApp::default();
        second.gamedev.autosave.start_in(dir.clone());
        let found = second.gamedev.autosave.offer().expect("offers the copies").to_vec();
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|c| c.original.as_deref() == Some(original.to_str().unwrap())));
        for c in found.iter().rev() {
            second.restore_recovered(c);
        }
        assert!(second.error.is_none(), "{:?}", second.error);
        assert_eq!(second.tabs.len(), 2);
        assert!(second.tabs.iter().all(|t| t.doc.is_dirty()), "restored work counts as unsaved");
        assert!(second.tabs.iter().any(|t| t.doc.path.as_deref() == Some(original.as_path())));
        // The restored tabs keep using the copies' ids, so nothing new is created beside them.
        settle(&mut second);
        assert_eq!(files(&dir, "pxmt").len(), 2, "{:?}", files(&dir, "pxmt"));
        let _ = std::fs::remove_dir_all(work);
    }
}
