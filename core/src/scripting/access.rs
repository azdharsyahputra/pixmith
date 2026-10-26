//! "Ask first" permissions for everything a script does outside its document: reading and writing
//! files, listing directories and running commands (docs/LUA_HOST_API.md, section 1).
//!
//! Every guarded call goes through [`check`]. A request is answered by the core when it can (a
//! pre-allowed path prefix, an earlier grant) and otherwise by the front end's permission handler,
//! which runs on the script's thread and may block while it shows a dialog. With no handler the
//! answer is "deny". The time spent waiting for the handler does not count toward the script's
//! wall-clock limit.

use super::Ctx;
use mlua::{Error, Result as LuaResult};
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// What a request wants to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Access {
    Read,
    Write,
    Execute,
}

impl Access {
    fn bit(self) -> u32 {
        match self {
            Access::Read => 1,
            Access::Write => 2,
            Access::Execute => 4,
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
            Access::Execute => "run",
        }
    }
}

/// One question for the front end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequest {
    /// The script's name (the file name the front end gave when it started the script).
    pub script_name: String,
    /// An absolute, normalized path; for `Execute` the command line.
    pub path: String,
    pub access: Access,
}

/// The answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Deny,
    /// This request, and for the rest of the run everything under the same path.
    AllowOnce,
    /// Every later request of this kind from this script, until the app quits.
    AllowAlwaysForScript,
}

type Handler = Arc<dyn Fn(&PermissionRequest) -> Permission + Send + Sync>;

static HANDLER: Mutex<Option<Handler>> = Mutex::new(None);
static ALLOWED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static GRANTS: Mutex<Option<HashSet<(String, u32)>>> = Mutex::new(None);

thread_local! {
    /// Overrides the global handler on one thread (tests run in parallel and must not share a handler).
    static THREAD_HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Installs (or removes) the permission handler. It is global to the process.
pub fn set_permission_handler(h: Option<Arc<dyn Fn(&PermissionRequest) -> Permission + Send + Sync>>) {
    *lock(&HANDLER) = h;
}

/// Like [`set_permission_handler`], but only for scripts that run on the calling thread (tests).
#[doc(hidden)]
pub fn set_thread_permission_handler(h: Option<Arc<dyn Fn(&PermissionRequest) -> Permission + Send + Sync>>) {
    THREAD_HANDLER.with(|t| *t.borrow_mut() = h);
}

/// Replaces the process-wide path prefixes that are always allowed for reading and writing.
pub fn set_allowed_paths(paths: Vec<PathBuf>) {
    *lock(&ALLOWED) = paths.into_iter().map(|p| normalize(&p)).collect();
}

pub fn allowed_paths() -> Vec<PathBuf> {
    lock(&ALLOWED).clone()
}

/// Forgets every "always allow" answer.
pub fn reset_grants() {
    *lock(&GRANTS) = None;
}

/// Forgets the "always allow" answers of one script (tests that run side by side must not reset each other).
#[doc(hidden)]
pub fn reset_script_grants(script_name: &str) {
    if let Some(set) = lock(&GRANTS).as_mut() {
        set.retain(|(s, _)| s != script_name);
    }
}

/// The permission state of one run, shared by all documents of the run.
#[derive(Default)]
pub(super) struct RunAccess {
    pub script: RefCell<String>,
    /// Prefixes allowed for this run only (`RunConfig::allowed_paths`).
    pub extra: RefCell<Vec<PathBuf>>,
    once: RefCell<Vec<(PathBuf, Access)>>,
    tmp: RefCell<Vec<PathBuf>>,
    commands: RefCell<Vec<String>>,
}

impl RunAccess {
    /// A new run: earlier "allow once" answers no longer apply.
    pub fn reset_run(&self) {
        self.once.borrow_mut().clear();
        self.tmp.borrow_mut().clear();
        self.commands.borrow_mut().clear();
    }

    pub fn allow_tmp(&self, p: PathBuf) {
        self.tmp.borrow_mut().push(p);
    }
}

// MARK: Paths

/// An absolute path with `.` and `..` resolved lexically (no symlinks are followed).
pub fn normalize(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")).join(p)
    };
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push("/");
    }
    out
}

fn grant_key(script: &str, access: Access) -> (String, u32) {
    (script.to_string(), access.bit())
}

fn always_granted(script: &str, access: Access) -> bool {
    let g = lock(&GRANTS);
    let Some(set) = g.as_ref() else { return false };
    set.contains(&grant_key(script, access))
        || (access == Access::Read && set.contains(&grant_key(script, Access::Write)))
}

fn handler() -> Option<Handler> {
    THREAD_HANDLER.with(|t| t.borrow().clone()).or_else(|| lock(&HANDLER).clone())
}

fn denied(script: &str, access: Access, what: &str) -> Error {
    Error::runtime(format!("permission denied: script \"{script}\" may not {} {what}", access.verb()))
}

