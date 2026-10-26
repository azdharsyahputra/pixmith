//! File and command permissions for scripts ("ask first", `docs/LUA_HOST_API.md` section 1).
//!
//! The core calls the permission handler on the script's own thread. The handler here queues the request, wakes the
//! UI and blocks until the user has answered; `ui::lua_dialog` draws the question. This is the only file that
//! talks to `pixmith_core::scripting::access`.
//!
//! Without [`install`] (tests, `PIXMITH_NO_DIALOGS`) the core has no handler and denies every request.

use eframe::egui;
use pixmith_core::scripting::access::{self, Access, Permission, PermissionRequest};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

/// One question waiting for the UI. Dropping it without an answer denies the request.
pub struct Pending {
    pub request: PermissionRequest,
    reply: Sender<Permission>,
}

impl Pending {
    /// Hands the answer to the script thread.
    pub fn answer(self, permission: Permission) {
        let _ = self.reply.send(permission);
    }
}

static QUEUE: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
static CONTEXTS: Mutex<Vec<egui::Context>> = Mutex::new(Vec::new());

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Installs the handler: requests are queued for the UI that owns `ctx` (and woken with a repaint) and the script
/// thread waits for the answer.
pub fn install(ctx: &egui::Context) {
    {
        let mut contexts = lock(&CONTEXTS);
        if !contexts.iter().any(|c| c == ctx) {
            contexts.push(ctx.clone());
        }
    }
    access::set_permission_handler(Some(Arc::new(|request: &PermissionRequest| {
        let (reply, answer) = channel();
        lock(&QUEUE).push(Pending { request: request.clone(), reply });
        for ctx in lock(&CONTEXTS).iter() {
            ctx.request_repaint();
        }
        // A dropped `Pending` (the app quit, the script was stopped) ends the wait with a denial.
        answer.recv().unwrap_or(Permission::Deny)
    })));
}

/// Takes the queued requests that `wanted` accepts, oldest first.
pub fn take(wanted: impl Fn(&PermissionRequest) -> bool) -> Vec<Pending> {
    let mut queue = lock(&QUEUE);
    let mut taken = Vec::new();
    let mut i = 0;
    while i < queue.len() {
        if wanted(&queue[i].request) {
            taken.push(queue.remove(i));
        } else {
            i += 1;
        }
    }
    taken
}

/// Preferences > "Forget allowed scripts": every "always allow" answer goes.
pub fn forget_grants() {
    access::reset_grants();
}

/// The verb of the question.
pub fn verb(access: Access) -> &'static str {
    match access {
        Access::Read => "read",
        Access::Write => "write",
        Access::Execute => "run",
    }
}

/// What "Always Allow for This Script" grants, for the line under the question.
pub fn always_note(access: Access) -> &'static str {
    match access {
        Access::Read => "Always Allow lets this script read any file until Pixmith quits.",
        Access::Write => "Always Allow lets this script read and write any file until Pixmith quits.",
        Access::Execute => "Always Allow lets this script run any command until Pixmith quits.",
    }
}
