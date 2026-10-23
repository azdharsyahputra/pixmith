//! Background work behind a progress sheet (`Background` and `ProgressSheet` in `ProgressSheet.swift`).
//!
//! Threading rule, as in the Swift app: the worker must never touch the document the UI is using. Give it a
//! snapshot (`Doc::snapshot`), an independent document or plain data; what it returns is a closure that runs
//! on the UI thread and may apply the result. One job runs at a time.

use crate::app::PixmithApp;
use crate::theme::{Colors, radius, space};
use crate::ui::widgets::{self, DialogAction};
use eframe::egui;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

/// What the worker hands back: runs on the UI thread once the work is done.
pub type Done = Box<dyn FnOnce(&mut PixmithApp) + Send>;

/// A running job and its progress sheet.
pub struct Job {
    title: String,
    started: Instant,
    /// Show the sheet at once; otherwise only if the job takes longer than a quarter of a second.
    immediate: bool,
    cancel: Option<fn()>,
    cancelling: bool,
    rx: Receiver<Done>,
}

impl Job {
    pub fn title(&self) -> &str {
        &self.title
    }

    fn sheet_visible(&self) -> bool {
        self.immediate || self.started.elapsed() >= Duration::from_millis(250)
    }
}

impl PixmithApp {
    /// Whether a job is running (`Background.busy`); menus disable commands that would change the document
    /// under it.
    pub fn job_running(&self) -> bool {
        self.gamedev.job.is_some()
    }

    /// Starts `work` on a background thread. Returns false, doing nothing, if another job is running.
    /// `cancel` is called (once) when the user presses Cancel on the sheet.
    pub fn start_job(
        &mut self,
        title: &str,
        immediate: bool,
        cancel: Option<fn()>,
        work: impl FnOnce() -> Done + Send + 'static,
    ) -> bool {
        if self.gamedev.job.is_some() {
            return false;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let done = work();
            let _ = tx.send(done);
        });
        self.gamedev.job =
            Some(Job { title: title.to_string(), started: Instant::now(), immediate, cancel, cancelling: false, rx });
        true
    }
}

/// Collects a finished job and runs its completion on the UI thread.
pub fn poll(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(job) = &app.gamedev.job else { return };
    match job.rx.try_recv() {
        Ok(done) => {
            app.gamedev.job = None;
            done(app);
            ctx.request_repaint();
        }
        Err(TryRecvError::Disconnected) => {
            app.gamedev.job = None;
            app.error = Some("The background job stopped unexpectedly.".into());
        }
        Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(40)),
    }
}

/// The progress sheet (`ProgressSheet`): title, an indeterminate bar and Cancel.
pub fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    let Some(job) = &mut app.gamedev.job else { return };
    if !job.sheet_visible() {
        return;
    }
    let c = Colors::of(ctx);
    let (title, can_cancel, cancelling) = (job.title.clone(), job.cancel.is_some(), job.cancelling);
    let action = widgets::dialog(ctx, "job", 340.0, &title, None, |ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, radius::CHIP, c.field);
        let t = ui.input(|i| i.time) as f32;
        let seg = rect.width() * 0.3;
        let phase = (t * 0.9).fract();
        let x = rect.left() - seg + phase * (rect.width() + seg);
        let lit = egui::Rect::from_min_max(
            egui::pos2(x.max(rect.left()), rect.top()),
            egui::pos2((x + seg).min(rect.right()), rect.bottom()),
        );
        if lit.is_positive() {
            ui.painter().rect_filled(lit, radius::CHIP, c.accent);
        }
        ui.ctx().request_repaint();
        if !can_cancel {
            return DialogAction::None;
        }
        ui.add_space(space::XS);
        let mut action = DialogAction::None;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!cancelling, |ui| {
                    let label = if cancelling { "Cancelling\u{2026}" } else { "Cancel" };
                    if widgets::secondary_button(ui, label, false).clicked() {
                        action = DialogAction::Cancel;
                    }
                });
            });
        });
        if !cancelling && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            action = DialogAction::Cancel;
        }
        action
    });
    if action == DialogAction::Cancel
        && let Some(job) = &mut app.gamedev.job
        && !job.cancelling
    {
        job.cancelling = true;
        if let Some(cancel) = job.cancel {
            cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_runs_off_thread_and_completes_on_the_ui_thread() {
        let mut app = PixmithApp::default();
        let ctx = egui::Context::default();
        assert!(app.start_job("Working", true, None, || {
            std::thread::sleep(Duration::from_millis(30));
            Box::new(|app: &mut PixmithApp| app.error = Some("done".into()))
        }));
        assert!(app.job_running());
        assert!(!app.start_job("Second", false, None, || Box::new(|_: &mut PixmithApp| {})), "one job at a time");
        for _ in 0..200 {
            poll(&mut app, &ctx);
            if !app.job_running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!app.job_running());
        assert_eq!(app.error.as_deref(), Some("done"));
    }

    #[test]
    fn a_panicking_job_reports_an_error() {
        let mut app = PixmithApp::default();
        let ctx = egui::Context::default();
        app.start_job("Boom", false, None, || -> Done { panic!("worker failed") });
        for _ in 0..200 {
            poll(&mut app, &ctx);
            if !app.job_running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_some());
    }
}
