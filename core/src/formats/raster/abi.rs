//! The C ABI of the image formats (declared in `pixmith_formats.h`).
//!
//! - `pixmith_import_image` / `pixmith_import_image_ex`: open any supported file as a new sprite, the format is
//!   detected from the content (the extension is only a hint for TGA, which has no signature).
//! - `pixmith_image_info`: what a file is and, for icons, which sizes it holds, so a front end can ask.
//! - `pixmith_export_image`: one frame (or all frames, for the animated formats) of a document as a file.
//! - `pixmith_export_run_ex`: the unified export of `pixmith_export_run` (tags, layers, trim, scale, ...) with the
//!   settings of the new formats passed as JSON.

use super::{encode_animation, encode_still, writes_animation, ImageFormat, ImageOptions, ImportOptions};
use crate::document::Editor;
use crate::ffi::{path_arg, set_error};
use crate::formats::export_plan::{from_c, render_outputs, Job, PixmithExport, PlanFrame};
use crate::formats::gif::{report, write_file};
use crate::formats::json::{self, Value};
use std::ffi::{c_char, CString};
use std::ptr::null_mut;

/// Frame number meaning "all frames" for `pixmith_export_image`.
pub const ALL_FRAMES: u32 = u32::MAX;

unsafe fn text_arg(p: *const c_char) -> String {
    path_arg(p).unwrap_or_default()
}

fn open(path: *const c_char, options: &str) -> Result<crate::document::Sprite, String> {
    let path = unsafe { path_arg(path) }.ok_or("invalid path")?;
    let opts = ImportOptions::from_json(options)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    super::sprite_from_file_bytes(&bytes, &path, &opts).map_err(|e| format!("{path}: {e}"))
}

fn into_editor(result: Result<crate::document::Sprite, String>) -> *mut Editor {
    match result {
        Ok(s) => Box::into_raw(Box::new(Editor::new(s))),
        Err(e) => {
            set_error(e);
            null_mut()
        }
    }
}

/// Opens an image file (PNG, GIF, WebP, JPEG, BMP, TGA, ICO, PCX, FLI/FLC) as a new sprite; animations get one
/// frame per image. An icon opens at its largest size. Returns null on failure (see `pixmith_last_error`).
#[no_mangle]
pub unsafe extern "C" fn pixmith_import_image(path: *const c_char) -> *mut Editor {
    crate::ffi::guarded("pixmith_import_image", null_mut(), || into_editor(open(path, "")))
}

/// Like `pixmith_import_image`, with options as JSON: `{"ico_size": 32}` opens that size of an icon.
#[no_mangle]
pub unsafe extern "C" fn pixmith_import_image_ex(path: *const c_char, options_json: *const c_char) -> *mut Editor {
    crate::ffi::guarded("pixmith_import_image_ex", null_mut(), || into_editor(open(path, &text_arg(options_json))))
}

/// Describes an image file as JSON: `{"format": "ico", "name": "ICO", "animated": false, "sizes": [16, 32, 256]}`
/// (`sizes` only for icons, ascending, the larger side of each image). Free the result with
/// `pixmith_string_free`. Returns null if the file cannot be read or is not an image.
#[no_mangle]
pub unsafe extern "C" fn pixmith_image_info(path: *const c_char) -> *mut c_char {
    crate::ffi::guarded("pixmith_image_info", null_mut(), || {
        let result = path_arg(path).ok_or("invalid path".to_string()).and_then(|p| {
            let bytes = std::fs::read(&p).map_err(|e| format!("{p}: {e}"))?;
            let fmt = super::detect(&bytes)
                .or_else(|| ImageFormat::from_path(&p))
                .ok_or(format!("{p}: not a supported image"))?;
            let mut members = vec![
                ("format".to_string(), Value::Str(fmt.extension().to_string())),
                ("name".to_string(), Value::Str(fmt.name().to_string())),
                ("animated".to_string(), Value::Bool(fmt.is_animated())),
            ];
            if fmt == ImageFormat::Ico {
                let mut sizes: Vec<u32> = super::ico::entries(&bytes)
                    .map_err(|e| format!("{p}: {e}"))?
                    .iter()
                    .map(|e| e.width.max(e.height) as u32)
                    .collect();
                sizes.sort_unstable();
                sizes.dedup();
                members.push(("sizes".into(), Value::Arr(sizes.into_iter().map(|s| Value::Num(s as f64)).collect())));
            }
            Ok(json::to_string(&Value::Obj(members)))
        });
        match result {
            Ok(text) => CString::new(text).map_or(null_mut(), CString::into_raw),
            Err(e) => {
                set_error(e);
                null_mut()
            }
        }
    })
}

fn one_frame(s: &crate::document::Sprite, frame: usize) -> PlanFrame {
    PlanFrame {
        rgba: s.render(frame),
        w: s.width as usize,
        h: s.height as usize,
        off: (0, 0),
        trimmed: false,
        duration: s.durations[frame],
        src_frame: frame,
    }
}

