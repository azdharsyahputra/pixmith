//! C ABI of the extension manager (`pixmith_ext.h`). Results are JSON text released with `pixmith_string_free`.

use super::{extensions_dir, installed, refresh, Extension};
use crate::ffi::{path_arg, set_error};
use crate::formats::json::{self, Value};
use std::ffi::{c_char, CString};
use std::path::Path;
use std::ptr::null_mut;

fn out(text: String) -> *mut c_char {
    CString::new(text.replace('\0', "")).map_or(null_mut(), CString::into_raw)
}

fn array(items: Vec<Value>) -> *mut c_char {
    out(json::to_string(&Value::Arr(items)))
}

/// `<app data>/Extensions` as a path string (the folder may not exist yet).
#[no_mangle]
pub extern "C" fn pixmith_ext_dir() -> *mut c_char {
    crate::ffi::guarded("pixmith_ext_dir", null_mut(), || out(extensions_dir().to_string_lossy().into_owned()))
}

/// Reads the extension folder again; call after changes made outside the app and when Preferences opens.
#[no_mangle]
pub extern "C" fn pixmith_ext_refresh() {
    crate::ffi::guarded("pixmith_ext_refresh", null_mut(), refresh)
}

/// Every installed extension as a JSON array of
/// `{name, displayName, version, author, description, enabled, dir, error, scripts:[{name,path}],
/// palettes:[{name,colors}], themes:[{id,name,dark,light}]}`.
#[no_mangle]
pub extern "C" fn pixmith_ext_list() -> *mut c_char {
    crate::ffi::guarded("pixmith_ext_list", null_mut(), || array(installed().iter().map(Extension::to_json).collect()))
}

