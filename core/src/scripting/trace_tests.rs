//! `app.command.TraceGrid` and `app.command.RemoveBackground`, plus the C ABI and the import mode of the same
//! features. Everything runs on a synthetic image held in memory or in a private temp directory.

use super::*;
use crate::document::{Cel, Editor, Sprite};
use crate::paint::trace_fixture as fx;
use std::ffi::{CStr, CString};
use std::sync::Arc;

static NEVER_FLAG: AtomicBool = AtomicBool::new(false);

fn editor() -> Editor {
    let (w, h, rgba) = fx::image();
    let mut s = Sprite::new(w as u32, h as u32);
    s.cels[0][0] = Some(Arc::new(Cel { opacity: 255, pixels: rgba, ..Default::default() }));
    Editor::new(s)
}

fn run(e: &mut Editor, src: &str) -> Outcome {
    run_in(e, src, (0, 0), Limits::default(), &NEVER_FLAG, &RunConfig::default())
}

fn ok(e: &mut Editor, src: &str) {
    let o = run(e, src);
    assert_eq!(o.error, None, "script failed: {src}");
}

#[test]
fn lua_remove_background_reports_and_respects_the_flood() {
    let mut e = editor();
    let (w, _, _) = fx::image();
    ok(
        &mut e,
        r#"
        assert(sprite:getPixel(0, 0) & 0xFF == 255, "the paper is opaque to begin with")
        local r = app.command.RemoveBackground{ tolerance = 28, shadow = true }
        assert(r.removed > 1000 and r.background > 1000 and r.shadow > 100, "report")
        assert(r.color.red >= 245 and r.color.green >= 245)
        assert(sprite:getPixel(0, 0) & 0xFF == 0, "the corner is gone")
        -- the white of an eye is enclosed by the figure and stays
        local ex = math.floor(14.4 + 5.5 * 8.3)
        local ey = math.floor(9.7 + 5.5 * 8.3)
        assert(sprite:getPixel(ex, ey) & 0xFF == 255, "enclosed white stays")
        app.undo()
        assert(sprite:getPixel(0, 0) & 0xFF == 255, "undo restores the paper")
        "#,
    );
    assert!(e.sprite.width as usize == w);
}

