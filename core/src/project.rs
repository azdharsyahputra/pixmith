//! Projects: a folder of canvases (`*.pxmt`, and also `*.aseprite` / `*.ase` found there) plus a `.pixmith`
//! folder holding `project.json` (name, creation time, canvas order) and the project's trash.
//!
//! The Rust API takes and returns paths (the egui app calls it directly); the `pixmith_project_*` functions at
//! the end are the C ABI declared in `app/Sources/PixmithCore/pixmith_project.h`. Strings that cross the C ABI
//! are UTF-8. Every file is written through [`crate::formats::durable`], so a crash never leaves half a file.

use crate::document::Sprite;
use crate::ffi::{path_arg, set_error};
use crate::formats::json::{self, Value};
use crate::formats::sheet::json_str;
use crate::formats::{durable, import, pxmt};
use std::collections::HashSet;
use std::ffi::{c_char, CString};
use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

pub type Result<T> = std::result::Result<T, String>;

/// File extensions of canvases (lowercase), in the order name collisions are checked.
pub const CANVAS_EXTENSIONS: [&str; 3] = ["pxmt", "aseprite", "ase"];
const PROJECT_DIR: &str = ".pixmith";
const PROJECT_FILE: &str = "project.json";
const TRASH_DIR: &str = "trash";
const DEFAULT_CANVAS_NAME: &str = "Canvas";
/// Longest canvas file name (without extension) in bytes; keeps names far from every file system's limit.
const MAX_NAME_BYTES: usize = 200;

// MARK: Types

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Pxmt,
    Aseprite,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Pxmt => "pxmt",
            Format::Aseprite => "aseprite",
        }
    }

    /// The format of a canvas file, from its extension (case-insensitive).
    pub fn of_path(path: &Path) -> Option<Format> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "pxmt" => Some(Format::Pxmt),
            "aseprite" | "ase" => Some(Format::Aseprite),
            _ => None,
        }
    }
}

/// What the project browser shows about a canvas, read without decoding its pixels where the format allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanvasInfo {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub layers: u32,
}

#[derive(Clone, Debug)]
pub struct Canvas {
    /// Absolute path.
    pub path: PathBuf,
    /// The file name without its extension.
    pub name: String,
    /// The file name, for [`set_order`].
    pub file_name: String,
    pub format: Format,
    /// Seconds since the Unix epoch (0 if unknown).
    pub modified: u64,
    /// File size in bytes.
    pub size: u64,
    /// The canvas facts, or why the file could not be read.
    pub info: std::result::Result<CanvasInfo, String>,
}

#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    /// Absolute path.
    pub folder: PathBuf,
    /// In the project's saved order, then new files by name.
    pub canvases: Vec<Canvas>,
}

impl Canvas {
    fn to_json(&self) -> String {
        let mut o = format!(
            "{{\"path\":{},\"name\":{},\"format\":{}",
            json_str(&self.path.to_string_lossy()),
            json_str(&self.name),
            json_str(self.format.as_str())
        );
        match &self.info {
            Ok(i) => write!(
                o,
                ",\"width\":{},\"height\":{},\"frames\":{},\"layers\":{},\"modified\":{},\"size\":{}}}",
                i.width, i.height, i.frames, i.layers, self.modified, self.size
            ),
            Err(e) => write!(o, ",\"error\":{},\"modified\":{},\"size\":{}}}", json_str(e), self.modified, self.size),
        }
        .unwrap();
        o
    }
}

impl Project {
    /// The document documented in `pixmith_project.h`.
    pub fn to_json(&self) -> String {
        let canvases: Vec<String> = self.canvases.iter().map(Canvas::to_json).collect();
        format!(
            "{{\"name\":{},\"folder\":{},\"canvases\":[{}]}}",
            json_str(&self.name),
            json_str(&self.folder.to_string_lossy()),
            canvases.join(",")
        )
    }
}

// MARK: project.json

