//! Core calls for `.pxmt` documents and projects (`pixmith_core::project`, `pixmith_core::formats::pxmt`): the
//! project folder (canvases, order, trash), file thumbnails and File > Export As Aseprite. These are the safe
//! calls the project screens use; `Doc::open` and `Doc::save` already choose `.pxmt` or `.aseprite` themselves.

use crate::doc::Doc;
use pixmith_core::formats::pxmt;
use pixmith_core::project as core;
use std::path::{Path, PathBuf};

/// The extension of Pixmith's own document format.
pub const PXMT: &str = "pxmt";

/// Is `path` a document Pixmith can open as a canvas (`.pxmt`, `.aseprite`, `.ase`)?
pub fn is_canvas_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        e.eq_ignore_ascii_case(PXMT) || e.eq_ignore_ascii_case("aseprite") || e.eq_ignore_ascii_case("ase")
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Pxmt,
    Aseprite,
}

/// One canvas file of a project as `pixmith_project_scan` reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    pub path: PathBuf,
    /// File name without the extension.
    pub name: String,
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub layers: u32,
    /// Seconds since the Unix epoch.
    pub modified: u64,
    pub size: u64,
    /// Why the file could not be read; the size fields are then 0.
    pub error: Option<String>,
}

impl Canvas {
    pub fn file_name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// A scanned project.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub name: String,
    pub folder: PathBuf,
    pub canvases: Vec<Canvas>,
}

impl Project {
    /// The newest modification time of any canvas (0 for an empty project).
    pub fn modified(&self) -> u64 {
        self.canvases.iter().map(|c| c.modified).max().unwrap_or(0)
    }
}

impl From<core::Canvas> for Canvas {
    fn from(c: core::Canvas) -> Canvas {
        let (info, error) = match c.info {
            Ok(i) => (Some(i), None),
            Err(e) => (None, Some(e)),
        };
        Canvas {
            path: c.path,
            name: c.name,
            format: if c.format == core::Format::Aseprite { Format::Aseprite } else { Format::Pxmt },
            width: info.map_or(0, |i| i.width),
            height: info.map_or(0, |i| i.height),
            frames: info.map_or(0, |i| i.frames),
            layers: info.map_or(0, |i| i.layers),
            modified: c.modified,
            size: c.size,
            error,
        }
    }
}

/// A preview image: straight-alpha RGBA.
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

// --- The safe API ------------------------------------------------------------------------------------

/// Makes `folder` a project named `name`, creating the folder if needed. Safe on an existing project.
pub fn create_project(folder: &Path, name: &str) -> Result<(), String> {
    core::create(folder, name)
}

pub fn is_project(folder: &Path) -> bool {
    core::is_project(folder)
}

/// Lists the project's canvases (any folder works; one without a project file is named after the folder). Reads
/// a header per file, so call it off the UI thread.
pub fn scan_project(folder: &Path) -> Result<Project, String> {
    let p = core::scan(folder)?;
    Ok(Project { name: p.name, folder: p.folder, canvases: p.canvases.into_iter().map(Canvas::from).collect() })
}

pub fn set_project_name(folder: &Path, name: &str) -> Result<(), String> {
    core::set_name(folder, name)
}

/// Saves the order of the canvases (file names).
pub fn set_order(folder: &Path, file_names: &[String]) -> Result<(), String> {
    core::set_order(folder, file_names)
}

/// A new blank canvas; the name is made unique ("Canvas", "Canvas 2", ...). Returns its path.
pub fn new_canvas(folder: &Path, name: &str, width: u32, height: u32) -> Result<PathBuf, String> {
    core::new_canvas(folder, name, width, height)
}

/// Copies a canvas next to itself ("hero copy.pxmt", ...).
pub fn duplicate_canvas(path: &Path) -> Result<PathBuf, String> {
    core::duplicate_canvas(path)
}

/// Renames a canvas file keeping its extension; an existing name is refused.
pub fn rename_canvas(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    core::rename_canvas(path, new_name)
}

