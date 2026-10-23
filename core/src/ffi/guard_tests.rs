//! Tests for the panic guard around every `extern "C"` entry point.

use super::{guarded, pixmith_last_error, pixmith_last_panic, pixmith_panic_count};
use crate::document::{Editor, Sprite};
use std::ffi::CStr;

/// The panic counter and message are process-wide, so tests that panic on purpose take turns.
static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// A test-only entry point shaped like the real ones. Modes:
/// 0 takes a checkpoint, edits, then panics; 1 edits without a checkpoint, then panics;
/// 2 panics inside an open transaction; 3 succeeds; 4 panics with a non-string payload.
unsafe extern "C" fn pixmith_test_panic(doc: *mut Editor, mode: u32) -> bool {
    guarded("pixmith_test_panic", doc, || {
        let e = &mut *doc;
        match mode {
            0 => {
                e.checkpoint();
                e.sprite.draw_line(0, 0, 0, 0, 3, 3, BLUE, 1);
                panic!("boom in mode 0");
            }
            1 => {
                e.sprite.draw_line(0, 0, 0, 0, 3, 3, BLUE, 1);
                panic!("boom in mode 1");
            }
            2 => {
                e.begin_transaction();
                e.checkpoint();
                e.sprite.draw_line(0, 0, 1, 1, 1, 1, BLUE, 1);
                panic!("boom in mode 2");
            }
            4 => std::panic::panic_any(42u8),
            _ => {}
        }
        true
    })
}

unsafe fn text(p: *const std::ffi::c_char) -> String {
    CStr::from_ptr(p).to_string_lossy().into_owned()
}

fn editor_with_red_pixel() -> Box<Editor> {
    let mut e = Box::new(Editor::new(Sprite::new(4, 4)));
    e.checkpoint();
    e.sprite.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
    e
}

#[test]
fn a_panic_after_the_operations_own_checkpoint_rolls_that_step_back() {
    let _turn = TURN.lock().unwrap_or_else(|p| p.into_inner());
    unsafe {
        let mut e = editor_with_red_pixel();
        let before = e.sprite.render(0);
        let history = e.history_len();
        let count = pixmith_panic_count();

        assert!(!pixmith_test_panic(&mut *e, 0));

        assert_eq!(e.sprite.render(0), before, "the half-done edit is gone");
        assert_eq!(e.history_len(), history, "no step was left behind and none was lost");
        assert_eq!(pixmith_panic_count(), count + 1);
        assert!(
            text(pixmith_last_error()).starts_with("internal error: pixmith_test_panic: boom in mode 0"),
            "{}",
            text(pixmith_last_error())
        );
        let last = text(pixmith_last_panic());
        assert!(
            last.contains("boom in mode 0") && last.contains("guard_tests.rs") && last.contains("rewound"),
            "{last}"
        );

        // The editor keeps working normally.
        assert!(pixmith_test_panic(&mut *e, 3));
        assert!(e.undo());
        assert_eq!(e.sprite.render(0), vec![0; 64]);
    }
}

#[test]
fn a_panic_without_a_new_checkpoint_undoes_the_last_step_and_keeps_it_redoable() {
    let _turn = TURN.lock().unwrap_or_else(|p| p.into_inner());
    unsafe {
        let mut e = editor_with_red_pixel();
        let edited = e.sprite.render(0);
        assert!(!pixmith_test_panic(&mut *e, 1));
        // The half-edited state is not left in the document; the user's earlier edit sits on the redo stack.
        assert_eq!(e.sprite.render(0), vec![0; 64]);
        assert_eq!(e.history_len(), (0, 1));
        // Redo brings back the state the panic interrupted: the earlier edit plus the half-done one.
        let mut interrupted = Sprite::new(4, 4);
        interrupted.draw_line(0, 0, 0, 0, 0, 0, RED, 1);
        interrupted.draw_line(0, 0, 0, 0, 3, 3, BLUE, 1);
        assert!(e.redo());
        assert_eq!(e.sprite.render(0), interrupted.render(0));
        assert_ne!(e.sprite.render(0), edited);
    }
}