#[derive(Default)]
struct Config {
    name: Option<String>,
    created: u64,
    order: Vec<String>,
    /// Members this version does not know; kept when the file is written again.
    extra: Vec<(String, Value)>,
}

impl Config {
    fn parse(text: &str) -> Option<Config> {
        let v = json::parse(text).ok()?;
        v.as_object()?;
        const KNOWN: [&str; 4] = ["v", "name", "created", "order"];
        Some(Config {
            name: v.get("name").and_then(Value::as_str).map(str::to_string),
            created: match v.get("created") {
                Some(Value::Num(n)) if *n >= 0.0 => *n as u64,
                _ => 0,
            },
            order: v
                .get("order")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            extra: v
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, _)| !KNOWN.contains(&k.as_str()))
                .map(|(k, x)| (k.clone(), x.clone()))
                .collect(),
        })
    }

    fn to_json(&self) -> String {
        let order: Vec<String> = self.order.iter().map(|n| json_str(n)).collect();
        let mut o = format!(
            "{{\"v\":1,\"name\":{},\"created\":{},\"order\":[{}]",
            json_str(self.name.as_deref().unwrap_or("")),
            self.created,
            order.join(",")
        );
        for (k, v) in &self.extra {
            write!(o, ",{}:{}", json_str(k), json::to_string(v)).unwrap();
        }
        o.push_str("}\n");
        o
    }
}

fn config_path(folder: &Path) -> PathBuf {
    folder.join(PROJECT_DIR).join(PROJECT_FILE)
}

/// The project file, or `None` when the folder is not a project or the file is unreadable.
fn load_config(folder: &Path) -> Option<Config> {
    Config::parse(&fs::read_to_string(config_path(folder)).ok()?)
}

fn save_config(folder: &Path, c: &Config) -> Result<()> {
    let dir = folder.join(PROJECT_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    durable::write_file(&utf8(&config_path(folder))?, c.to_json().as_bytes())
}

/// Changes the saved order after a file was renamed, copied or removed. A folder without a project file is left alone.
fn update_order(folder: &Path, f: impl FnOnce(&mut Vec<String>)) {
    if let Some(mut c) = load_config(folder) {
        let before = c.order.clone();
        f(&mut c.order);
        if c.order != before {
            // The order is a convenience; failing to save it must not fail the file operation.
            let _ = save_config(folder, &c);
        }
    }
}

fn folder_name(folder: &Path) -> String {
    folder.file_name().map_or_else(|| "Project".to_string(), |n| n.to_string_lossy().into_owned())
}

// MARK: Paths and names

fn absolute(p: &Path) -> Result<PathBuf> {
    std::path::absolute(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn utf8(p: &Path) -> Result<String> {
    p.to_str().map(str::to_string).ok_or_else(|| format!("{}: the path is not valid UTF-8", p.display()))
}

fn file_name_of(p: &Path) -> Result<String> {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{}: not a canvas file", p.display()))
}

/// The canvas file at `path`: it must exist and have a canvas extension.
fn existing_canvas(path: &Path) -> Result<(PathBuf, Format)> {
    let path = absolute(path)?;
    let format = Format::of_path(&path)
        .ok_or_else(|| format!("{}: not a canvas file (.pxmt, .aseprite, .ase)", path.display()))?;
    if !path.is_file() {
        return Err(format!("{}: no such canvas", path.display()));
    }
    Ok((path, format))
}

const WINDOWS_RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
    "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

fn is_bad_name_char(c: char) -> bool {
    matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*') || (c as u32) < 0x20 || c as u32 == 0x7f
}

/// A name the user typed for a canvas, checked for use as a file name on every platform: no empty names, no
/// `/ \ :` (nor the other characters Windows forbids), no leading or trailing dot, no Windows device names.
fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("the name is empty".into());
    }
    if let Some(c) = name.chars().find(|&c| is_bad_name_char(c)) {
        return Err(if c.is_control() {
            "the name cannot contain control characters".into()
        } else {
            format!("the name cannot contain \"{c}\"")
        });
    }
    if name.starts_with('.') || name.ends_with('.') {
        return Err("the name cannot start or end with a dot".into());
    }
    if name.len() > MAX_NAME_BYTES {
        return Err("the name is too long".into());
    }
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_lowercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        return Err(format!("\"{name}\" is a reserved file name"));
    }
    Ok(name.to_string())
}

