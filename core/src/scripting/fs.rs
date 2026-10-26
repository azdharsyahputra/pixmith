//! Lua's `io` and `os` libraries and Aseprite's `app.fs`, behind the permission check of `access.rs`.
//!
//! The standard `io` and `os` libraries are loaded and wrapped: every function that can reach a file
//! or a program asks first (`io.open`, `io.lines`, `io.input`, `io.output`, `io.popen`, `os.remove`,
//! `os.rename`, `os.execute`, `dofile`, `loadfile`), the ones that could end the host (`os.exit`) or
//! that depend on process state (`os.setlocale`, `io.stdin`) are removed, and `os.getenv` answers for
//! a short list of harmless variables only.

use super::access::{self, Access};
use super::Ctx;
use mlua::{Error, Function, Lua, MultiValue, Result as LuaResult, Table, Value};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Largest script file `dofile` / `loadfile` read.
const MAX_LUA_FILE: u64 = 16 << 20;
/// Most entries `app.fs.listFiles` returns.
const MAX_LISTED: usize = 100_000;

const ENV_ALLOWED: [&str; 14] = [
    "HOME",
    "USER",
    "USERNAME",
    "USERPROFILE",
    "TEMP",
    "TMP",
    "TMPDIR",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "LANG",
    "TERM",
    "PWD",
];

fn text(v: &Value, what: &str) -> LuaResult<String> {
    match v {
        Value::String(s) => Ok(s.to_str()?.to_string()),
        Value::Integer(i) => Ok(i.to_string()),
        _ => Err(Error::runtime(format!("{what}: expected a string"))),
    }
}

fn writes(mode: &str) -> bool {
    mode.contains(['w', 'a', '+'])
}