/// Decides one request. Returns the normalized path (the command line for `Execute`).
pub(super) fn check(ctx: &Ctx, raw: &str, access: Access) -> LuaResult<PathBuf> {
    let run = &ctx.shared.access;
    let script = run.script.borrow().clone();
    if raw.is_empty() || raw.contains('\0') {
        return Err(Error::runtime("invalid path"));
    }
    if access == Access::Execute {
        if run.commands.borrow().iter().any(|c| c == raw) || always_granted(&script, access) {
            return Ok(PathBuf::from(raw));
        }
        ask(ctx, &script, raw, access, |_| run.commands.borrow_mut().push(raw.to_string()))?;
        return Ok(PathBuf::from(raw));
    }
    let path = normalize(Path::new(raw));
    let covers = |prefix: &Path| path.starts_with(prefix);
    let allowed = lock(&ALLOWED).iter().any(|p| covers(p))
        || run.extra.borrow().iter().any(|p| covers(&normalize(p)))
        || run.tmp.borrow().iter().any(|p| covers(p))
        || run.once.borrow().iter().any(|(p, a)| covers(p) && (*a == access || *a == Access::Write))
        || always_granted(&script, access);
    if allowed {
        return Ok(path);
    }
    let shown = path.to_string_lossy().into_owned();
    ask(ctx, &script, &shown, access, |_| run.once.borrow_mut().push((path.clone(), access)))?;
    Ok(path)
}

/// Calls the handler (timing it) and records an `AllowOnce` through `once`.
fn ask(ctx: &Ctx, script: &str, path: &str, access: Access, once: impl FnOnce(())) -> LuaResult<()> {
    let Some(h) = handler() else { return Err(denied(script, access, path)) };
    let req = PermissionRequest { script_name: script.to_string(), path: path.to_string(), access };
    let started = Instant::now();
    let answer = catch_unwind(AssertUnwindSafe(|| h(&req))).unwrap_or(Permission::Deny);
    ctx.add_wait(started.elapsed());
    match answer {
        Permission::Deny => Err(denied(script, access, path)),
        Permission::AllowOnce => {
            once(());
            Ok(())
        }
        Permission::AllowAlwaysForScript => {
            lock(&GRANTS).get_or_insert_with(HashSet::new).insert(grant_key(script, access));
            Ok(())
        }
    }
}

// MARK: C ABI

/// `access`: 1 read, 2 write, 4 execute. Returns 0 deny, 1 allow once, 2 always for this script.
pub type PermissionFn =
    unsafe extern "C" fn(ctx: *mut c_void, script: *const c_char, path: *const c_char, access: u32) -> u32;

struct CHandler {
    f: PermissionFn,
    ctx: *mut c_void,
}

impl CHandler {
    fn call(&self, script: &CString, path: &CString, access: u32) -> u32 {
        // SAFETY: see below.
        unsafe { (self.f)(self.ctx, script.as_ptr(), path.as_ptr(), access) }
    }
}

// SAFETY: the front end promises that its callback and context may be used from any thread (it is called on
// script threads); the context pointer is only ever passed back to the callback.
unsafe impl Send for CHandler {}
unsafe impl Sync for CHandler {}

/// Installs (or, with a NULL function, removes) the permission handler. See `docs/LUA_HOST_API.md`.
///
/// # Safety
/// `f` must be callable from any thread with `ctx` until it is replaced.
#[no_mangle]
pub unsafe extern "C" fn pixmith_script_set_permission_handler(f: Option<PermissionFn>, ctx: *mut c_void) {
    crate::ffi::guarded("pixmith_script_set_permission_handler", std::ptr::null_mut(), || match f {
        None => set_permission_handler(None),
        Some(f) => {
            let h = CHandler { f, ctx };
            set_permission_handler(Some(Arc::new(move |r: &PermissionRequest| {
                let (Ok(s), Ok(p)) =
                    (CString::new(r.script_name.replace('\0', "")), CString::new(r.path.replace('\0', "")))
                else {
                    return Permission::Deny;
                };
                match h.call(&s, &p, r.access.bit()) {
                    1 => Permission::AllowOnce,
                    2 => Permission::AllowAlwaysForScript,
                    _ => Permission::Deny,
                }
            })));
        }
    })
}

/// Forgets every "always allow" answer (Preferences > Forget allowed scripts).
#[no_mangle]
pub extern "C" fn pixmith_script_reset_grants() {
    crate::ffi::guarded("pixmith_script_reset_grants", std::ptr::null_mut(), reset_grants)
}

/// Replaces the process-wide pre-allowed path prefixes (read and write).
///
/// # Safety
/// `paths` must point to `count` valid NUL-terminated strings (or be NULL when `count` is 0).
#[no_mangle]
pub unsafe extern "C" fn pixmith_script_set_allowed_paths(paths: *const *const c_char, count: u32) {
    crate::ffi::guarded("pixmith_script_set_allowed_paths", std::ptr::null_mut(), || {
        let mut v = Vec::new();
        if !paths.is_null() {
            for i in 0..count as usize {
                let p = *paths.add(i);
                if !p.is_null() {
                    v.push(PathBuf::from(CStr::from_ptr(p).to_string_lossy().into_owned()));
                }
            }
        }
        set_allowed_paths(v);
    })
}