/// Turns any text into a valid canvas name: forbidden characters become `-`, an empty result becomes "Canvas".
fn clean_stem(name: &str) -> String {
    let mut s: String = name.trim().chars().map(|c| if is_bad_name_char(c) { '-' } else { c }).collect();
    for ext in CANVAS_EXTENSIONS {
        if let Some(stem) = strip_extension(&s, ext) {
            s = stem.to_string();
            break;
        }
    }
    let mut s = s.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    while s.len() > MAX_NAME_BYTES {
        s.pop();
    }
    let s = s.trim_end().to_string();
    let reserved = s.split('.').next().unwrap_or("").trim_end().to_ascii_lowercase();
    if s.is_empty() || WINDOWS_RESERVED.contains(&reserved.as_str()) {
        return if s.is_empty() { DEFAULT_CANVAS_NAME.into() } else { format!("{s}-") };
    }
    s
}

fn strip_extension<'a>(name: &'a str, ext: &str) -> Option<&'a str> {
    let n = ext.len() + 1;
    (name.len() > n && name.as_bytes()[name.len() - n..].eq_ignore_ascii_case(format!(".{ext}").as_bytes()))
        .then(|| &name[..name.len() - n])
}

/// Lowercase file names in `dir`: the basis for collision checks that hold on case-insensitive file systems too.
fn taken_names(dir: &Path) -> HashSet<String> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_lowercase))
        .collect()
}

fn name_taken(taken: &HashSet<String>, stem: &str) -> bool {
    let stem = stem.to_lowercase();
    CANVAS_EXTENSIONS.iter().any(|e| taken.contains(&format!("{stem}.{e}")))
}

/// `dir/stem.ext`, or `dir/stem 2.ext`, `stem 3.ext`, ... when a canvas of that name exists in any format. A stem
/// that already ends in a number continues counting ("hero 2" -> "hero 3", not "hero 2 2").
fn unique_path(dir: &Path, stem: &str, ext: &str) -> Result<PathBuf> {
    let taken = taken_names(dir);
    let (base, first) = match stem.rsplit_once(' ') {
        Some((b, n)) if !b.is_empty() && n.parse::<u32>().is_ok_and(|n| n >= 2) => (b, n.parse::<u32>().unwrap_or(1)),
        _ => (stem, 1),
    };
    if !name_taken(&taken, stem) {
        return Ok(dir.join(format!("{stem}.{ext}")));
    }
    for n in first.saturating_add(1)..first.saturating_add(100_000) {
        let candidate = format!("{base} {n}");
        if !name_taken(&taken, &candidate) {
            return Ok(dir.join(format!("{candidate}.{ext}")));
        }
    }
    Err(format!("{}: no free name for \"{stem}\"", dir.display()))
}

fn unix_secs(t: std::io::Result<std::time::SystemTime>) -> u64 {
    t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

// MARK: Reading canvas facts

/// Width, height, frame and layer count of an `.aseprite` file from its header and the first frame's chunk
/// headers; pixel data is skipped over.
fn ase_info_fast(f: &mut fs::File) -> Option<CanvasInfo> {
    let u16le = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]) as u32;
    let mut head = [0u8; 144];
    f.read_exact(&mut head).ok()?;
    if u16le(&head, 4) != 0xA5E0 || u16le(&head, 128 + 4) != 0xF1FA {
        return None;
    }
    let (frames, width, height) = (u16le(&head, 6), u16le(&head, 8), u16le(&head, 10));
    let old = u16le(&head, 128 + 6);
    let new = u32::from_le_bytes(head[140..144].try_into().ok()?);
    let count = if new != 0 { new } else { old };
    let mut layers = 0;
    for _ in 0..count.min(1 << 20) {
        let mut h = [0u8; 6];
        f.read_exact(&mut h).ok()?;
        let size = u32::from_le_bytes(h[..4].try_into().ok()?);
        if u16::from_le_bytes([h[4], h[5]]) == 0x2004 {
            layers += 1;
        }
        f.seek(SeekFrom::Current(i64::from(size.checked_sub(6)?))).ok()?;
    }
    Some(CanvasInfo { width, height, frames, layers })
}