#[test]
fn lua_trace_grid_resizes_and_sets_the_palette() {
    let mut e = editor();
    ok(
        &mut e,
        r#"
        local r = app.command.TraceGrid{ pitch = "auto", colors = 16, removeBackground = true, shadow = true, closeOutline = true }
        assert(math.abs(r.pitch[1] - 8.3) <= 0.1 and math.abs(r.pitch[2] - 8.3) <= 0.1, "pitch " .. r.pitch[1])
        assert(r.width == 16 and r.height == 16 and sprite.width == 16 and sprite.height == 16)
        assert(r.quality > 0.6 and #r.palette >= 4 and #r.palette <= 16)
        assert(r.background.removed > 0)
        assert(sprite:getPixel(0, 0) & 0xFF == 0, "the corner is transparent")
        assert(sprite:getPixel(8, 0) & 0xFF == 255, "the outline is there")
        assert(#sprite.palettes[1] == #r.palette, "the sprite palette is the traced palette")
        app.undo()
        assert(sprite.width > 100, "undo brings the picture back")
        "#,
    );
    // Undoing the whole script restores the document through the script's own history; the result stays valid.
    e.sprite.validate().unwrap();
}

#[test]
fn lua_trace_grid_is_exact_with_known_pitch_and_palette_mode() {
    let mut e = editor();
    ok(
        &mut e,
        r#"
        local r = app.command.TraceGrid{ pitch = 8.3, phase = { 14.4 % 8.3, 9.7 % 8.3 }, colors = { Color(30, 28, 40), Color(200, 60, 70), Color(240, 190, 140), Color(255, 255, 255) }, removeBackground = true, shadow = true }
        assert(r.width == 16 and r.height == 16)
        assert(#r.palette == 4)
        assert(sprite:getPixel(0, 0) & 0xFF == 0 and sprite:getPixel(8, 0) & 0xFF == 255)
        assert(sprite:getPixel(8, 0) >> 8 == 0x1E1C28, "the outline snapped to the given dark color")
        "#,
    );
}

#[test]
fn lua_rejects_bad_trace_arguments() {
    for (src, needle) in [
        (r#"app.command.TraceGrid{ pitch = "huge" }"#, "pitch"),
        (r#"app.command.TraceGrid{ colors = "many" }"#, "colors"),
        (r#"app.command.TraceGrid{ colors = 0 }"#, "colors"),
        (r#"app.command.TraceGrid{ pitch = 0.2 }"#, "pitch"),
    ] {
        let mut e = editor();
        let o = run(&mut e, src);
        let err = o.error.unwrap_or_else(|| panic!("{src} should fail"));
        assert!(err.contains(needle), "{err:?} should mention {needle}");
        assert_eq!(e.sprite.width as usize, fx::image().0, "a refused command leaves the sprite alone");
    }
    // A flat sprite has no grid, and the message says so.
    let mut flat = Editor::new(Sprite::new(40, 40));
    let o = run(&mut flat, "app.command.TraceGrid{}");
    assert!(o.error.unwrap_or_default().contains("TraceGrid"), "flat sprite");
}

// MARK: C ABI

fn json(p: *mut std::ffi::c_char) -> crate::formats::json::Value {
    assert!(!p.is_null(), "{:?}", unsafe { CStr::from_ptr(crate::ffi::pixmith_last_error()) });
    let text = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { crate::ffi::pixmith_string_free(p) };
    crate::formats::json::parse(&text).unwrap()
}

#[test]
fn ffi_trace_grid_is_one_undo_step() {
    let mut e = editor();
    let (w, _, _) = fx::image();
    let opts =
        CString::new(r#"{"pitch":"auto","colors":16,"remove_background":true,"shadow":true,"close_outline":true}"#)
            .unwrap();
    let report = json(unsafe { crate::paint::trace::pixmith_trace_grid(&mut e, 0, 0, opts.as_ptr()) });
    assert_eq!(report.get("width").and_then(|v| v.as_f64()), Some(16.0));
    let pitch = report.get("pitch").and_then(|v| v.as_array()).unwrap()[0].as_f64().unwrap();
    assert!((pitch - 8.3).abs() <= 0.1, "{pitch}");
    assert!(report.get("palette").and_then(|v| v.as_array()).unwrap().len() >= 4);
    assert_eq!((e.sprite.width, e.sprite.height), (16, 16));
    assert!(fx::match_share(&e.sprite.render(0)) >= 0.95);
    assert!(e.undo());
    assert_eq!(e.sprite.width as usize, w);
    // A bad option is reported and leaves the document alone.
    let bad = CString::new(r#"{"colors":"lots"}"#).unwrap();
    assert!(unsafe { crate::paint::trace::pixmith_trace_grid(&mut e, 0, 0, bad.as_ptr()) }.is_null());
    let msg = unsafe { CStr::from_ptr(crate::ffi::pixmith_last_error()) }.to_string_lossy().into_owned();
    assert!(msg.contains("colors"), "{msg}");
    assert_eq!(e.sprite.width as usize, w);
    // Null document and options are tolerated.
    assert!(unsafe { crate::paint::trace::pixmith_trace_grid(std::ptr::null_mut(), 0, 0, std::ptr::null()) }.is_null());
}

#[test]
fn ffi_remove_background_and_the_filter_kind() {
    let mut e = editor();
    let opts = CString::new(r#"{"tolerance":28,"shadow":true}"#).unwrap();
    let stats = json(unsafe { crate::paint::background::pixmith_remove_background(&mut e, 0, 0, opts.as_ptr()) });
    assert!(stats.get("shadow").and_then(|v| v.as_f64()).unwrap() > 100.0);
    assert_eq!(e.sprite.render(0)[3], 0);
    assert!(e.undo());
    assert_eq!(e.sprite.render(0)[3], 255);
    // The same operation through the generic filter entry point: tolerance 28, shadow flag.
    let p = [28, 1];
    assert!(unsafe {
        crate::paint::filters::pixmith_filter_apply(
            &mut e,
            crate::paint::filters::KIND_REMOVE_BACKGROUND,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            p.as_ptr(),
            2,
        )
    });
    assert_eq!(e.sprite.render(0)[3], 0);
    // Nothing to remove the second time: no new history entry, still a report.
    let again = json(unsafe { crate::paint::background::pixmith_remove_background(&mut e, 0, 0, std::ptr::null()) });
    assert_eq!(again.get("removed").and_then(|v| v.as_f64()), Some(0.0));
}

// MARK: Import mode

#[test]
fn import_image_in_trace_grid_mode() {
    let (w, h, rgba) = fx::image();
    let dir = std::env::temp_dir().join(format!("pixmith-trace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sheet.png");
    std::fs::write(&path, crate::formats::export::encode_png(&rgba, w, h).unwrap()).unwrap();
    let c_path = CString::new(path.to_string_lossy().as_bytes()).unwrap();
    let opts = CString::new(r#"{"mode":"trace_grid","colors":12,"remove_background":true,"shadow":true}"#).unwrap();
    let ed = unsafe { crate::formats::raster::abi::pixmith_import_image_ex(c_path.as_ptr(), opts.as_ptr()) };
    assert!(!ed.is_null(), "{:?}", unsafe { CStr::from_ptr(crate::ffi::pixmith_last_error()) });
    let e = unsafe { Box::from_raw(ed) };
    assert_eq!((e.sprite.width, e.sprite.height), (16, 16));
    assert!(fx::match_share(&e.sprite.render(0)) >= 0.95);
    assert!(e.sprite.palette.len() <= 12 && e.sprite.palette.len() >= 4, "palette {}", e.sprite.palette.len());
    // Without the mode the file opens as it is.
    let plain = unsafe { crate::formats::raster::abi::pixmith_import_image_ex(c_path.as_ptr(), std::ptr::null()) };
    assert!(!plain.is_null());
    assert_eq!(unsafe { Box::from_raw(plain) }.sprite.width as usize, w);
    // A mode that does not exist is an error.
    let bad = CString::new(r#"{"mode":"nope"}"#).unwrap();
    assert!(unsafe { crate::formats::raster::abi::pixmith_import_image_ex(c_path.as_ptr(), bad.as_ptr()) }.is_null());
    let _ = std::fs::remove_dir_all(&dir);
}
