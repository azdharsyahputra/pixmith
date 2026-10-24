//! The C ABI boundary: error reporting, the panic guard, and the helpers every `pixmith_*` entry point shares.
//!
//! Every entry point tolerates null pointers and out-of-range indices, and runs its body inside
//! [`guarded`] so a panic never crosses into Swift. The entry points for the document itself live in
//! [`document`]; each feature module exposes its own next to the Rust implementation.

pub mod document;
pub mod live;

#[cfg(test)]
mod guard_tests;

use crate::document::{Editor, Sprite};
use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, Once};

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

/// Coordinates beyond this are far outside any canvas; clamping them keeps drawing loops bounded
/// (and their arithmetic free of overflow) when a caller passes absurd values.
pub(crate) const COORD_LIMIT: i32 = 1 << 17;

pub(crate) fn clampc(v: i32) -> i32 {
    v.clamp(-COORD_LIMIT, COORD_LIMIT)
}

pub(crate) fn set_error(msg: impl Into<String>) {
    let msg = CString::new(msg.into().replace('\0', "")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = msg);
}

// MARK: Panic guard

/// What an entry point returns after a panic: false, zero or null.
pub trait FfiDefault {
    fn ffi_default() -> Self;
}

macro_rules! ffi_default_zero {
    ($($t:ty),*) => {$(impl FfiDefault for $t {
        fn ffi_default() -> Self {
            Default::default()
        }
    })*};
}
ffi_default_zero!((), bool, u8, u16, u32, u64, i8, i16, i32, i64, usize, f32, f64);

impl<T> FfiDefault for *mut T {
    fn ffi_default() -> Self {
        null_mut()
    }
}

impl<T> FfiDefault for *const T {
    fn ffi_default() -> Self {
        std::ptr::null()
    }
}

static PANIC_COUNT: AtomicU32 = AtomicU32::new(0);
static LAST_PANIC: Mutex<String> = Mutex::new(String::new());

thread_local! {
    static PANIC_SITE: RefCell<String> = const { RefCell::new(String::new()) };
    static LAST_PANIC_C: RefCell<CString> = RefCell::new(CString::default());
}

/// Records every panic (also those that Lua's `pcall` swallows inside a script callback, which never
/// reach an entry point's guard), then keeps the default report.
fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let site = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
            *LAST_PANIC.lock().unwrap_or_else(|p| p.into_inner()) =
                format!("{} (at {site})", panic_message(info.payload()));
            PANIC_SITE.with(|s| *s.borrow_mut() = site);
            PANIC_COUNT.fetch_add(1, Ordering::SeqCst);
            previous(info);
        }));
    });
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

/// Runs the body of an `extern "C"` entry point. A panic never crosses the FFI boundary: it becomes a
/// default return value and an "internal error" in `pixmith_last_error`, the panic is recorded for
/// `pixmith_last_panic`, and the document the call was working on (`doc`, may be null) is rewound to a
/// consistent state.
pub(crate) fn guarded<R: FfiDefault>(name: &'static str, doc: *mut Editor, body: impl FnOnce() -> R) -> R {
    install_panic_hook();
    // SAFETY: entry points receive either null or a live editor, as their contract says.
    let before = unsafe { doc.as_ref() }.map(Editor::checkpoint_count);
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(payload) => {
            let site = PANIC_SITE.with(|s| s.borrow().clone());
            let rewound = match (unsafe { doc.as_mut() }, before) {
                (Some(e), Some(before)) => {
                    catch_unwind(AssertUnwindSafe(|| e.recover_after_panic(before))).unwrap_or(false)
                }
                _ => false,
            };
            let mut text = format!("{name}: {} (at {site})", panic_message(&*payload));
            if rewound {
                text.push_str("; the document was rewound to its last undo step");
            }
            set_error(format!("internal error: {text}"));
            *LAST_PANIC.lock().unwrap_or_else(|p| p.into_inner()) = text;
            R::ffi_default()
        }
    }
}

/// Number of panics since the process started (caught by an entry point's guard or inside a script). When it grows after a call,
/// something went wrong inside the core: tell the user and offer to save a copy of the document.
#[no_mangle]
pub extern "C" fn pixmith_panic_count() -> u32 {
    PANIC_COUNT.load(Ordering::SeqCst)
}

/// Description of the most recent caught panic (empty if none). Valid until the next call on this thread.
#[no_mangle]
pub extern "C" fn pixmith_last_panic() -> *const c_char {
    let text = LAST_PANIC.lock().unwrap_or_else(|p| p.into_inner()).replace('\0', "");
    LAST_PANIC_C.with(|c| {
        *c.borrow_mut() = CString::new(text).unwrap_or_default();
        c.borrow().as_ptr()
    })
}

/// The editor for an operation that may change anything: the whole document counts as dirty afterwards.
pub(crate) unsafe fn editor<'a>(doc: *mut Editor) -> Option<&'a mut Editor> {
    let e = editor_quiet(doc)?;
    e.dirty.mark_all();
    Some(e)
}

/// Like `editor`, for operations that report exactly which pixels they change (`Dirty::mark_rect`).
pub(crate) unsafe fn editor_quiet<'a>(doc: *mut Editor) -> Option<&'a mut Editor> {
    let e = doc.as_mut()?;
    e.sprite.flush_links();
    // Catches an invariant broken by the previous call right where it happens (tests, debug builds).
    debug_assert!(e.sprite.validate().is_ok(), "sprite invariant broken: {:?}", e.sprite.validate());
    Some(e)
}

/// Like `doc.as_ref()`, but first re-shares the cels of a linked-cel edit that is still pending, so readers (render,
/// export, save) never see linked cels out of sync right after a stroke.
pub(crate) unsafe fn synced<'a>(doc: *const Editor) -> Option<&'a Editor> {
    let e = (doc as *mut Editor).as_mut()?;
    e.sprite.flush_links();
    Some(e)
}

pub(crate) unsafe fn sprite<'a>(doc: *const Editor) -> Option<&'a Sprite> {
    synced(doc).map(|e| &e.sprite)
}

pub(crate) unsafe fn path_arg(p: *const c_char) -> Option<String> {
    (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
}

/// Saves through `formats::durable::write_file`: a failed or interrupted save never damages the original.
pub(crate) unsafe fn save_to(path: *const c_char, data: Result<Vec<u8>, String>) -> bool {
    let Some(path) = path_arg(path) else {
        set_error("invalid path");
        return false;
    };
    match data.and_then(|d| crate::formats::durable::write_file(&path, &d)) {
        Ok(()) => true,
        Err(e) => {
            set_error(e);
            false
        }
    }
}

/// Message for the last failed call on this thread. Valid until the next failure.
#[no_mangle]
pub extern "C" fn pixmith_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_string_free(s: *mut c_char) {
    crate::ffi::guarded("pixmith_string_free", std::ptr::null_mut(), || {
        if !s.is_null() {
            drop(CString::from_raw(s));
        }
    })
}