fn info_from_sprite(s: &Sprite) -> CanvasInfo {
    CanvasInfo { width: s.width, height: s.height, frames: s.frame_count() as u32, layers: s.layers.len() as u32 }
}

/// Canvas facts for the project browser; reads only what is needed (the `META` chunk of a `.pxmt`, the leading
/// headers of an `.aseprite`) and falls back to decoding the document when that is not enough.
pub fn canvas_info(path: impl AsRef<Path>) -> Result<CanvasInfo> {
    let path = path.as_ref();
    let format = Format::of_path(path).ok_or("not a canvas file")?;
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut sniff = [0u8; 8];
    let is_pxmt = file.read_exact(&mut sniff).is_ok() && sniff == pxmt::MAGIC;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    // The content decides, not the extension.
    let fast = if is_pxmt {
        pxmt::read_meta(&mut file).map_err(|e| e.to_string())?.filter(|m| m.has_size()).map(|m| CanvasInfo {
            width: m.width,
            height: m.height,
            frames: m.frames,
            layers: m.layers,
        })
    } else if format == Format::Aseprite {
        ase_info_fast(&mut file).filter(|i| i.width > 0 && i.height > 0 && i.frames > 0)
    } else {
        None
    };
    match fast {
        Some(i) => Ok(i),
        None => {
            let bytes = fs::read(path).map_err(|e| e.to_string())?;
            pxmt::read_document(&bytes).map(|s| info_from_sprite(&s))
        }
    }
}

fn canvas_entry(path: PathBuf, file_name: String, format: Format) -> Canvas {
    let meta = fs::metadata(&path);
    let stem = Path::new(&file_name).file_stem().map_or(file_name.clone(), |s| s.to_string_lossy().into_owned());
    Canvas {
        info: canvas_info(&path),
        modified: meta.as_ref().map_or(0, |m| unix_secs(m.modified())),
        size: meta.as_ref().map_or(0, |m| m.len()),
        path,
        name: stem,
        file_name,
        format,
    }
}

// MARK: Project operations

/// True when `folder` has a `.pixmith/project.json`.
pub fn is_project(folder: impl AsRef<Path>) -> bool {
    config_path(folder.as_ref()).is_file()
}