/// Installs or updates the `.pixmith-extension` package at `path`. Returns the extension as JSON (as one element
/// of `pixmith_ext_list`), or null with `pixmith_last_error` set; a rejected package leaves nothing behind.
#[no_mangle]
pub unsafe extern "C" fn pixmith_ext_install(path: *const c_char) -> *mut c_char {
    crate::ffi::guarded("pixmith_ext_install", null_mut(), || {
        let Some(path) = path_arg(path) else {
            set_error("invalid path");
            return null_mut();
        };
        match super::install(Path::new(&path)) {
            Ok(ext) => out(json::to_string(&ext.to_json())),
            Err(e) => {
                set_error(e);
                null_mut()
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_ext_set_enabled(name: *const c_char, enabled: bool) -> bool {
    crate::ffi::guarded("pixmith_ext_set_enabled", null_mut(), || {
        let Some(name) = path_arg(name) else {
            set_error("invalid name");
            return false;
        };
        super::set_enabled(&name, enabled).map_err(set_error).is_ok()
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_ext_uninstall(name: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_ext_uninstall", null_mut(), || {
        let Some(name) = path_arg(name) else {
            set_error("invalid name");
            return false;
        };
        super::uninstall(&name).map_err(set_error).is_ok()
    })
}

/// Themes of the enabled extensions as JSON: `[{id, name, extension, dark:{token:"RRGGBBAA"}, light:{...}}]`.
#[no_mangle]
pub extern "C" fn pixmith_ext_themes() -> *mut c_char {
    crate::ffi::guarded("pixmith_ext_themes", null_mut(), || {
        let list = installed()
            .iter()
            .filter(|e| e.active())
            .flat_map(|e| {
                e.themes.iter().map(|t| {
                    let Value::Obj(mut members) = t.theme.to_json() else { unreachable!() };
                    members.insert(0, ("id".into(), Value::Str(t.id.clone())));
                    members.push(("extension".into(), Value::Str(e.manifest.display_name.clone())));
                    Value::Obj(members)
                })
            })
            .collect();
        array(list)
    })
}

/// Scripts of the enabled extensions as JSON: `[{extension, name, path, dir, prefs}]` (`dir` is the extension's
/// folder, `prefs` the file for the plugin's preferences).
#[no_mangle]
pub extern "C" fn pixmith_ext_scripts() -> *mut c_char {
    crate::ffi::guarded("pixmith_ext_scripts", null_mut(), || {
        let list = super::scripts()
            .into_iter()
            .map(|(ext, s)| {
                Value::Obj(vec![
                    ("extension".into(), Value::Str(ext)),
                    ("name".into(), Value::Str(s.name)),
                    ("path".into(), Value::Str(s.path.to_string_lossy().into_owned())),
                    ("dir".into(), Value::Str(s.dir.to_string_lossy().into_owned())),
                    ("prefs".into(), Value::Str(s.prefs.to_string_lossy().into_owned())),
                ])
            })
            .collect();
        array(list)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    unsafe fn take(p: *mut c_char) -> String {
        assert!(!p.is_null());
        let s = CStr::from_ptr(p).to_string_lossy().into_owned();
        drop(CString::from_raw(p));
        s
    }

    /// The only test that points the process-wide support folder somewhere else.
    #[test]
    fn the_c_abi_round_trip_in_a_private_support_folder() {
        let base = std::env::temp_dir().join(format!("pixmith-ext-ffi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let _guard = crate::extensions::tests::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("PIXMITH_SUPPORT_DIR", &base);
        refresh();
        unsafe {
            assert_eq!(take(pixmith_ext_list()), "[]");
            let dir = take(pixmith_ext_dir());
            assert!(dir.starts_with(base.to_str().unwrap()) && dir.ends_with("Extensions"), "{dir}");

            let pkg = base.join("sample.pixmith-extension");
            std::fs::write(&pkg, crate::extensions::tests::sample_package()).unwrap();
            let c = CString::new(pkg.to_str().unwrap()).unwrap();
            let json = json::parse(&take(pixmith_ext_install(c.as_ptr()))).unwrap();
            assert_eq!(json.get("name").and_then(Value::as_str), Some("sample"));
            let list = json::parse(&take(pixmith_ext_list())).unwrap();
            assert_eq!(list.as_array().unwrap().len(), 1);

            let themes = json::parse(&take(pixmith_ext_themes())).unwrap();
            let t = &themes.as_array().unwrap()[0];
            assert_eq!(t.get("id").and_then(Value::as_str), Some("sample/themes/midnight.json"));
            assert_eq!(t.get("dark").and_then(|d| d.get("panel")).and_then(Value::as_str), Some("14141FFF"));
            let scripts = json::parse(&take(pixmith_ext_scripts())).unwrap();
            assert_eq!(scripts.as_array().unwrap().len(), 1);

            let name = CString::new("sample").unwrap();
            assert!(pixmith_ext_set_enabled(name.as_ptr(), false));
            assert_eq!(take(pixmith_ext_themes()), "[]", "a disabled extension contributes nothing");
            assert_eq!(take(pixmith_ext_scripts()), "[]");
            assert!(pixmith_ext_set_enabled(name.as_ptr(), true));
            assert!(crate::extensions::palettes().len() == 2);

            let bad = base.join("bad.pixmith-extension");
            std::fs::write(&bad, b"nope").unwrap();
            let c = CString::new(bad.to_str().unwrap()).unwrap();
            assert!(pixmith_ext_install(c.as_ptr()).is_null());
            assert!(!CStr::from_ptr(crate::ffi::pixmith_last_error()).to_bytes().is_empty());
            assert!(pixmith_ext_install(std::ptr::null()).is_null());

            assert!(pixmith_ext_uninstall(name.as_ptr()));
            assert!(!pixmith_ext_uninstall(name.as_ptr()));
            assert!(!pixmith_ext_uninstall(std::ptr::null()));
            assert_eq!(take(pixmith_ext_list()), "[]");
        }
        std::env::remove_var("PIXMITH_SUPPORT_DIR");
        refresh();
        let _ = std::fs::remove_dir_all(&base);
    }
}