#[test]
fn a_panic_inside_a_transaction_closes_it() {
    let _turn = TURN.lock().unwrap_or_else(|p| p.into_inner());
    unsafe {
        let mut e = editor_with_red_pixel();
        assert!(!pixmith_test_panic(&mut *e, 2));
        assert_eq!(e.txn_depth, 0);
        // Later checkpoints are recorded normally again.
        let steps = e.history_len().0;
        e.checkpoint();
        e.checkpoint();
        assert_eq!(e.history_len().0, steps + 2);
    }
}

#[test]
fn null_documents_and_odd_payloads_are_handled() {
    let _turn = TURN.lock().unwrap_or_else(|p| p.into_inner());
    unsafe {
        assert!(!pixmith_test_panic_null());
        let mut e = editor_with_red_pixel();
        assert!(!pixmith_test_panic(&mut *e, 4));
        assert!(text(pixmith_last_panic()).contains("unknown panic"));
        assert!(pixmith_test_panic(&mut *e, 3));
    }
}

unsafe fn pixmith_test_panic_null() -> bool {
    guarded("pixmith_test_panic_null", std::ptr::null_mut(), || -> bool { panic!("no document") })
}

#[test]
fn lua_errors_stay_errors_and_panics_in_callbacks_reach_the_guard() {
    let _turn = TURN.lock().unwrap_or_else(|p| p.into_inner());
    let lua = mlua::Lua::new();
    // An ordinary Lua error is a value, caught by pcall and returned by exec.
    assert!(lua.load("error('plain')").exec().is_err());
    lua.load("assert(not pcall(error, 'x'))").exec().unwrap();
    // A Rust panic inside a callback becomes a Lua error that pcall catches; it is still recorded, so the
    // app hears about it through pixmith_panic_count.
    lua.globals()
        .set("boom", lua.create_function(|_, ()| -> mlua::Result<()> { panic!("callback boom") }).unwrap())
        .unwrap();
    let count = pixmith_panic_count();
    guarded("lua_test", std::ptr::null_mut(), || lua.load("assert(not pcall(boom))").exec().unwrap());
    assert_eq!(pixmith_panic_count(), count + 1);
    unsafe {
        assert!(text(pixmith_last_panic()).contains("callback boom"));
    }
    // Without pcall the panic travels up through the script run to the entry point's guard, which turns it
    // into the default return value and an error message instead of a crash.
    let survived = guarded("lua_test", std::ptr::null_mut(), || lua.load("boom()").exec().is_err());
    assert!(!survived);
    unsafe {
        assert!(text(pixmith_last_error()).starts_with("internal error: lua_test"), "{}", text(pixmith_last_error()));
    }
    // The Lua state is still usable afterwards.
    assert!(lua.load("return 1 + 1").eval::<i32>().is_ok());
}

/// Every `.rs` file under `dir`, recursively.
fn collect_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sources(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_entry_point_is_guarded() {
    // Entry points that only read a thread-local string and cannot panic.
    const EXEMPT: [&str; 3] = ["pixmith_last_error", "pixmith_last_panic", "pixmith_panic_count"];
    let mut files = Vec::new();
    collect_sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut unguarded = Vec::new();
    let mut seen = 0;
    for path in files {
        if path.ends_with("guard_tests.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for (i, line) in source.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") || !t.contains("extern \"C\" fn pixmith_") || !t.starts_with("pub ") {
                continue;
            }
            let name: String =
                t.split("fn ").nth(1).unwrap().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if EXEMPT.contains(&name.as_str()) {
                continue;
            }
            seen += 1;
            // The guard is the first statement of the body (the signature may span several lines).
            let rest: String = source.lines().skip(i).take(40).collect::<Vec<_>>().join("\n");
            let body = rest.split_once(" {\n").map_or("", |(_, b)| b);
            if !body.trim_start().starts_with("crate::ffi::guarded(") {
                unguarded.push(format!("{}: {name}", path.display()));
            }
        }
    }
    assert!(seen > 150, "scanned only {seen} entry points");
    assert!(unguarded.is_empty(), "entry points without a panic guard: {unguarded:?}");
}