/// Makes `folder` a project named `name` (the folder's own name if `name` is blank), creating the folder if
/// needed. An existing project is left exactly as it is.
pub fn create(folder: impl AsRef<Path>, name: &str) -> Result<()> {
    let folder = absolute(folder.as_ref())?;
    let dir = folder.join(PROJECT_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if load_config(&folder).is_some() {
        return Ok(());
    }
    let name = name.trim();
    let name = if name.is_empty() { folder_name(&folder) } else { name.to_string() };
    save_config(&folder, &Config { name: Some(name), created: pxmt::now_secs(), ..Default::default() })
}

/// The project in `folder`: its canvases in saved order, then new files by name. Any folder can be scanned; one
/// without a project file is named after the folder and has no saved order. Unreadable canvases are listed with
/// their error in [`Canvas::info`].
/// Orders names the way people expect: "Canvas" < "Canvas 2" < "Canvas 10" (digit runs compare as numbers).
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (da, db) = (run(&mut a), run(&mut b));
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| da.len().cmp(&db.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

pub fn scan(folder: impl AsRef<Path>) -> Result<Project> {
    let folder = absolute(folder.as_ref())?;
    let entries = fs::read_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let config = load_config(&folder).unwrap_or_default();
    let mut found: Vec<(String, PathBuf, Format)> = Vec::new();
    for entry in entries.flatten() {
        let Some(file_name) = entry.file_name().to_str().map(str::to_string) else { continue };
        let path = entry.path();
        let Some(format) = Format::of_path(&path) else { continue };
        if file_name.starts_with('.') || !path.is_file() {
            continue;
        }
        found.push((file_name, path, format));
    }
    found.sort_by(|a, b| {
        let stem = |n: &str| Path::new(n).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        natural_cmp(&stem(&a.0), &stem(&b.0)).then_with(|| a.0.cmp(&b.0))
    });
    let mut canvases = Vec::with_capacity(found.len());
    for name in &config.order {
        if let Some(i) = found.iter().position(|f| &f.0 == name) {
            let (file_name, path, format) = found.remove(i);
            canvases.push(canvas_entry(path, file_name, format));
        }
    }
    canvases.extend(found.into_iter().map(|(file_name, path, format)| canvas_entry(path, file_name, format)));
    let name = config.name.filter(|n| !n.is_empty()).unwrap_or_else(|| folder_name(&folder));
    Ok(Project { name, folder, canvases })
}

pub fn set_name(folder: impl AsRef<Path>, name: &str) -> Result<()> {
    let folder = absolute(folder.as_ref())?;
    if name.trim().is_empty() {
        return Err("the project name is empty".into());
    }
    let mut c = load_config(&folder).unwrap_or(Config { created: pxmt::now_secs(), ..Default::default() });
    c.name = Some(name.trim().to_string());
    save_config(&folder, &c)
}

/// Saves the order of canvases (file names) shown by the project browser.
pub fn set_order(folder: impl AsRef<Path>, file_names: &[String]) -> Result<()> {
    let folder = absolute(folder.as_ref())?;
    let mut c = load_config(&folder).unwrap_or(Config { created: pxmt::now_secs(), ..Default::default() });
    let mut seen = HashSet::new();
    c.order = file_names.iter().filter(|n| !n.is_empty() && seen.insert(n.to_string())).cloned().collect();
    save_config(&folder, &c)
}

/// A new blank `.pxmt` canvas; the name is made unique ("Canvas", "Canvas 2", ...). Returns its path.
pub fn new_canvas(folder: impl AsRef<Path>, name: &str, width: u32, height: u32) -> Result<PathBuf> {
    if width == 0 || height == 0 {
        return Err("a canvas needs a width and a height of at least 1".into());
    }
    let folder = absolute(folder.as_ref())?;
    fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let path = unique_path(&folder, &clean_stem(name), "pxmt")?;
    write_canvas(&path, &Sprite::new(width, height))?;
    Ok(path)
}

fn write_canvas(path: &Path, s: &Sprite) -> Result<()> {
    durable::write_file(&utf8(path)?, &pxmt::write(s)?)
}

/// Copies a canvas next to itself ("hero copy.pxmt", "hero copy 2.pxmt", ...), in the same format. Returns the
/// new path; the copy follows the original in the project's order.
pub fn duplicate_canvas(path: impl AsRef<Path>) -> Result<PathBuf> {
    let (path, _) = existing_canvas(path.as_ref())?;
    let dir = path.parent().ok_or("not a canvas file")?.to_path_buf();
    let file_name = file_name_of(&path)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("pxmt").to_string();
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // "hero copy" and "hero copy 3" duplicate to "hero copy 2", not "hero copy copy".
    let base = stem
        .strip_suffix(" copy")
        .or_else(|| {
            let (head, n) = stem.rsplit_once(' ')?;
            n.parse::<u32>().ok()?;
            head.strip_suffix(" copy")
        })
        .unwrap_or(&stem);
    let dest = unique_path(&dir, &format!("{base} copy"), &ext)?;
    let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    durable::write_file(&utf8(&dest)?, &bytes)?;
    let new_name = file_name_of(&dest)?;
    update_order(&dir, |order| {
        if let Some(i) = order.iter().position(|n| *n == file_name) {
            order.insert(i + 1, new_name);
        }
    });
    Ok(dest)
}

/// Renames a canvas file, keeping its extension; refuses a name that is invalid (empty, or containing `/ \ :`
/// and the other characters Windows forbids) or already taken. Returns the new path.
pub fn rename_canvas(path: impl AsRef<Path>, new_name: &str) -> Result<PathBuf> {
    let (path, _) = existing_canvas(path.as_ref())?;
    let dir = path.parent().ok_or("not a canvas file")?.to_path_buf();
    let old_file = file_name_of(&path)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("pxmt").to_string();
    let mut name = validate_name(new_name)?;
    // "hero.pxmt" typed for a .pxmt canvas means "hero".
    if let Some(stem) = strip_extension(&name, &ext) {
        name = validate_name(stem)?;
    }
    let new_file = format!("{name}.{ext}");
    if new_file == old_file {
        return Ok(path);
    }
    let mut taken = taken_names(&dir);
    // The canvas's own name is free to be re-spelled ("hero" -> "Hero").
    taken.remove(&old_file.to_lowercase());
    if name_taken(&taken, &name) {
        return Err(format!("a canvas named \"{name}\" already exists"));
    }
    let dest = dir.join(&new_file);
    fs::rename(&path, &dest).map_err(|e| format!("{}: {e}", path.display()))?;
    update_order(&dir, |order| order.iter_mut().filter(|n| **n == old_file).for_each(|n| *n = new_file.clone()));
    Ok(dest)
}

/// Moves a canvas into the project's `.pixmith/trash` (restorable). Returns the path inside the trash.
pub fn trash_canvas(path: impl AsRef<Path>) -> Result<PathBuf> {
    let (path, _) = existing_canvas(path.as_ref())?;
    let dir = path.parent().ok_or("not a canvas file")?.to_path_buf();
    let file_name = file_name_of(&path)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("pxmt").to_string();
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let trash = dir.join(PROJECT_DIR).join(TRASH_DIR);
    fs::create_dir_all(&trash).map_err(|e| format!("{}: {e}", trash.display()))?;
    let dest = unique_path(&trash, &stem, &ext)?;
    fs::rename(&path, &dest).map_err(|e| format!("{}: {e}", path.display()))?;
    update_order(&dir, |order| order.retain(|n| *n != file_name));
    Ok(dest)
}

/// The canvases in a project's trash, by file name.
pub fn trashed_canvases(folder: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let trash = absolute(folder.as_ref())?.join(PROJECT_DIR).join(TRASH_DIR);
    let mut v: Vec<PathBuf> = match fs::read_dir(&trash) {
        Ok(rd) => rd.flatten().map(|e| e.path()).filter(|p| Format::of_path(p).is_some() && p.is_file()).collect(),
        Err(_) => Vec::new(),
    };
    v.sort();
    Ok(v)
}

/// Moves a trashed canvas back into its project (under another name if its own is taken). Returns the new path.
pub fn restore_canvas(trashed_path: impl AsRef<Path>) -> Result<PathBuf> {
    let (path, _) = existing_canvas(trashed_path.as_ref())?;
    let trash = path.parent().ok_or("not in a project's trash")?;
    let project = trash
        .parent()
        .filter(|p| {
            trash.file_name().is_some_and(|n| n == TRASH_DIR) && p.file_name().is_some_and(|n| n == PROJECT_DIR)
        })
        .and_then(Path::parent)
        .ok_or_else(|| format!("{}: not in a project's trash", path.display()))?
        .to_path_buf();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("pxmt").to_string();
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let dest = unique_path(&project, &stem, &ext)?;
    fs::rename(&path, &dest).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(dest)
}

/// Copies an `.aseprite` / `.ase` / `.png` file (or a `.pxmt` from elsewhere) into the project as a new `.pxmt`
/// canvas named after the file. Returns the new path.
pub fn import(folder: impl AsRef<Path>, file: impl AsRef<Path>) -> Result<PathBuf> {
    let folder = absolute(folder.as_ref())?;
    let file = file.as_ref();
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "aseprite" | "ase" | "png" | "pxmt") {
        return Err(format!("{}: cannot import this kind of file (.aseprite, .ase, .png)", file.display()));
    }
    let bytes = fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let sprite = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        import::sprite_from_png(&bytes).and_then(|s| s.validate().map(|_| s))
    } else {
        pxmt::read_document(&bytes)
    }
    .map_err(|e| format!("{}: {e}", file.display()))?;
    fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let dest = unique_path(&folder, &clean_stem(&stem), "pxmt")?;
    write_canvas(&dest, &sprite)?;
    Ok(dest)
}