/// Writes `frame` of the document (0-based) at `scale` as an image file. The format comes from the options
/// (`"format": "webp"`) or else from the file extension. `ALL_FRAMES` (0xFFFFFFFF) as the frame writes the whole
/// animation in the animated formats (WebP, FLC); for the others it means the first frame. WebP is written as one
/// still frame when `"animated": false`. `options_json` may be null or empty; see `pixmith_formats.h`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_export_image(
    doc: *const Editor,
    frame: u32,
    scale: u32,
    path: *const c_char,
    options_json: *const c_char,
) -> bool {
    crate::ffi::guarded("pixmith_export_image", null_mut(), || {
        let (Some(ed), Some(path)) = (crate::ffi::synced(doc), path_arg(path)) else { return false };
        let options = text_arg(options_json);
        report((|| {
            let o = ImageOptions::from_json(&options)?;
            let named = if options.trim().is_empty() {
                None
            } else {
                json::parse(&options)?.get("format").and_then(Value::as_str).map(str::to_string)
            };
            let fmt = match named {
                Some(n) => ImageFormat::from_name(&n).ok_or(format!("unknown format \"{n}\""))?,
                None => {
                    ImageFormat::from_path(&path).ok_or("the file name does not say which image format to write")?
                }
            };
            if fmt == ImageFormat::Gif {
                return Err("export GIF with pixmith_export_gif".into());
            }
            let s = &ed.sprite;
            let all = frame == ALL_FRAMES;
            let data = if all && writes_animation(fmt, &o) {
                encode_animation(fmt, &Job::from_sprite(s), scale as usize, &o)?
            } else {
                let f = if all { 0 } else { frame as usize };
                if f >= s.frame_count() {
                    return Err(format!("frame {} does not exist (the sprite has {})", f + 1, s.frame_count()));
                }
                let o = ImageOptions { webp_animated: false, ..o };
                encode_still(fmt, &one_frame(s, f), scale as usize, &o)?
            };
            write_file(&path, &data)
        })())
    })
}

/// `pixmith_export_run` with the settings of the image formats as JSON (null or empty: the defaults). The
/// `format` field of the options struct also accepts the values 5 WebP, 6 JPEG, 7 BMP, 8 TGA, 9 ICO, 10 PCX and
/// 11 FLC. Returns the number of files written, or -1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_export_run_ex(
    doc: *const Editor,
    opts: *const PixmithExport,
    options_json: *const c_char,
    path: *const c_char,
) -> i32 {
    crate::ffi::guarded("pixmith_export_run_ex", null_mut(), || {
        let (Some(ed), Some(c), Some(path)) = (crate::ffi::synced(doc), opts.as_ref(), path_arg(path)) else {
            return -1;
        };
        let image = match ImageOptions::from_json(&text_arg(options_json)) {
            Ok(i) => i,
            Err(e) => {
                set_error(e);
                return -1;
            }
        };
        let Some((fmt, o, mut e)) = from_c(c) else {
            set_error("unknown export format");
            return -1;
        };
        e.image = image;
        let mut count = 0;
        let ok = report(render_outputs(&ed.sprite, fmt, &o, &e, &path, None).and_then(|files| {
            for (p, data) in &files {
                write_file(p, data)?;
                count += 1;
            }
            Ok(())
        }));
        if ok {
            count
        } else {
            -1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Sprite;
    use std::ffi::CStr;

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pixmith-raster-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn export_then_import_through_the_c_functions() {
        let dir = scratch("abi");
        let mut s = Sprite::new(5, 4);
        s.draw_line(0, 0, 0, 0, 4, 3, [200, 30, 40, 255], 1);
        let ed = Box::into_raw(Box::new(Editor::new(s)));
        for (name, opts) in [
            ("a.bmp", ""),
            ("a.tga", ""),
            ("a.pcx", ""),
            ("a.ico", r#"{"ico_sizes":[16,32]}"#),
            ("a.webp", ""),
            ("a.jpg", r#"{"quality": 100}"#),
            ("a.flc", ""),
            ("noext", r#"{"format":"bmp"}"#),
        ] {
            let path = c(dir.join(name).to_str().unwrap());
            let json = c(opts);
            assert!(unsafe { pixmith_export_image(ed, 0, 1, path.as_ptr(), json.as_ptr()) }, "{name}: {:?}", unsafe {
                CStr::from_ptr(crate::ffi::pixmith_last_error())
            });
            let back = unsafe { pixmith_import_image(path.as_ptr()) };
            assert!(!back.is_null(), "{name}");
            unsafe { drop(Box::from_raw(back)) };
        }
        // Unknown extension and no format: an error, not a crash.
        let path = c(dir.join("x.xyz").to_str().unwrap());
        assert!(!unsafe { pixmith_export_image(ed, 0, 1, path.as_ptr(), std::ptr::null()) });
        // The info call reports the icon sizes.
        let ico = c(dir.join("a.ico").to_str().unwrap());
        let info = unsafe { pixmith_image_info(ico.as_ptr()) };
        let text = unsafe { CStr::from_ptr(info).to_string_lossy().into_owned() };
        unsafe { crate::ffi::pixmith_string_free(info) };
        assert!(text.contains("\"sizes\":[16,32]"), "{text}");
        // Picking an icon size.
        let opt = c(r#"{"ico_size": 16}"#);
        let small = unsafe { pixmith_import_image_ex(ico.as_ptr(), opt.as_ptr()) };
        assert!(!small.is_null());
        assert_eq!(unsafe { (*small).sprite.width }, 16);
        unsafe {
            drop(Box::from_raw(small));
            drop(Box::from_raw(ed));
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_and_garbage_files_report_errors() {
        let dir = scratch("abi-bad");
        let missing = c(dir.join("nope.png").to_str().unwrap());
        assert!(unsafe { pixmith_import_image(missing.as_ptr()) }.is_null());
        std::fs::write(dir.join("junk.bin"), b"this is not an image at all").unwrap();
        let junk = c(dir.join("junk.bin").to_str().unwrap());
        assert!(unsafe { pixmith_import_image(junk.as_ptr()) }.is_null());
        assert!(unsafe { pixmith_image_info(junk.as_ptr()) }.is_null());
        assert!(unsafe { pixmith_import_image(std::ptr::null()) }.is_null());
        let _ = std::fs::remove_dir_all(dir);
    }
}