/// Moves a canvas into the project's trash. Returns its path there.
pub fn trash_canvas(path: &Path) -> Result<PathBuf, String> {
    core::trash_canvas(path)
}

/// Moves a trashed canvas back into the project. Returns its restored path.
pub fn restore_canvas(trashed: &Path) -> Result<PathBuf, String> {
    core::restore_canvas(trashed)
}

/// Copies an `.aseprite`, `.ase` or `.png` file into the project as a `.pxmt` canvas.
pub fn import_into_project(folder: &Path, file: &Path) -> Result<PathBuf, String> {
    core::import(folder, file)
}

/// A preview of any document file fitted inside `max` x `max` pixels (small sprites are enlarged by whole
/// factors, so pixel art stays crisp).
pub fn file_thumbnail(path: &Path, max: u32) -> Result<Thumb, String> {
    let t = pxmt::file_thumbnail(path, max, max)?;
    Ok(Thumb { width: t.width, height: t.height, rgba: t.rgba })
}

/// The trash folder of a project.
pub fn trash_dir(folder: &Path) -> PathBuf {
    folder.join(".pixmith").join("trash")
}

/// The project a trashed canvas belongs to (`<project>/.pixmith/trash/<file>`).
pub fn project_of_trashed(trashed: &Path) -> Option<PathBuf> {
    trashed.parent()?.parent()?.parent().map(Path::to_path_buf)
}

/// Canvas files in a project's trash, newest first, with their file times.
pub fn trashed_canvases(folder: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(trash_dir(folder)) else { return Vec::new() };
    let mut out: Vec<(PathBuf, u64)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| is_canvas_file(p))
        .map(|p| (p.clone(), modified_secs(&p)))
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

pub(crate) fn modified_secs(path: &Path) -> u64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