/// Converts a document file between formats by the output's extension (`.pxmt` or anything else as
/// `.aseprite`); the input is detected by content. Used by `pixmith-cli convert`.
pub fn convert(input: &str, output: &str) -> Result<()> {
    let s = pxmt::open_file(input)?;
    pxmt::save_file(output, &s)
}

// MARK: C ABI

fn out_string(r: Result<String>) -> *mut c_char {
    match r.and_then(|s| CString::new(s.replace('\0', "")).map_err(|e| e.to_string())) {
        Ok(c) => c.into_raw(),
        Err(e) => {
            set_error(e);
            null_mut()
        }
    }
}

fn out_path(r: Result<PathBuf>) -> *mut c_char {
    out_string(r.and_then(|p| utf8(&p)))
}

fn out_bool(r: Result<()>) -> bool {
    match r {
        Ok(()) => true,
        Err(e) => {
            set_error(e);
            false
        }
    }
}

unsafe fn path_in(p: *const c_char) -> Result<String> {
    path_arg(p).filter(|s| !s.is_empty()).ok_or_else(|| "invalid path".to_string())
}

/// Makes `folder` a project named `name`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_project_create(folder: *const c_char, name: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_project_create", std::ptr::null_mut(), || {
        out_bool(path_in(folder).and_then(|f| create(f, &path_arg(name).unwrap_or_default())))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_project_is_project(folder: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_project_is_project", std::ptr::null_mut(), || {
        path_arg(folder).is_some_and(|f| !f.is_empty() && is_project(f))
    })
}