fn path_string(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

pub(super) fn install(lua: &Lua, ctx: &Rc<Ctx>) -> LuaResult<()> {
    let g = lua.globals();

    // MARK: io
    let io: Table = g.get("io")?;
    let input_set = Rc::new(Cell::new(false));
    let c = ctx.clone();
    let orig: Function = io.get("open")?;
    io.set(
        "open",
        lua.create_function(move |_, (p, mode): (Value, Option<String>)| {
            let p = text(&p, "io.open")?;
            let mode = mode.unwrap_or_else(|| "r".into());
            let acc = if writes(&mode) { Access::Write } else { Access::Read };
            let norm = access::check(&c, &p, acc)?;
            orig.call::<MultiValue>((path_string(norm), mode))
        })?,
    )?;
    let c = ctx.clone();
    let orig: Function = io.get("lines")?;
    io.set(
        "lines",
        lua.create_function(move |lua, mut a: MultiValue| {
            let Some(p) = a.pop_front().filter(|v| !matches!(v, Value::Nil)) else {
                return Err(Error::runtime("io.lines needs a file name (scripts have no standard input)"));
            };
            let norm = access::check(&c, &text(&p, "io.lines")?, Access::Read)?;
            a.push_front(Value::String(lua.create_string(path_string(norm))?));
            orig.call::<MultiValue>(a)
        })?,
    )?;
    let c = ctx.clone();
    let flag = input_set.clone();
    let orig: Function = io.get("input")?;
    io.set(
        "input",
        lua.create_function(move |lua, mut a: MultiValue| {
            match a.front() {
                Some(Value::String(s)) => {
                    let norm = access::check(&c, &s.to_str()?, Access::Read)?;
                    a.pop_front();
                    a.push_front(Value::String(lua.create_string(path_string(norm))?));
                    flag.set(true);
                }
                Some(Value::UserData(_)) => flag.set(true),
                _ => {}
            }
            orig.call::<MultiValue>(a)
        })?,
    )?;
    let c = ctx.clone();
    let orig: Function = io.get("output")?;
    io.set(
        "output",
        lua.create_function(move |lua, mut a: MultiValue| {
            if let Some(Value::String(s)) = a.front() {
                let norm = access::check(&c, &s.to_str()?, Access::Write)?;
                a.pop_front();
                a.push_front(Value::String(lua.create_string(path_string(norm))?));
            }
            orig.call::<MultiValue>(a)
        })?,
    )?;
    let flag = input_set;
    let orig: Function = io.get("read")?;
    io.set(
        "read",
        lua.create_function(move |_, a: MultiValue| {
            if !flag.get() {
                return Ok(MultiValue::from_iter([Value::Nil]));
            }
            orig.call::<MultiValue>(a)
        })?,
    )?;
    let c = ctx.clone();
    let orig: Function = io.get("popen")?;
    io.set(
        "popen",
        lua.create_function(move |_, a: MultiValue| {
            let cmd = text(a.front().unwrap_or(&Value::Nil), "io.popen")?;
            access::check(&c, &cmd, Access::Execute)?;
            orig.call::<MultiValue>(a)
        })?,
    )?;
    io.set("stdin", Value::Nil)?;

    // MARK: os
    let os: Table = g.get("os")?;
    let c = ctx.clone();
    let orig: Function = os.get("remove")?;
    os.set(
        "remove",
        lua.create_function(move |_, p: Value| {
            let norm = access::check(&c, &text(&p, "os.remove")?, Access::Write)?;
            orig.call::<MultiValue>(path_string(norm))
        })?,
    )?;
    let c = ctx.clone();
    let orig: Function = os.get("rename")?;
    os.set(
        "rename",
        lua.create_function(move |_, (a, b): (Value, Value)| {
            let from = access::check(&c, &text(&a, "os.rename")?, Access::Write)?;
            let to = access::check(&c, &text(&b, "os.rename")?, Access::Write)?;
            orig.call::<MultiValue>((path_string(from), path_string(to)))
        })?,
    )?;
    let c = ctx.clone();
    let orig: Function = os.get("execute")?;
    os.set(
        "execute",
        lua.create_function(move |_, a: MultiValue| {
            if let Some(cmd) = a.front().filter(|v| !matches!(v, Value::Nil)) {
                access::check(&c, &text(cmd, "os.execute")?, Access::Execute)?;
            }
            orig.call::<MultiValue>(a)
        })?,
    )?;
    let c = ctx.clone();
    let orig: Function = os.get("tmpname")?;
    os.set(
        "tmpname",
        lua.create_function(move |_, ()| {
            let name: String = orig.call(())?;
            c.shared.access.allow_tmp(access::normalize(Path::new(&name)));
            Ok(name)
        })?,
    )?;
    os.set(
        "getenv",
        lua.create_function(|_, name: String| {
            Ok(ENV_ALLOWED.contains(&name.as_str()).then(|| std::env::var(&name).ok()).flatten())
        })?,
    )?;
    os.set(
        "exit",
        lua.create_function(|_, _: MultiValue| -> LuaResult<()> {
            Err(Error::runtime("os.exit is not available in Pixmith scripts"))
        })?,
    )?;
    os.set("setlocale", Value::Nil)?;

    // MARK: dofile / loadfile
    let c = ctx.clone();
    g.set(
        "dofile",
        lua.create_function(move |lua, p: Value| {
            let (src, name) = read_lua_file(&c, &p, "dofile")?;
            lua.load(src).set_name(name).set_mode(mlua::chunk::ChunkMode::Text).call::<MultiValue>(())
        })?,
    )?;
    let c = ctx.clone();
    g.set(
        "loadfile",
        lua.create_function(move |lua, p: Value| match read_lua_file(&c, &p, "loadfile") {
            Ok((src, name)) => {
                match lua.load(src).set_name(name).set_mode(mlua::chunk::ChunkMode::Text).into_function() {
                    Ok(f) => Ok(MultiValue::from_iter([Value::Function(f)])),
                    Err(e) => Ok(MultiValue::from_iter([Value::Nil, Value::String(lua.create_string(e.to_string())?)])),
                }
            }
            Err(e) if e.to_string().contains("permission denied") => Err(e),
            Err(e) => Ok(MultiValue::from_iter([Value::Nil, Value::String(lua.create_string(e.to_string())?)])),
        })?,
    )?;

    // MARK: app.fs
    install_fs(lua, ctx)
}

fn read_lua_file(ctx: &Ctx, p: &Value, what: &str) -> LuaResult<(String, String)> {
    let raw = text(p, what)?;
    let norm = access::check(ctx, &raw, Access::Read)?;
    let meta = std::fs::metadata(&norm).map_err(|e| Error::runtime(format!("cannot open {raw}: {e}")))?;
    if !meta.is_file() || meta.len() > MAX_LUA_FILE {
        return Err(Error::runtime(format!("cannot open {raw}: not a regular file of at most 16 MB")));
    }
    let bytes = std::fs::read(&norm).map_err(|e| Error::runtime(format!("cannot open {raw}: {e}")))?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), format!("@{raw}")))
}

// MARK: Path text

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// (directory without trailing separator, file name)
fn split_path(p: &str) -> (&str, &str) {
    match p.rfind(is_sep) {
        Some(0) => ("/", &p[1..]),
        Some(i) => (&p[..i], &p[i + 1..]),
        None => ("", p),
    }
}

fn title_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