impl Doc {
    /// Writes the document as `.aseprite` whatever the path's extension is (File > Export As Aseprite). The
    /// document keeps its own path and format.
    pub fn save_aseprite(&self, path: &Path) -> Result<(), String> {
        let c = std::ffi::CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| "path contains a NUL byte".to_string())?;
        // SAFETY: `self.raw()` is a live document and `c` outlives the call.
        if unsafe { pixmith_core::ffi::document::pixmith_save_aseprite(self.raw(), c.as_ptr()) } {
            Ok(())
        } else {
            Err(crate::doc::last_error())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pixmith-projects-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(p: &Project) -> Vec<String> {
        p.canvases.iter().map(Canvas::file_name).collect()
    }

    #[test]
    fn a_project_lists_canvases_in_saved_order_then_by_name() {
        let dir = scratch("scan").join("Game");
        create_project(&dir, "Game").unwrap();
        assert!(is_project(&dir));
        for n in ["zeta", "Alpha", "mid"] {
            new_canvas(&dir, n, 8, 6).unwrap();
        }
        let p = scan_project(&dir).unwrap();
        assert_eq!(p.name, "Game");
        assert_eq!(names(&p), ["Alpha.pxmt", "mid.pxmt", "zeta.pxmt"]);
        assert_eq!((p.canvases[0].width, p.canvases[0].height, p.canvases[0].frames), (8, 6, 1));
        set_order(&dir, &["zeta.pxmt".into(), "Alpha.pxmt".into()]).unwrap();
        assert_eq!(names(&scan_project(&dir).unwrap()), ["zeta.pxmt", "Alpha.pxmt", "mid.pxmt"]);
        set_project_name(&dir, "Renamed").unwrap();
        assert_eq!(scan_project(&dir).unwrap().name, "Renamed");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn new_canvases_get_unique_names() {
        let dir = scratch("unique");
        create_project(&dir, "P").unwrap();
        let a = new_canvas(&dir, "Canvas", 4, 4).unwrap();
        let b = new_canvas(&dir, "Canvas", 4, 4).unwrap();
        let c = new_canvas(&dir, "Canvas", 4, 4).unwrap();
        assert_eq!(
            [a, b, c].map(|p| p.file_name().unwrap().to_string_lossy().into_owned()),
            ["Canvas.pxmt", "Canvas 2.pxmt", "Canvas 3.pxmt"]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicate_rename_trash_and_restore() {
        let dir = scratch("ops");
        create_project(&dir, "P").unwrap();
        let hero = new_canvas(&dir, "hero", 5, 5).unwrap();
        let copy = duplicate_canvas(&hero).unwrap();
        assert_eq!(copy.file_name().unwrap(), "hero copy.pxmt");
        assert_eq!(duplicate_canvas(&hero).unwrap().file_name().unwrap(), "hero copy 2.pxmt");

        let renamed = rename_canvas(&copy, "villain").unwrap();
        assert_eq!(renamed.file_name().unwrap(), "villain.pxmt");
        assert!(rename_canvas(&renamed, "hero").is_err(), "an existing name is refused");
        assert!(rename_canvas(&renamed, "a/b").is_err());
        assert_eq!(rename_canvas(&renamed, "villain.pxmt").unwrap(), renamed, "the extension may be typed");

        let trashed = trash_canvas(&renamed).unwrap();
        assert!(!renamed.exists() && trashed.exists());
        assert_eq!(project_of_trashed(&trashed).as_deref(), Some(dir.as_path()));
        assert_eq!(trashed_canvases(&dir).len(), 1);
        assert!(!names(&scan_project(&dir).unwrap()).contains(&"villain.pxmt".to_string()));

        new_canvas(&dir, "villain", 3, 3).unwrap(); // the name is taken again
        let back = restore_canvas(&trashed).unwrap();
        assert_eq!(back.file_name().unwrap(), "villain 2.pxmt");
        assert!(trashed_canvases(&dir).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn importing_converts_to_a_pxmt_canvas() {
        let dir = scratch("import");
        let project = dir.join("P");
        create_project(&project, "P").unwrap();
        let png = dir.join("pic.png");
        Doc::new(7, 9).export_png(0, 1, &png).unwrap();
        let ase = dir.join("old.aseprite");
        Doc::new(10, 3).save_aseprite(&ase).unwrap();
        let a = import_into_project(&project, &png).unwrap();
        let b = import_into_project(&project, &ase).unwrap();
        assert_eq!(
            (a.file_name().unwrap().to_str(), b.file_name().unwrap().to_str()),
            (Some("pic.pxmt"), Some("old.pxmt"))
        );
        let p = scan_project(&project).unwrap();
        let size = |n: &str| p.canvases.iter().find(|c| c.file_name() == n).map(|c| (c.width, c.height));
        assert_eq!((size("pic.pxmt"), size("old.pxmt")), (Some((7, 9)), Some((10, 3))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn aseprite_files_are_canvases_too_and_unreadable_ones_are_flagged() {
        let dir = scratch("mixed");
        create_project(&dir, "P").unwrap();
        Doc::new(4, 4).save_aseprite(&dir.join("legacy.aseprite")).unwrap();
        std::fs::write(dir.join("broken.pxmt"), b"nope").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let p = scan_project(&dir).unwrap();
        assert_eq!(names(&p), ["broken.pxmt", "legacy.aseprite"]);
        assert!(p.canvases[0].error.is_some());
        assert_eq!(p.canvases[1].format, Format::Aseprite);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn thumbnails_fit_and_export_as_aseprite_keeps_the_document_path() {
        let dir = scratch("thumb");
        let big = dir.join("big.pxmt");
        let mut doc = Doc::new(100, 50);
        doc.save(&big).unwrap();
        let t = file_thumbnail(&big, 40).unwrap();
        assert!(t.width <= 40 && t.height <= 40 && t.width > 0, "{}x{}", t.width, t.height);
        assert_eq!(t.rgba.len(), (t.width * t.height * 4) as usize);
        assert_eq!(t.width, t.height * 2, "aspect kept");
        assert!(file_thumbnail(&dir.join("missing.pxmt"), 40).is_err());
        let out = dir.join("out.aseprite");
        doc.save_aseprite(&out).unwrap();
        assert!(out.exists());
        assert_eq!(doc.path.as_deref(), Some(big.as_path()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
