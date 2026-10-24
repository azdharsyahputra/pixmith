//! Projects: scan, order, new/duplicate/rename/trash/restore/import, through the Rust API and the C ABI.

use pixmith_core::document::Sprite;
use pixmith_core::formats::json::{self, Value};
use pixmith_core::formats::{ase, export, pxmt};
use pixmith_core::project::{self, Format};
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-project-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample(w: u32, h: u32) -> Sprite {
    let mut s = Sprite::new(w, h);
    s.draw_line(0, 0, 0, 0, 0, 0, [255, 0, 0, 255], 1);
    s.add_layer(0);
    s.add_frame(0, true);
    s
}

fn write_pxmt(path: &Path, s: &Sprite) {
    std::fs::write(path, pxmt::write(s).unwrap()).unwrap();
}

fn names(p: &project::Project) -> Vec<String> {
    p.canvases.iter().map(|c| c.file_name.clone()).collect()
}

fn files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    v.sort();
    v
}

#[test]
fn create_and_is_project() {
    let dir = temp_dir("create");
    let folder = dir.join("My Game").join("art");
    assert!(!project::is_project(&folder));
    project::create(&folder, "Hero Art").unwrap();
    assert!(project::is_project(&folder));
    assert!(folder.join(".pixmith/project.json").is_file());
    assert_eq!(project::scan(&folder).unwrap().name, "Hero Art");
    // Safe on an existing project: name, order and unknown members stay.
    project::set_order(&folder, &["b.pxmt".into(), "a.pxmt".into()]).unwrap();
    let raw = std::fs::read_to_string(folder.join(".pixmith/project.json")).unwrap();
    std::fs::write(folder.join(".pixmith/project.json"), raw.replace("\"order\"", "\"future\":{\"x\":[1]},\"order\""))
        .unwrap();
    project::create(&folder, "Other").unwrap();
    assert_eq!(project::scan(&folder).unwrap().name, "Hero Art");
    project::set_name(&folder, "Renamed \"Art\" \\ é").unwrap();
    let raw = std::fs::read_to_string(folder.join(".pixmith/project.json")).unwrap();
    assert!(raw.contains("\"future\":{\"x\":[1]}") && raw.contains("\"order\":[\"b.pxmt\",\"a.pxmt\"]"), "{raw}");
    assert_eq!(project::scan(&folder).unwrap().name, "Renamed \"Art\" \\ é");
    // A blank name falls back to the folder name; a corrupt project file is replaced.
    let other = dir.join("plain");
    project::create(&other, "  ").unwrap();
    assert_eq!(project::scan(&other).unwrap().name, "plain");
    std::fs::write(other.join(".pixmith/project.json"), "{ not json").unwrap();
    assert!(!project::scan(&other).unwrap().name.is_empty());
    project::create(&other, "Fresh").unwrap();
    assert_eq!(project::scan(&other).unwrap().name, "Fresh");
    assert!(project::set_name(&other, " ").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_lists_canvases_in_order_then_by_name() {
    let dir = temp_dir("scan");
    project::create(&dir, "P").unwrap();
    write_pxmt(&dir.join("b.pxmt"), &sample(8, 4));
    write_pxmt(&dir.join("A.pxmt"), &sample(2, 2));
    std::fs::write(dir.join("c.aseprite"), ase::write(&sample(5, 7))).unwrap();
    std::fs::write(dir.join("d.ASE"), ase::write(&sample(3, 3))).unwrap();
    std::fs::write(dir.join("notes.txt"), "x").unwrap();
    std::fs::write(dir.join("image.png"), "x").unwrap();
    std::fs::write(dir.join(".hidden.pxmt"), "x").unwrap();
    std::fs::write(dir.join("a.pxmt.pixmith-tmp"), "x").unwrap();
    std::fs::create_dir(dir.join("folder.pxmt")).unwrap();
    let p = project::scan(&dir).unwrap();
    assert_eq!(names(&p), ["A.pxmt", "b.pxmt", "c.aseprite", "d.ASE"]);
    let b = &p.canvases[1];
    assert_eq!((b.name.as_str(), b.format), ("b", Format::Pxmt));
    let info = b.info.as_ref().unwrap();
    assert_eq!((info.width, info.height, info.frames, info.layers), (8, 4, 2, 2));
    assert!(b.size > 0 && b.modified > 1_600_000_000 && b.path.is_absolute());
    let c = p.canvases[2].info.as_ref().unwrap();
    assert_eq!((c.width, c.height, c.frames, c.layers), (5, 7, 2, 2));
    assert_eq!(p.canvases[3].format, Format::Aseprite);

    project::set_order(&dir, &["d.ASE".into(), "gone.pxmt".into(), "b.pxmt".into(), "d.ASE".into()]).unwrap();
    assert_eq!(names(&project::scan(&dir).unwrap()), ["d.ASE", "b.pxmt", "A.pxmt", "c.aseprite"]);
    project::set_order(&dir, &[]).unwrap();
    assert_eq!(names(&project::scan(&dir).unwrap()), ["A.pxmt", "b.pxmt", "c.aseprite", "d.ASE"]);
    // A folder that is not a project scans too; a missing one does not.
    let plain = temp_dir("scan-plain");
    write_pxmt(&plain.join("x.pxmt"), &sample(1, 1));
    let q = project::scan(&plain).unwrap();
    assert_eq!(names(&q), ["x.pxmt"]);
    assert!(!q.name.is_empty());
    assert!(project::scan(plain.join("missing")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&plain);
}

#[test]
fn scan_json_has_the_documented_shape() {
    let dir = temp_dir("json");
    let folder = dir.join("Quote \"d\" \\ ünï");
    project::create(&folder, "Name with \"quotes\"\nand a newline").unwrap();
    write_pxmt(&folder.join("hero.pxmt"), &sample(32, 16));
    std::fs::write(folder.join("old.aseprite"), ase::write(&sample(4, 4))).unwrap();
    std::fs::write(folder.join("broken.pxmt"), b"PXMT\r\n\x1a\nzz").unwrap();
    std::fs::write(folder.join("empty.aseprite"), b"").unwrap();
    let text = project::scan(&folder).unwrap().to_json();
    let v = json::parse(&text).unwrap();
    assert_eq!(v.get("name").and_then(Value::as_str), Some("Name with \"quotes\"\nand a newline"));
    assert_eq!(v.get("folder").and_then(Value::as_str), Some(project::scan(&folder).unwrap().folder.to_str().unwrap()));
    let canvases = v.get("canvases").and_then(Value::as_array).unwrap();
    assert_eq!(canvases.len(), 4);
    let by = |n: &str| canvases.iter().find(|c| c.get("name").and_then(Value::as_str) == Some(n)).unwrap();
    let num = |c: &Value, k: &str| c.get(k).and_then(Value::as_f64).unwrap() as u64;
    let hero = by("hero");
    assert_eq!(hero.get("format").and_then(Value::as_str), Some("pxmt"));
    assert_eq!(hero.get("path").and_then(Value::as_str), Some(folder.join("hero.pxmt").to_str().unwrap()));
    assert_eq!((num(hero, "width"), num(hero, "height"), num(hero, "frames"), num(hero, "layers")), (32, 16, 2, 2));
    assert!(num(hero, "size") > 0 && num(hero, "modified") > 0);
    assert!(hero.get("error").is_none());
    assert_eq!(by("old").get("format").and_then(Value::as_str), Some("aseprite"));
    // Unreadable files are listed with an error instead of the size fields.
    for bad in ["broken", "empty"] {
        let c = by(bad);
        assert!(c.get("error").and_then(Value::as_str).is_some_and(|e| !e.is_empty()), "{bad}");
        assert!(c.get("width").is_none() && c.get("frames").is_none());
        assert!(c.get("path").is_some() && c.get("format").is_some());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cheap_info_agrees_with_decoding_for_every_fixture() {
    let dir = temp_dir("fixture-info");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aseprite");
    let mut n = 0;
    let mut stack = vec![src];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if matches!(p.extension().and_then(|e| e.to_str()), Some("aseprite" | "ase")) {
                n += 1;
                let sprite = ase::read(&std::fs::read(&p).unwrap()).unwrap();
                let want = (sprite.width, sprite.height, sprite.frame_count() as u32, sprite.layers.len() as u32);
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                let a = dir.join(&name);
                std::fs::copy(&p, &a).unwrap();
                let x = project::canvas_info(&a).unwrap();
                assert_eq!((x.width, x.height, x.frames, x.layers), want, "{name} (.aseprite)");
                let b = dir.join(format!("{name}.pxmt"));
                write_pxmt(&b, &sprite);
                let y = project::canvas_info(&b).unwrap();
                assert_eq!((y.width, y.height, y.frames, y.layers), want, "{name} (.pxmt)");
            }
        }
    }
    assert!(n >= 30);
    let p = project::scan(&dir).unwrap();
    assert_eq!(p.canvases.len(), n * 2);
    assert!(p.canvases.iter().all(|c| c.info.is_ok()));
    // The content decides, not the extension: a .pxmt file holding .aseprite data (and the reverse) still scans.
    let swapped = dir.join("swapped.pxmt");
    std::fs::copy(dir.join("basic-16x16.aseprite"), &swapped).unwrap();
    assert_eq!(project::canvas_info(&swapped).unwrap().width, 16);
    let swapped2 = dir.join("swapped2.aseprite");
    std::fs::copy(dir.join("basic-16x16.aseprite.pxmt"), &swapped2).unwrap();
    assert_eq!(project::canvas_info(&swapped2).unwrap().width, 16);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pxmt_with_a_corrupt_sprite_still_scans_cheaply_but_a_corrupt_header_is_an_error() {
    let dir = temp_dir("scan-damage");
    let path = dir.join("a.pxmt");
    write_pxmt(&path, &sample(6, 6));
    // META is intact: scanning does not decode the sprite, so it shows the canvas (opening it reports the damage).
    let mut bytes = std::fs::read(&path).unwrap();
    let at = bytes.windows(4).position(|w| w == b"SPRT").unwrap() + 30;
    bytes[at] ^= 0x44;
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(project::canvas_info(&path).unwrap().width, 6);
    assert!(pxmt::open_file(&path).is_err());
    // Without usable META the sprite is decoded instead.
    let parts = [pxmt::MAGIC.to_vec(), vec![1, 0], Vec::new()].concat();
    std::fs::write(&path, &parts).unwrap();
    assert!(project::canvas_info(&path).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_canvas_names_are_unique() {
    let dir = temp_dir("new");
    let a = project::new_canvas(&dir, "", 16, 16).unwrap();
    let b = project::new_canvas(&dir, "Canvas", 8, 8).unwrap();
    let c = project::new_canvas(&dir, "canvas", 8, 8).unwrap(); // case-insensitive collision
    let d = project::new_canvas(&dir, "Sky/Box: 1", 4, 4).unwrap();
    let e = project::new_canvas(&dir, "tiles.pxmt", 4, 4).unwrap();
    let f = project::new_canvas(&dir, "tiles", 4, 4).unwrap();
    assert_eq!(
        files(&dir),
        ["Canvas 2.pxmt", "Canvas.pxmt", "Sky-Box- 1.pxmt", "canvas 3.pxmt", "tiles 2.pxmt", "tiles.pxmt"]
    );
    assert_eq!(a, dir.join("Canvas.pxmt"));
    assert!(b.ends_with("Canvas 2.pxmt") && c.ends_with("canvas 3.pxmt") && d.ends_with("Sky-Box- 1.pxmt"));
    assert!(e.ends_with("tiles.pxmt") && f.ends_with("tiles 2.pxmt"));
    let doc = pxmt::open_file(&a).unwrap();
    assert_eq!((doc.width, doc.height, doc.frame_count(), doc.layers.len()), (16, 16, 1, 1));
    // The thumbnail exists even for a blank canvas.
    assert!(pxmt::read_thumbnail_file(&a).unwrap().is_some());
    // An .aseprite of the same name blocks the name too.
    std::fs::write(dir.join("hero.aseprite"), ase::write(&sample(2, 2))).unwrap();
    assert!(project::new_canvas(&dir, "hero", 2, 2).unwrap().ends_with("hero 2.pxmt"));
    // Zero size is refused; huge sizes are clamped by the document model; a missing folder is created.
    assert!(project::new_canvas(&dir, "z", 0, 5).is_err());
    assert!(project::new_canvas(&dir, "z", 5, 0).is_err());
    let big = project::new_canvas(&dir, "big", u32::MAX, u32::MAX).unwrap();
    assert!(pxmt::open_file(big).unwrap().width <= 65535);
    let nested = project::new_canvas(dir.join("a/b/c"), "x", 3, 3).unwrap();
    assert!(nested.is_file());
    // Reserved and odd names are made safe.
    assert!(project::new_canvas(&dir, "CON", 1, 1).unwrap().file_name().unwrap().to_str().unwrap().starts_with("CON-"));
    assert!(project::new_canvas(&dir, "...", 1, 1).unwrap().ends_with("Canvas 4.pxmt"));
    assert!(project::new_canvas(&dir, "  ", 1, 1).unwrap().ends_with("Canvas 5.pxmt"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_copies_next_to_the_original() {
    let dir = temp_dir("dup");
    project::create(&dir, "P").unwrap();
    let hero = project::new_canvas(&dir, "hero", 8, 8).unwrap();
    let other = project::new_canvas(&dir, "zed", 8, 8).unwrap();
    project::set_order(&dir, &["hero.pxmt".into(), "zed.pxmt".into()]).unwrap();
    let c1 = project::duplicate_canvas(&hero).unwrap();
    let c2 = project::duplicate_canvas(&hero).unwrap();
    let c3 = project::duplicate_canvas(&c1).unwrap();
    assert_eq!(files(&dir), ["hero copy 2.pxmt", "hero copy 3.pxmt", "hero copy.pxmt", "hero.pxmt", "zed.pxmt"]);
    assert!(c1.ends_with("hero copy.pxmt") && c2.ends_with("hero copy 2.pxmt") && c3.ends_with("hero copy 3.pxmt"));
    assert_eq!(std::fs::read(&hero).unwrap(), std::fs::read(&c1).unwrap());
    // The copy follows its original in the order.
    assert_eq!(
        names(&project::scan(&dir).unwrap()),
        ["hero.pxmt", "hero copy 2.pxmt", "hero copy.pxmt", "hero copy 3.pxmt", "zed.pxmt"]
    );
    // Other formats keep their extension; missing and non-canvas files are errors.
    std::fs::write(dir.join("old.aseprite"), ase::write(&sample(3, 3))).unwrap();
    assert!(project::duplicate_canvas(dir.join("old.aseprite")).unwrap().ends_with("old copy.aseprite"));
    assert!(project::duplicate_canvas(dir.join("missing.pxmt")).is_err());
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    assert!(project::duplicate_canvas(dir.join("a.txt")).is_err());
    assert!(project::duplicate_canvas(&dir).is_err());
    let _ = other;
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rename_rules() {
    let dir = temp_dir("rename");
    project::create(&dir, "P").unwrap();
    let a = project::new_canvas(&dir, "a", 4, 4).unwrap();
    let b = project::new_canvas(&dir, "b", 4, 4).unwrap();
    std::fs::write(dir.join("old.aseprite"), ase::write(&sample(3, 3))).unwrap();
    project::set_order(&dir, &["b.pxmt".into(), "a.pxmt".into()]).unwrap();

    let hero = project::rename_canvas(&a, "hero").unwrap();
    assert_eq!(hero, dir.join("hero.pxmt"));
    assert!(!a.exists() && hero.is_file());
    assert_eq!(
        names(&project::scan(&dir).unwrap()),
        ["b.pxmt", "hero.pxmt", "old.aseprite"],
        "order follows the rename"
    );
    // Extension kept, and a typed extension is not doubled.
    assert!(project::rename_canvas(&hero, "hero.pxmt").unwrap().ends_with("hero.pxmt"));
    assert!(project::rename_canvas(&hero, "Hero Two.PXMT").unwrap().ends_with("Hero Two.pxmt"));
    let r = project::rename_canvas(dir.join("old.aseprite"), "new look").unwrap();
    assert!(r.ends_with("new look.aseprite"), "{r:?}");
    // Case-only rename and renaming to itself work.
    let two = dir.join("Hero Two.pxmt");
    assert!(project::rename_canvas(&two, "hero two").unwrap().ends_with("hero two.pxmt"));
    assert!(project::rename_canvas(dir.join("hero two.pxmt"), "hero two").unwrap().ends_with("hero two.pxmt"));
    // Refusals: existing (any case, any canvas format), and invalid names.
    let two = dir.join("hero two.pxmt");
    assert!(project::rename_canvas(&two, "b").unwrap_err().contains("already exists"));
    assert!(project::rename_canvas(&two, "B").unwrap_err().contains("already exists"));
    assert!(project::rename_canvas(&two, "new look").unwrap_err().contains("already exists"));
    for bad in [
        "",
        "   ",
        "a/b",
        "a\\b",
        "c:d",
        "x:",
        "..",
        ".",
        ".hidden",
        "dots.",
        "a|b",
        "a?b",
        "a*b",
        "a<b",
        "a>b",
        "a\"b",
        "tab\there",
        "nul\0x",
        "CON",
        "con.txt",
        "Lpt1",
        "COM9 ",
        ".pxmt",
        "../up",
        "C:\\Users\\x",
    ] {
        let err = project::rename_canvas(&two, bad);
        assert!(err.is_err(), "{bad:?} was accepted: {err:?}");
        assert!(two.is_file());
    }
    assert!(project::rename_canvas(&two, &"x".repeat(300)).is_err());
    // Windows-style but legal names work.
    for ok in ["My Sprite (final) [v2]", "Привет мир", "日本語 キャラ", "emoji 🙂", "a-b_c.d", "  padded  ", "ünïcode"]
    {
        let got = project::rename_canvas(&two, ok).unwrap();
        assert_eq!(got.file_name().unwrap().to_str().unwrap(), format!("{}.pxmt", ok.trim()));
        assert!(got.is_file());
        std::fs::rename(&got, &two).unwrap();
    }
    assert!(project::rename_canvas(dir.join("missing.pxmt"), "x").is_err());
    let _ = b;
    assert_eq!(files(&dir).len(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trash_and_restore() {
    let dir = temp_dir("trash");
    project::create(&dir, "P").unwrap();
    let a = project::new_canvas(&dir, "a", 4, 4).unwrap();
    let b = project::new_canvas(&dir, "b", 4, 4).unwrap();
    project::set_order(&dir, &["b.pxmt".into(), "a.pxmt".into()]).unwrap();
    let bytes = std::fs::read(&a).unwrap();

    let t1 = project::trash_canvas(&a).unwrap();
    assert_eq!(t1, dir.join(".pixmith/trash/a.pxmt"));
    assert!(!a.exists() && t1.is_file());
    assert_eq!(names(&project::scan(&dir).unwrap()), ["b.pxmt"]);
    // A second canvas of the same name gets a distinct trash name.
    let a2 = project::new_canvas(&dir, "a", 2, 2).unwrap();
    let t2 = project::trash_canvas(&a2).unwrap();
    assert_eq!(t2, dir.join(".pixmith/trash/a 2.pxmt"));
    assert_eq!(project::trashed_canvases(&dir).unwrap(), [t2.clone(), t1.clone()]);
    // Restoring puts it back, under another name if the name is taken meanwhile.
    let r1 = project::restore_canvas(&t1).unwrap();
    assert_eq!(r1, a);
    assert_eq!(std::fs::read(&r1).unwrap(), bytes);
    let a3 = project::new_canvas(&dir, "a", 2, 2).unwrap();
    assert_eq!(a3, dir.join("a 2.pxmt"));
    let r2 = project::restore_canvas(&t2).unwrap();
    assert_eq!(r2, dir.join("a 3.pxmt"));
    assert!(project::trashed_canvases(&dir).unwrap().is_empty());
    assert_eq!(names(&project::scan(&dir).unwrap()), ["b.pxmt", "a.pxmt", "a 2.pxmt", "a 3.pxmt"]);
    // The trash folder never shows up as a canvas; other formats can be trashed; bad targets are refused.
    std::fs::write(dir.join("old.aseprite"), ase::write(&sample(3, 3))).unwrap();
    let t = project::trash_canvas(dir.join("old.aseprite")).unwrap();
    assert!(t.ends_with("old.aseprite") && project::restore_canvas(&t).unwrap().ends_with("old.aseprite"));
    assert!(project::trash_canvas(dir.join("missing.pxmt")).is_err());
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    assert!(project::trash_canvas(dir.join("a.txt")).is_err());
    assert!(project::restore_canvas(&b).is_err(), "not in a trash folder");
    assert!(project::restore_canvas(dir.join(".pixmith/trash/none.pxmt")).is_err());
    assert!(project::restore_canvas(dir.join(".pixmith/project.json")).is_err());
    // Trashing without a project file works and creates the trash folder.
    let loose = temp_dir("trash-loose");
    let x = project::new_canvas(&loose, "x", 1, 1).unwrap();
    assert!(project::trash_canvas(&x).unwrap().is_file());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&loose);
}

#[test]
fn import_converts_to_pxmt() {
    let dir = temp_dir("import");
    let src = temp_dir("import-src");
    let project_dir = dir.join("proj");
    project::create(&project_dir, "P").unwrap();
    let sprite = sample(12, 9);
    let a = src.join("walk cycle.aseprite");
    std::fs::write(&a, ase::write(&sprite)).unwrap();
    std::fs::write(src.join("tiny.ase"), ase::write(&sample(2, 2))).unwrap();
    std::fs::write(src.join("pic.png"), export::frame_png(&sprite, 0, 1).unwrap()).unwrap();
    write_pxmt(&src.join("native.pxmt"), &sprite);
    std::fs::write(src.join("notes.txt"), "hi").unwrap();
    std::fs::write(src.join("bad.aseprite"), b"nope").unwrap();
    std::fs::write(src.join("bad.png"), b"nope").unwrap();

    let p1 = project::import(&project_dir, &a).unwrap();
    assert_eq!(p1, project_dir.join("walk cycle.pxmt"));
    let imported = pxmt::open_file(&p1).unwrap();
    assert_eq!(imported.render(0), sprite.render(0));
    assert_eq!(imported.layers.len(), sprite.layers.len());
    assert!(pxmt::read_thumbnail_file(&p1).unwrap().is_some());
    // Importing again does not overwrite.
    assert!(project::import(&project_dir, &a).unwrap().ends_with("walk cycle 2.pxmt"));
    assert!(project::import(&project_dir, src.join("tiny.ase")).unwrap().ends_with("tiny.pxmt"));
    let png = project::import(&project_dir, src.join("pic.png")).unwrap();
    assert!(png.ends_with("pic.pxmt"));
    let from_png = pxmt::open_file(&png).unwrap();
    assert_eq!((from_png.width, from_png.height), (12, 9));
    assert_eq!(from_png.render(0), sprite.render(0));
    assert!(project::import(&project_dir, src.join("native.pxmt")).unwrap().ends_with("native.pxmt"));
    for bad in ["notes.txt", "bad.aseprite", "bad.png", "missing.png"] {
        let err = project::import(&project_dir, src.join(bad));
        assert!(err.is_err(), "{bad}");
    }
    assert_eq!(project::scan(&project_dir).unwrap().canvases.len(), 5);
    // The folder is created when needed.
    assert!(project::import(dir.join("new/sub"), src.join("tiny.ase")).unwrap().is_file());
    // The original file is untouched.
    assert_eq!(std::fs::read(&a).unwrap(), ase::write(&sprite));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&src);
}

// MARK: C ABI

unsafe fn take(p: *mut std::ffi::c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let text = CStr::from_ptr(p).to_string_lossy().into_owned();
    pixmith_core::ffi::pixmith_string_free(p);
    Some(text)
}

fn c(p: &Path) -> CString {
    CString::new(p.to_str().unwrap()).unwrap()
}

#[test]
fn c_abi_round_trip() {
    use pixmith_core::project::*;
    let dir = temp_dir("abi");
    let folder = dir.join("Proj é");
    let f = c(&folder);
    unsafe {
        assert!(!pixmith_project_is_project(f.as_ptr()));
        assert!(pixmith_project_create(f.as_ptr(), c"My Project".as_ptr()));
        assert!(pixmith_project_is_project(f.as_ptr()));
        assert!(
            pixmith_project_create(f.as_ptr(), std::ptr::null()),
            "safe on an existing project, null name tolerated"
        );

        let canvas = take(pixmith_project_new_canvas(f.as_ptr(), c"Hero".as_ptr(), 16, 8)).unwrap();
        assert_eq!(PathBuf::from(&canvas), folder.join("Hero.pxmt"));
        let dup = take(pixmith_project_duplicate_canvas(c(Path::new(&canvas)).as_ptr())).unwrap();
        assert!(dup.ends_with("Hero copy.pxmt"));
        let renamed = take(pixmith_project_rename_canvas(c(Path::new(&dup)).as_ptr(), c"Villain".as_ptr())).unwrap();
        assert!(renamed.ends_with("Villain.pxmt"));
        assert!(take(pixmith_project_rename_canvas(c(Path::new(&renamed)).as_ptr(), c"Hero".as_ptr())).is_none());
        let err = CStr::from_ptr(pixmith_core::ffi::pixmith_last_error()).to_string_lossy().into_owned();
        assert!(err.contains("already exists"), "{err}");
        assert!(take(pixmith_project_rename_canvas(c(Path::new(&renamed)).as_ptr(), c"a/b".as_ptr())).is_none());
        assert!(pixmith_project_set_order(f.as_ptr(), c"[\"Villain.pxmt\", \"Hero.pxmt\"]".as_ptr()));
        assert!(!pixmith_project_set_order(f.as_ptr(), c"{\"a\":1}".as_ptr()));
        assert!(!pixmith_project_set_order(f.as_ptr(), c"[1,2]".as_ptr()));
        assert!(!pixmith_project_set_order(f.as_ptr(), c"[".as_ptr()));
        assert!(pixmith_project_set_name(f.as_ptr(), c"Renamed".as_ptr()));

        let text = take(pixmith_project_scan(f.as_ptr())).unwrap();
        let v = json::parse(&text).unwrap();
        assert_eq!(v.get("name").and_then(Value::as_str), Some("Renamed"));
        let list = v.get("canvases").and_then(Value::as_array).unwrap();
        let listed: Vec<_> = list.iter().map(|c| c.get("name").and_then(Value::as_str).unwrap()).collect();
        assert_eq!(listed, ["Villain", "Hero"]);
        assert_eq!(list[1].get("width").and_then(Value::as_f64), Some(16.0));

        let trashed = take(pixmith_project_trash_canvas(c(Path::new(&renamed)).as_ptr())).unwrap();
        assert!(trashed.contains(".pixmith"));
        let restored = take(pixmith_project_restore_canvas(c(Path::new(&trashed)).as_ptr())).unwrap();
        assert!(restored.ends_with("Villain.pxmt"));

        let src = dir.join("sprite.aseprite");
        std::fs::write(&src, ase::write(&sample(4, 4))).unwrap();
        let imported = take(pixmith_project_import(f.as_ptr(), c(&src).as_ptr())).unwrap();
        assert!(imported.ends_with("sprite.pxmt"));

        // Null and garbage arguments never crash.
        let n = std::ptr::null();
        assert!(!pixmith_project_create(n, n));
        assert!(!pixmith_project_is_project(n));
        assert!(take(pixmith_project_scan(n)).is_none());
        assert!(take(pixmith_project_scan(c"".as_ptr())).is_none());
        assert!(take(pixmith_project_scan(c(&dir.join("missing")).as_ptr())).is_none());
        assert!(!pixmith_project_set_name(n, n));
        assert!(!pixmith_project_set_name(f.as_ptr(), n));
        assert!(!pixmith_project_set_order(n, n));
        assert!(!pixmith_project_set_order(f.as_ptr(), n));
        assert!(take(pixmith_project_new_canvas(n, n, 1, 1)).is_none());
        assert!(take(pixmith_project_new_canvas(f.as_ptr(), n, 0, 0)).is_none());
        assert!(take(pixmith_project_duplicate_canvas(n)).is_none());
        assert!(take(pixmith_project_rename_canvas(n, n)).is_none());
        assert!(take(pixmith_project_rename_canvas(f.as_ptr(), n)).is_none());
        assert!(take(pixmith_project_trash_canvas(n)).is_none());
        assert!(take(pixmith_project_restore_canvas(n)).is_none());
        assert!(take(pixmith_project_import(n, n)).is_none());
        assert!(take(pixmith_project_import(f.as_ptr(), n)).is_none());
    }
    let _ = std::fs::remove_dir_all(&dir);
}