/// The project as JSON (see `pixmith_project.h`); free with `pixmith_string_free`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_project_scan(folder: *const c_char) -> *mut c_char {
    crate::ffi::guarded("pixmith_project_scan", std::ptr::null_mut(), || {
        out_string(path_in(folder).and_then(scan).map(|p| p.to_json()))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_project_set_name(folder: *const c_char, name: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_project_set_name", std::ptr::null_mut(), || {
        out_bool(path_in(folder).and_then(|f| {
            let name = path_arg(name).ok_or("invalid name")?;
            set_name(f, &name)
        }))
    })
}

/// Saves the order of canvases: a JSON array of file names.
#[no_mangle]
pub unsafe extern "C" fn pixmith_project_set_order(folder: *const c_char, json_file_names: *const c_char) -> bool {
    crate::ffi::guarded("pixmith_project_set_order", std::ptr::null_mut(), || {
        out_bool(path_in(folder).and_then(|f| {
            let text = path_arg(json_file_names).ok_or("invalid order")?;
            let v = json::parse(&text)?;
            let items = v.as_array().ok_or("the order must be a JSON array of file names")?;
            let names: Option<Vec<String>> = items.iter().map(|x| x.as_str().map(str::to_string)).collect();
            set_order(f, &names.ok_or("the order must be a JSON array of file names")?)
        }))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_project_new_canvas(
    folder: *const c_char,
    name: *const c_char,
    width: u32,
    height: u32,
) -> *mut c_char {
    crate::ffi::guarded("pixmith_project_new_canvas", std::ptr::null_mut(), || {
        out_path(path_in(folder).and_then(|f| new_canvas(f, &path_arg(name).unwrap_or_default(), width, height)))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_project_duplicate_canvas(path: *const c_char) -> *mut c_char {
    crate::ffi::guarded("pixmith_project_duplicate_canvas", std::ptr::null_mut(), || {
        out_path(path_in(path).and_then(duplicate_canvas))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_project_rename_canvas(path: *const c_char, new_name: *const c_char) -> *mut c_char {
    crate::ffi::guarded("pixmith_project_rename_canvas", std::ptr::null_mut(), || {
        out_path(path_in(path).and_then(|p| {
            let name = path_arg(new_name).ok_or("invalid name")?;
            rename_canvas(p, &name)
        }))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_project_trash_canvas(path: *const c_char) -> *mut c_char {
    crate::ffi::guarded("pixmith_project_trash_canvas", std::ptr::null_mut(), || {
        out_path(path_in(path).and_then(trash_canvas))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_project_restore_canvas(trashed_path: *const c_char) -> *mut c_char {
    crate::ffi::guarded("pixmith_project_restore_canvas", std::ptr::null_mut(), || {
        out_path(path_in(trashed_path).and_then(restore_canvas))
    })
}

#[no_mangle]
pub unsafe extern "C" fn pixmith_project_import(folder: *const c_char, file: *const c_char) -> *mut c_char {
    crate::ffi::guarded("pixmith_project_import", std::ptr::null_mut(), || {
        out_path(path_in(folder).and_then(|f| import(f, path_in(file)?)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_naturally() {
        let mut v = vec!["canvas 10", "canvas 2", "canvas", "b", "a2", "a10"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a2", "a10", "b", "canvas", "canvas 2", "canvas 10"]);
    }

    #[test]
    fn names_are_checked_and_cleaned() {
        assert_eq!(validate_name("  hero  ").unwrap(), "hero");
        assert!(validate_name("a/b").is_err() && validate_name("a\\b").is_err() && validate_name("a:b").is_err());
        assert!(validate_name("").is_err() && validate_name(" \t").is_err());
        assert!(
            validate_name("nul").is_err() && validate_name("COM1.txt").is_err() && validate_name("console").is_ok()
        );
        assert_eq!(clean_stem("a/b\\c:d"), "a-b-c-d");
        assert_eq!(clean_stem("hero.PXMT"), "hero");
        assert_eq!(clean_stem("  . "), DEFAULT_CANVAS_NAME);
        assert_eq!(clean_stem("aux"), "aux-");
        assert_eq!(strip_extension("a.PxMt", "pxmt"), Some("a"));
        assert_eq!(strip_extension(".pxmt", "pxmt"), None);
    }

    #[test]
    fn unique_names_continue_counting() {
        let dir = std::env::temp_dir().join(format!("pixmith-project-unit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let name = |p: PathBuf| p.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name(unique_path(&dir, "a", "pxmt").unwrap()), "a.pxmt");
        for f in ["a.pxmt", "a 2.ASEPRITE", "a 3.ase"] {
            fs::write(dir.join(f), "x").unwrap();
        }
        assert_eq!(name(unique_path(&dir, "a", "pxmt").unwrap()), "a 4.pxmt");
        assert_eq!(name(unique_path(&dir, "A 2", "pxmt").unwrap()), "A 4.pxmt");
        assert_eq!(name(unique_path(&dir, "a 9", "pxmt").unwrap()), "a 9.pxmt");
        assert_eq!(name(unique_path(&dir, "a 1", "pxmt").unwrap()), "a 1.pxmt");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_shape_of_an_empty_project() {
        let p = Project { name: "N".into(), folder: PathBuf::from("/x"), canvases: Vec::new() };
        assert_eq!(p.to_json(), r#"{"name":"N","folder":"/x","canvases":[]}"#);
    }
}