/// Lexical normalization that keeps relative paths relative and uses `/`.
fn normalize_text(p: &str) -> String {
    let absolute = p.starts_with(is_sep);
    let mut parts: Vec<&str> = Vec::new();
    for c in p.split(is_sep) {
        match c {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    match (absolute, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => ".".into(),
        (false, false) => joined,
    }
}

fn join_text(parts: &[String]) -> String {
    let mut out = String::new();
    for p in parts.iter().filter(|p| !p.is_empty()) {
        if out.is_empty() {
            out = p.clone();
        } else {
            let trimmed = out.trim_end_matches(is_sep).to_string();
            out = format!("{}/{}", if trimmed.is_empty() { "" } else { &trimmed }, p.trim_start_matches(is_sep));
        }
    }
    out
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

fn install_fs(lua: &Lua, ctx: &Rc<Ctx>) -> LuaResult<()> {
    let fs = lua.create_table()?;
    fs.set("pathSeparator", std::path::MAIN_SEPARATOR.to_string())?;
    fs.set("filePath", lua.create_function(|_, p: String| Ok(split_path(&p).0.to_string()))?)?;
    fs.set("fileName", lua.create_function(|_, p: String| Ok(split_path(&p).1.to_string()))?)?;
    fs.set("fileExtension", lua.create_function(|_, p: String| Ok(title_ext(split_path(&p).1).1.to_string()))?)?;
    fs.set("fileTitle", lua.create_function(|_, p: String| Ok(title_ext(split_path(&p).1).0.to_string()))?)?;
    fs.set(
        "filePathAndTitle",
        lua.create_function(|_, p: String| {
            let (dir, name) = split_path(&p);
            let title = title_ext(name).0;
            Ok(if dir.is_empty() {
                title.to_string()
            } else if dir == "/" {
                format!("/{title}")
            } else {
                format!("{dir}/{title}")
            })
        })?,
    )?;
    fs.set("normalizePath", lua.create_function(|_, p: String| Ok(normalize_text(&p)))?)?;
    fs.set(
        "joinPath",
        lua.create_function(|_, a: MultiValue| {
            let parts = a.iter().map(|v| text(v, "joinPath")).collect::<LuaResult<Vec<_>>>()?;
            Ok(join_text(&parts))
        })?,
    )?;
    fs.set(
        "currentPath",
        lua.create_function(|_, ()| Ok(path_string(std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))))?,
    )?;
    fs.set(
        "appPath",
        lua.create_function(|_, ()| {
            Ok(path_string(
                std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default(),
            ))
        })?,
    )?;
    fs.set("tempPath", lua.create_function(|_, ()| Ok(path_string(std::env::temp_dir())))?)?;
    fs.set("userDocsPath", lua.create_function(|_, ()| Ok(path_string(home().join("Documents"))))?)?;
    fs.set(
        "userConfigPath",
        lua.create_function(|_, ()| {
            Ok(path_string(if cfg!(target_os = "macos") {
                home().join("Library").join("Application Support").join("Pixmith")
            } else {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map_or_else(|| home().join(".config"), PathBuf::from)
                    .join("pixmith")
            }))
        })?,
    )?;

    let c = ctx.clone();
    fs.set("isFile", lua.create_function(move |_, p: String| Ok(access::check(&c, &p, Access::Read)?.is_file()))?)?;
    let c = ctx.clone();
    fs.set("isDirectory", lua.create_function(move |_, p: String| Ok(access::check(&c, &p, Access::Read)?.is_dir()))?)?;
    let c = ctx.clone();
    fs.set(
        "fileSize",
        lua.create_function(move |_, p: String| {
            let path = access::check(&c, &p, Access::Read)?;
            Ok(std::fs::metadata(path).map(|m| m.len()).unwrap_or(0))
        })?,
    )?;
    let c = ctx.clone();
    fs.set(
        "listFiles",
        lua.create_function(move |lua, p: String| {
            let path = access::check(&c, &p, Access::Read)?;
            let mut names: Vec<String> = match std::fs::read_dir(path) {
                Ok(rd) => rd
                    .filter_map(|e| e.ok())
                    .take(MAX_LISTED)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect(),
                Err(_) => Vec::new(),
            };
            names.sort();
            lua.create_sequence_from(names)
        })?,
    )?;
    let c = ctx.clone();
    fs.set(
        "makeDirectory",
        lua.create_function(move |_, p: String| {
            let path = access::check(&c, &p, Access::Write)?;
            Ok(std::fs::create_dir(path).is_ok())
        })?,
    )?;
    let c = ctx.clone();
    fs.set(
        "makeAllDirectories",
        lua.create_function(move |_, p: String| {
            let path = access::check(&c, &p, Access::Write)?;
            Ok(std::fs::create_dir_all(path).is_ok())
        })?,
    )?;
    let c = ctx.clone();
    fs.set(
        "removeDirectory",
        lua.create_function(move |_, p: String| {
            let path = access::check(&c, &p, Access::Write)?;
            Ok(std::fs::remove_dir(path).is_ok())
        })?,
    )?;
    lua.globals().get::<Table>("app")?.set("fs", fs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_text() {
        assert_eq!(split_path("/a/b/c.png"), ("/a/b", "c.png"));
        assert_eq!(split_path("c.png"), ("", "c.png"));
        assert_eq!(split_path("/c.png"), ("/", "c.png"));
        assert_eq!(title_ext("c.tar.gz"), ("c.tar", "gz"));
        assert_eq!(title_ext(".hidden"), (".hidden", ""));
        assert_eq!(normalize_text("/a/./b/../c//d"), "/a/c/d");
        assert_eq!(normalize_text("a/../../b"), "../b");
        assert_eq!(normalize_text(""), ".");
        assert_eq!(normalize_text("/.."), "/");
        assert_eq!(join_text(&["/a/".into(), "/b".into(), "c.png".into()]), "/a/b/c.png");
        assert_eq!(join_text(&["".into(), "b".into()]), "b");
    }
}
