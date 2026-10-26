//! Aseprite compatibility: `Color` and `app.pixelColor`, `io` / `os` / `app.fs` behind permissions, documents created
//! by scripts, `app.command`, and community-style scripts run unchanged. Tests use temp directories only.

use super::access::{self, Access, Permission, PermissionRequest};
use super::*;
use crate::document::{Editor, Sprite};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

static NEVER_FLAG: AtomicBool = AtomicBool::new(false);

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("pixmith-compat-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    /// A Lua string literal for `rel` inside the directory.
    fn lua(&self, rel: &str) -> String {
        format!("{:?}", self.path(rel).to_string_lossy())
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cfg(dir: &Dir) -> RunConfig {
    RunConfig { script_name: "compat".into(), doc_path: None, allowed_paths: vec![dir.0.clone()] }
}

fn editor(w: u32, h: u32) -> Editor {
    Editor::new(Sprite::new(w, h))
}

fn go(e: &mut Editor, src: &str, cfg: &RunConfig) -> Outcome {
    run_in(e, src, (0, 0), Limits::default(), &NEVER_FLAG, cfg)
}

fn good(e: &mut Editor, src: &str, cfg: &RunConfig) -> Outcome {
    let o = go(e, src, cfg);
    assert_eq!(o.error, None, "script failed: {src}");
    o
}

fn plain(e: &mut Editor, src: &str) -> Outcome {
    good(e, src, &RunConfig::default())
}

fn failing(e: &mut Editor, src: &str, needle: &str) {
    let o = go(e, src, &RunConfig::default());
    let err = o.error.unwrap_or_else(|| panic!("{src} should fail"));
    assert!(err.contains(needle), "error {err:?} does not contain {needle:?}");
}

fn red_png(w: usize, h: usize) -> Vec<u8> {
    let rgba: Vec<u8> = [255u8, 0, 0, 255].iter().copied().cycle().take(w * h * 4).collect();
    crate::formats::export::encode_png(&rgba, w, h).unwrap()
}

// MARK: Color

#[test]
fn color_constructors_and_properties() {
    let mut e = editor(4, 3);
    plain(
        &mut e,
        r##"
        local c = Color(10, 20, 30)
        assert(c.red == 10 and c.green == 20 and c.blue == 30 and c.alpha == 255)
        assert(Color(1, 2, 3, 4).alpha == 4)
        local d = Color{ r = 1, g = 2, b = 3, a = 4 }
        assert(d.red == 1 and d.green == 2 and d.blue == 3 and d.alpha == 4)
        assert(Color{ red = 5, green = 6, blue = 7 }.alpha == 255)
        local p = Color(0x80402010)                       -- an Aseprite pixel value, 0xAABBGGRR
        assert(p.red == 0x10 and p.green == 0x20 and p.blue == 0x40 and p.alpha == 0x80)
        assert(p.rgbaPixel == 0x80402010)

        local hsv = Color{ h = 0, s = 1, v = 1 }
        assert(hsv.red == 255 and hsv.green == 0 and hsv.blue == 0 and hsv.alpha == 255)
        assert(hsv.hue == 0 and hsv.saturation == 1 and hsv.value == 1)
        assert(Color{ hue = 120, saturation = 1, value = 1 }.green == 255)
        local hsl = Color{ h = 240, s = 1, l = 0.5, a = 128 }
        assert(hsl.blue == 255 and hsl.red == 0 and hsl.alpha == 128)
        assert(hsl.hslHue == 240 and hsl.hslLightness == 0.5 and hsl.lightness == 0.5)
        local gray = Color{ gray = 100, alpha = 200 }
        assert(gray.gray == 100 and gray.red == 100 and gray.blue == 100 and gray.alpha == 200)
        assert(gray.grayPixel == 100 + (200 << 8))
        assert(Color(255, 0, 0).hue == 0 and Color(0, 255, 0).hue == 120 and Color(0, 0, 255).hue == 240)
        assert(Color(255, 0, 0).value == 1 and Color(255, 0, 0).saturation == 1 and Color(255, 0, 0).lightness == 0.5)
        assert(Color(0, 0, 0).value == 0 and Color(255, 255, 255).lightness == 1)
        assert(Color(255, 0, 0).hsvHue == 0 and Color(0, 255, 0).hslHue == 120)

        local idx = Color{ index = 2 }
        assert(idx.index == 2 and idx.red == sprite:getPaletteColor(2) >> 24)

        assert(Color(1, 2, 3) == Color{ r = 1, g = 2, b = 3 } and Color(1, 2, 3) ~= Color(1, 2, 4))
        assert(Color{ r = 255 } == Color(255, 0, 0, 255))
        assert(tostring(Color(1, 2, 3, 4)) == "Color{ r=1, g=2, b=3, a=4 }")
        assert(tostring(Color{ h = 120, s = 0.5, v = 1 }) == "Color{ h=120, s=0.5, v=1, a=255 }")
        assert(tostring(Color{ gray = 7 }) == "Color{ gray=7, a=255 }")
        assert(tostring(Color{ index = 3 }) == "Color{ index=3 }")

        local m = Color(255, 0, 0)
        m.hue = 120
        assert(m.green == 255 and m.red == 0)
        m.red = 10 m.alpha = 20
        assert(m.red == 10 and m.alpha == 20)
        m.gray = 50
        assert(m.red == 50 and m.green == 50 and m.alpha == 20)
        assert(Color("#ff8000").green == 128 and Color("#ff800080").alpha == 128 and Color("#f00").red == 255)

        -- older scripts treated colors as 0xRRGGBBAA integers: bit operations still work on a Color
        local big = Color(0x11, 0x22, 0x33, 0x44)
        assert((big >> 24) == 0x11 and (big & 0xFF) == 0x44 and ((big >> 8) & 0xFF) == 0x33)

        assert(not pcall(Color, "nope") and not pcall(Color, 1, 2) and not pcall(Color, {}, {}))
        assert(not pcall(function() return Color{ hue = 1 } end))
        "##,
    );
}

#[test]
fn colors_work_wherever_a_color_is_taken() {
    let mut e = editor(4, 3);
    plain(
        &mut e,
        r#"
        sprite:setPixel(0, 0, Color(255, 0, 0))
        assert(sprite:getPixel(0, 0) == 0xFF0000FF)
        sprite:setPixel(1, 0, 0x00FF00FF)                       -- integers keep their 0xRRGGBBAA meaning
        assert(sprite:getPixel(1, 0) == 0x00FF00FF)
        assert(color.toPixel(0x00FF00FF) == app.pixelColor.rgba(0, 255, 0))
        assert(color.fromPixel(app.pixelColor.rgba(0, 255, 0)) == 0x00FF00FF)
        assert(color.toPixel(Color(0, 255, 0)) == app.pixelColor.rgba(0, 255, 0))
        local r, g, b, a = color.split(Color(1, 2, 3, 4))
        assert(r == 1 and g == 2 and b == 3 and a == 4)
        assert(color.rgba(1, 2, 3) == 0x010203FF)

        sprite:setPaletteColor(0, Color(1, 2, 3))
        assert(sprite:getPaletteColor(0) == 0x010203FF)
        local pal = sprite.palettes[1]
        assert(#pal == sprite.paletteSize and pal:getColor(0) == Color(1, 2, 3))
        pal:setColor(1, Color{ r = 9, g = 8, b = 7 })
        assert(sprite:getPaletteColor(1) == 0x090807FF)
        pal:resize(3)
        assert(#pal == 3 and sprite.paletteSize == 3)
        assert(not pcall(pal.getColor, pal, 3))
        local copy = Palette(4)
        assert(#copy == 4 and copy:getColor(0) == Color(0, 0, 0))
        copy:setColor(0, 0x112233FF)
        assert(copy:getColor(0) == Color(0x11, 0x22, 0x33))
        sprite:setPalette(copy)
        assert(sprite.paletteSize == 4 and sprite:getPaletteColor(0) == 0x112233FF)
        sprite:addPaletteColor(Color(5, 6, 7))
        assert(sprite:getPaletteColor(4) == 0x050607FF)

        assert(app.fgColor == Color(0, 0, 0) and app.bgColor == Color(255, 255, 255))
        local fg = app.fgColor
        fg.red = 7                                               -- a copy: the app's color is unchanged
        assert(app.fgColor.red == 0)
        app.fgColor = Color(1, 2, 3)
        app.bgColor = 0x040506FF

        sprite:newFrame()
        local t = sprite:newTag("t", 1, 1)
        t.color = Color(10, 20, 30)
        assert(t.color == Color(10, 20, 30))
        t.color = 0x0A141EFF
        assert(t.color == Color(10, 20, 30))

        sprite:setPixels(1, 1, { Color(1, 2, 3), 0x040506FF }, 0, 2, 2, 1)
        assert(sprite:getPixel(0, 2) == 0x010203FF and sprite:getPixel(1, 2) == 0x040506FF)
        sprite:newLayer("x").userData.color = Color(9, 9, 9)
        assert(sprite.layers[2].userData.color == Color(9, 9, 9) and sprite.layers[2].color == Color(9, 9, 9))
        "#,
    );
}

#[test]
fn pixel_color_packs_like_aseprite() {
    let mut e = editor(2, 2);
    plain(
        &mut e,
        r#"
        local P = app.pixelColor
        assert(P.rgba(1, 2, 3, 4) == 0x04030201 and P.rgba(1, 2, 3) == 0xFF030201)
        local px = P.rgba(10, 20, 30, 40)
        assert(P.rgbaR(px) == 10 and P.rgbaG(px) == 20 and P.rgbaB(px) == 30 and P.rgbaA(px) == 40)
        assert(P.graya(7, 9) == 7 + (9 << 8) and P.grayaV(P.graya(7, 9)) == 7 and P.grayaA(P.graya(7, 9)) == 9)
        assert(P.graya(5) == 5 + (255 << 8))

        -- RGB images speak pixel values
        local img = Image(2, 1)
        img:putPixel(0, 0, P.rgba(1, 2, 3, 255))
        assert(img:getPixel(0, 0) == 0xFF030201)
        local ok = img.bytes
        assert(ok:byte(1) == 1 and ok:byte(2) == 2 and ok:byte(3) == 3 and ok:byte(4) == 255)
        for it in img:pixels() do it(P.rgba(it.x, 0, 0, 255)) end
        assert(img:getPixel(1, 0) == P.rgba(1, 0, 0, 255))

        -- grayscale images: value | alpha << 8
        local g = Image(2, 1, ColorMode.GRAY)
        g:putPixel(0, 0, P.graya(100, 200))
        assert(g:getPixel(0, 0) == P.graya(100, 200))
        assert(g.bytes:byte(1) == 100 and g.bytes:byte(4) == 200)

        -- sprites keep their own 0xRRGGBBAA integers
        sprite:setPixel(0, 0, 0x010203FF)
        assert(sprite:getPixel(0, 0) == 0x010203FF)
        local cel = sprite:cel(1, 1)
        assert(cel.image:getPixel(0, 0) == P.rgba(1, 2, 3, 255))
        "#,
    );
}

#[test]
fn indexed_images_use_palette_indices() {
    let mut s = Sprite::new(2, 1);
    s.palette = vec![[0, 0, 0, 255], [255, 0, 0, 255], [0, 255, 0, 255]];
    s.color_mode = crate::document::color_mode::ColorMode::Indexed;
    let mut e = Editor::new(s);
    plain(
        &mut e,
        r#"
        assert(sprite.colorMode == ColorMode.INDEXED)
        sprite:setPixel(0, 0, 0xFF0000FF)
        local img = sprite.cels[1].image
        assert(img:getPixel(0, 0) == 1)                       -- the palette index
        img:putPixel(0, 0, 2)
        assert(sprite:getPixel(0, 0) == 0x00FF00FF)
        assert(sprite.palettes[1]:getColor(img:getPixel(0, 0)) == Color(0, 255, 0))
        "#,
    );
}

// MARK: Permissions

fn recording(
    answer: impl Fn(&PermissionRequest) -> Permission + Send + Sync + 'static,
) -> Arc<Mutex<Vec<PermissionRequest>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let l = log.clone();
    access::set_thread_permission_handler(Some(Arc::new(move |r: &PermissionRequest| {
        l.lock().unwrap().push(r.clone());
        answer(r)
    })));
    log
}

#[test]
fn files_are_denied_without_a_handler_and_allowed_under_allowed_paths() {
    let dir = Dir::new("noaccess");
    access::set_thread_permission_handler(None);
    let mut e = editor(2, 2);
    let src = format!(
        r#"
        local ok, err = pcall(io.open, {p}, "w")
        assert(not ok and err:find("permission denied"), tostring(err))
        assert(not pcall(app.fs.makeDirectory, {d}) and not pcall(app.fs.listFiles, {d}))
        assert(not pcall(os.remove, {p}) and not pcall(os.rename, {p}, {q}))
        assert(not pcall(os.execute, "true") and not pcall(io.popen, "true"))
        assert(not pcall(io.lines, {p}) and not pcall(app.open, {p}))
        assert(not pcall(Image, {{ fromFile = {p} }}) and not pcall(sprite.saveAs, sprite, {p}))
        assert(not pcall(dofile, {p}) and not pcall(loadfile, {p}))
        "#,
        p = dir.lua("x.txt"),
        q = dir.lua("y.txt"),
        d = dir.lua("sub"),
    );
    good(&mut e, &src, &RunConfig::default());
    assert!(!dir.path("x.txt").exists() && !dir.path("sub").exists());

    // The same calls work inside allowed_paths, with no handler.
    let src = format!(
        r#"
        local f = assert(io.open({p}, "wb"))                    -- binary: no CRLF translation on Windows
        f:write("hello\nworld")
        f:close()
        local r = assert(io.open({p}, "r"))
        assert(r:read("a") == "hello\nworld")
        r:close()
        local n = 0
        for line in io.lines({p}) do n = n + 1 end
        assert(n == 2)
        assert(app.fs.isFile({p}) and not app.fs.isDirectory({p}) and app.fs.fileSize({p}) == 11)
        assert(os.rename({p}, {q}) and not app.fs.isFile({p}) and app.fs.isFile({q}))
        assert(os.remove({q}) and not app.fs.isFile({q}))
        assert(app.fs.fileSize({q}) == 0)
        -- outside the allowed directory: still denied
        assert(not pcall(io.open, "/etc/hosts") and not pcall(os.remove, "/etc/hosts"))
        "#,
        p = dir.lua("a.txt"),
        q = dir.lua("b.txt"),
    );
    good(&mut e, &src, &cfg(&dir));
}

#[test]
fn the_handler_is_asked_once_per_path_tree_and_always_remembers_the_script() {
    let dir = Dir::new("asking");
    let log = recording(|r| match r.access {
        Access::Execute => Permission::Deny,
        _ if r.path.contains("forbidden") => Permission::Deny,
        _ => Permission::AllowOnce,
    });
    let mut e = editor(2, 2);
    let src = format!(
        r#"
        assert(app.fs.makeAllDirectories({tree}))               -- asks: write {tree}
        local f = assert(io.open({tree} .. "/a.txt", "w"))      -- covered by the grant for the directory
        f:write("x") f:close()
        assert(app.fs.isFile({tree} .. "/a.txt"))               -- read, covered
        assert(#app.fs.listFiles({tree}) == 1)
        local g = assert(io.open({other}, "w"))                  -- another tree: asks again
        g:close()
        local ok, err = pcall(io.open, {dir} .. "/forbidden.txt", "w")
        assert(not ok and err:find("permission denied"), tostring(err))
        assert(not pcall(os.execute, "echo nope"))
        "#,
        tree = dir.lua("t/u"),
        other = dir.lua("other.txt"),
        dir = dir.lua(""),
    );
    let c = RunConfig { script_name: "asking-script".into(), ..RunConfig::default() };
    good(&mut e, &src, &c);
    let reqs = log.lock().unwrap().clone();
    let shown: Vec<(Access, bool)> = reqs.iter().map(|r| (r.access, r.path.ends_with("other.txt"))).collect();
    assert_eq!(
        shown,
        vec![
            (Access::Write, false),
            (Access::Write, true),
            (Access::Write, false), // forbidden.txt
            (Access::Execute, false),
        ],
        "{reqs:?}"
    );
    assert!(reqs.iter().all(|r| r.script_name == "asking-script"));
    assert_eq!(reqs[0].path, dir.path("t").join("u").to_string_lossy());
    assert_eq!(reqs[3].path, "echo nope");
    assert!(dir.path("t/u/a.txt").is_file() && dir.path("other.txt").is_file());

    // "Always allow" is remembered per script and per kind of access.
    let log = recording(|_| Permission::AllowAlwaysForScript);
    let c = RunConfig { script_name: "always-script".into(), ..RunConfig::default() };
    let src = format!(
        r#"
        io.open({a}, "w"):close()
        io.open({b}, "w"):close()
        assert(app.fs.isFile({a}))
        "#,
        a = dir.lua("always1.txt"),
        b = dir.lua("always2.txt"),
    );
    good(&mut e, &src, &c);
    assert_eq!(log.lock().unwrap().len(), 1, "the second write needed no question");
    // A new run of the same script is not asked again; another script is.
    good(&mut e, &format!("io.open({}, 'w'):close()", dir.lua("always3.txt")), &c);
    assert_eq!(log.lock().unwrap().len(), 1);
    let other = RunConfig { script_name: "someone-else".into(), ..RunConfig::default() };
    good(&mut e, &format!("io.open({}, 'w'):close()", dir.lua("always4.txt")), &other);
    assert_eq!(log.lock().unwrap().len(), 2);
    access::reset_script_grants("always-script");
    good(&mut e, &format!("io.open({}, 'w'):close()", dir.lua("always5.txt")), &c);
    assert_eq!(log.lock().unwrap().len(), 3, "grants can be reset");
    access::set_thread_permission_handler(None);
}

#[test]
fn time_spent_in_the_permission_handler_does_not_count_toward_the_time_limit() {
    let dir = Dir::new("waiting");
    let _log = recording(|_| {
        std::thread::sleep(Duration::from_millis(500));
        Permission::AllowOnce
    });
    let mut e = editor(2, 2);
    let limits = Limits::default().with_time(Duration::from_millis(250));
    let src = format!("io.open({}, 'w'):close() local n = 0 for i = 1, 20000 do n = n + 1 end", dir.lua("w.txt"));
    let o = run_in(&mut e, &src, (0, 0), limits, &NEVER_FLAG, &RunConfig::default());
    assert_eq!(o.error, None, "the 500 ms dialog is not script time");
    // The limit still applies to the script itself.
    let o = run_in(
        &mut e,
        &format!("io.open({}, 'w'):close() while true do end", dir.lua("w2.txt")),
        (0, 0),
        limits,
        &NEVER_FLAG,
        &RunConfig::default(),
    );
    assert!(o.error.unwrap().contains("time limit"));
    access::set_thread_permission_handler(None);
}

#[test]
fn os_tmpname_files_need_no_question_and_dangerous_os_calls_are_gone() {
    access::set_thread_permission_handler(None);
    let mut e = editor(2, 2);
    plain(
        &mut e,
        r#"
        local t = os.tmpname()
        local f = assert(io.open(t, "w"))
        f:write("scratch") f:close()
        local r = assert(io.open(t, "r"))
        assert(r:read("a") == "scratch")
        r:close()                                                 -- Windows cannot remove an open file
        assert(os.remove(t))
        assert(os.exit and not pcall(os.exit))
        assert(os.setlocale == nil and io.stdin == nil)
        assert(os.getenv("PIXMITH_NOT_A_REAL_VARIABLE") == nil and os.getenv("SSH_AUTH_SOCK") == nil)
        assert(type(os.time()) == "number" and type(os.clock()) == "number" and os.date("%Y") ~= nil)
        assert(io.read() == nil)                                  -- no standard input
        assert(not pcall(io.lines))
        "#,
    );
}

#[test]
fn app_fs_path_functions() {
    let dir = Dir::new("fs");
    let separator = format!("{:?}", std::path::MAIN_SEPARATOR.to_string());
    let mut e = editor(2, 2);
    let src = format!(
        r#"
        local fs = app.fs
        assert(fs.pathSeparator == {sep})
        assert(fs.filePath("/a/b/c.png") == "/a/b" and fs.fileName("/a/b/c.png") == "c.png")
        assert(fs.fileExtension("/a/b/c.png") == "png" and fs.fileTitle("/a/b/c.png") == "c")
        assert(fs.filePathAndTitle("/a/b/c.png") == "/a/b/c" and fs.filePath("c.png") == "")
        assert(fs.fileExtension("/a/b/noext") == "" and fs.fileTitle("x.tar.gz") == "x.tar")
        assert(fs.normalizePath("/a/./b/../c") == "/a/c" and fs.normalizePath("a/../../b") == "../b")
        assert(fs.joinPath("/a", "b", "c.png") == "/a/b/c.png" and fs.joinPath("/a/", "/b") == "/a/b")
        assert(fs.currentPath() ~= "" and fs.tempPath() ~= "" and fs.appPath() ~= nil)
        assert(fs.userDocsPath():find("Documents") and fs.userConfigPath():lower():find("pixmith"))

        local d = {sub}
        assert(fs.makeDirectory(d) and fs.isDirectory(d) and not fs.isFile(d))
        assert(not fs.makeDirectory(d), "already there")
        assert(fs.makeAllDirectories(d .. "/x/y") and fs.isDirectory(d .. "/x/y"))
        io.open(d .. "/f.txt", "w"):close()
        local list = fs.listFiles(d)
        assert(#list == 2 and list[1] == "f.txt" and list[2] == "x")
        assert(#fs.listFiles(d .. "/missing") == 0)
        assert(not fs.removeDirectory(d), "not empty")
        os.remove(d .. "/f.txt")
        assert(fs.removeDirectory(d .. "/x/y") and fs.removeDirectory(d .. "/x") and fs.removeDirectory(d))
        assert(not fs.isDirectory(d))
        "#,
        sub = dir.lua("sub"),
        sep = separator,
    );
    good(&mut e, &src, &cfg(&dir));
}

// MARK: Documents

#[test]
fn constructors_create_documents_and_the_run_reports_them() {
    let dir = Dir::new("docs");
    let mut e = editor(4, 4);
    let src = format!(
        r#"
        local a = Sprite(5, 3)
        assert(a.width == 5 and a.height == 3 and a.colorMode == ColorMode.RGB and a.filename == "")
        assert(#a.layers == 1 and #a.frames == 1 and a.layers[1].name == "Layer 1")
        assert(a.cels[1] ~= nil, "a new sprite has its first (empty) cel")
        a.layers[1].name = "made"
        local b = Sprite(2, 2)
        assert(app.activeSprite == b and app.sprite == b and #app.sprites == 3)
        assert(app.sprites[1] == sprite and app.sprites[2] == a and app.sprites[3] == b)
        app.activeSprite = a
        assert(app.activeSprite == a and app.activeSprite ~= sprite)
        assert(app.activeLayer.name == "made" and app.activeFrame.frameNumber == 1)
        b:saveAs({bpath})
        assert(b.filename == {bpath} and not b.isModified)
        b:setPixel(0, 0, 0xFF0000FF)
        assert(b.isModified)
        local g = Sprite(4, 4, ColorMode.GRAY)
        assert(g.colorMode == ColorMode.GRAY)
        local i = Sprite(ImageSpec{{ width = 3, height = 5, colorMode = ColorMode.INDEXED }})
        assert(i.colorMode == ColorMode.INDEXED and i.width == 3 and i.height == 5)
        local c = Sprite(a)                                        -- a copy
        assert(c.width == 5 and c.layers[1].name == "made" and c ~= a)
        app.activeSprite = a
        "#,
        bpath = dir.lua("b.aseprite"),
    );
    let o = good(&mut e, &src, &cfg(&dir));
    let docs = &o.new_documents;
    assert_eq!(docs.len(), 5);
    assert_eq!((docs[0].sprite.width, docs[0].sprite.height), (5, 3));
    assert_eq!(docs[0].path, None);
    assert!(docs[0].modified);
    assert_eq!(docs[1].path.as_deref(), Some(dir.path("b.aseprite").as_path()));
    assert!(docs[1].modified, "changed after the save");
    assert_eq!(docs[2].sprite.color_mode, crate::document::color_mode::ColorMode::Grayscale);
    assert_eq!(docs[3].sprite.color_mode, crate::document::color_mode::ColorMode::Indexed);
    assert_eq!(o.active, ActiveDoc::New(0));
    assert!(!o.close_original && o.original_path.is_none());
    assert!(dir.path("b.aseprite").is_file());
    // The original is untouched and no undo step was recorded.
    assert_eq!(e.history_len().0, 0);

    // A failing run reports nothing.
    let o = go(&mut e, "Sprite(3, 3) error('boom')", &cfg(&dir));
    assert!(o.error.unwrap().contains("boom"));
    assert!(o.new_documents.is_empty() && o.active == ActiveDoc::Unchanged);

    // A run that creates nothing leaves the active document alone.
    let o = plain(&mut e, "local s = app.activeSprite assert(s == sprite)");
    assert!(o.new_documents.is_empty() && o.active == ActiveDoc::Unchanged);
}

#[test]
fn saving_and_opening_round_trip() {
    let dir = Dir::new("saveopen");
    std::fs::write(dir.path("pic.png"), red_png(3, 2)).unwrap();
    let mut e = editor(4, 4);
    let src = format!(
        r#"
        local s = Sprite(4, 4)
        s.layers[1].name = "ink"
        local img = Image(s.spec)
        img:putPixel(1, 1, 0xFF0000FF)
        s:newCel(s.layers[1], 1, img)
        s:newFrame()
        s.frames[2].duration = 250
        s:saveAs({a})
        assert(s.filename == {a} and not s.isModified)
        s:saveCopyAs({png})
        s:saveCopyAs({gif})
        s:saveCopyAs({pxmt})
        assert(s.filename == {a}, "saveCopyAs keeps the sprite's file")
        assert(app.fs.isFile({gif}) and app.fs.isFile({pxmt}))
        assert(app.fs.isFile(app.fs.joinPath({dir}, "out-001.png")) and app.fs.isFile(app.fs.joinPath({dir}, "out-002.png")))
        assert(app.open({a}) == s, "opening a file that is open gives the open sprite")
        s:close()
        assert(not pcall(s.saveAs, s, {a}), "a closed sprite cannot be saved")
        local again = app.open({a})
        assert(again ~= nil and again.width == 4 and again.layers[1].name == "ink" and #again.frames == 2)
        assert(again.frames[2].duration == 250 and again.filename == {a})
        assert(app.open({missing}) == nil)
        local pic = app.open({pic})
        assert(pic.width == 3 and pic.height == 2 and pic.filename == {pic})
        local viaSprite = Sprite{{ fromFile = {pic} }}
        assert(viaSprite.width == 3)
        assert(not pcall(Sprite, {{ fromFile = {missing} }}))
        "#,
        a = dir.lua("a.aseprite"),
        png = dir.lua("out.png"),
        gif = dir.lua("out.gif"),
        pxmt = dir.lua("out.pxmt"),
        pic = dir.lua("pic.png"),
        missing = dir.lua("missing.aseprite"),
        dir = dir.lua(""),
    );
    let o = good(&mut e, &src, &cfg(&dir));
    // the reopened sprite, the PNG and the Sprite{ fromFile } copy of it
    assert_eq!(o.new_documents.len(), 3);
    assert_eq!(o.new_documents[0].path.as_deref(), Some(dir.path("a.aseprite").as_path()));
    assert!(!o.new_documents[0].modified, "exactly what the file holds");
    assert_eq!(o.new_documents[1].path, None, "images open untitled");
    assert!(o.new_documents[1].modified);
    assert_eq!(o.active, ActiveDoc::New(2));
    // The saved files really are what they say.
    let saved = crate::formats::pxmt::open_file(dir.path("a.aseprite")).unwrap();
    assert_eq!((saved.width, saved.frame_count(), saved.durations[1]), (4, 2, 250));
    assert_eq!(&saved.render(0)[(4 + 1) * 4..][..4], &[255, 0, 0, 255]);
    let pxmt = crate::formats::pxmt::open_file(dir.path("out.pxmt")).unwrap();
    assert_eq!(pxmt.frame_count(), 2);
    let gif = std::fs::read(dir.path("out.gif")).unwrap();
    assert!(gif.starts_with(b"GIF89a"));
}

#[test]
fn the_original_document_can_be_saved_closed_and_named() {
    let dir = Dir::new("original");
    let mut e = editor(4, 4);
    let c = cfg(&dir);
    let o = good(&mut e, &format!("sprite:setPixel(0, 0, 0xFF0000FF) sprite:saveAs({})", dir.lua("o.aseprite")), &c);
    assert_eq!(o.original_path.as_deref(), Some(dir.path("o.aseprite").as_path()));
    assert!(o.original_saved);
    assert!(dir.path("o.aseprite").is_file());
    assert_eq!(e.history_len().0, 1, "the edit is still one undo step");

    let o = good(&mut e, &format!("sprite:saveAs({}) sprite:setPixel(1, 1, 0x00FF00FF)", dir.lua("p.aseprite")), &c);
    assert!(o.original_path.is_some() && !o.original_saved, "changed after the save");

    // saveCopyAs and exports do not change the document's file.
    let o =
        good(&mut e, &format!("sprite:saveCopyAs({}) sprite:saveAs({})", dir.lua("c.aseprite"), dir.lua("e.png")), &c);
    assert!(o.original_path.is_none());

    // sprite.filename comes from the front end; app.open of that path is the original document.
    let with_path = RunConfig { doc_path: Some(dir.path("o.aseprite")), ..cfg(&dir) };
    let o = good(
        &mut e,
        &format!(
            "assert(sprite.filename == {p} and app.open({p}) == sprite and #app.sprites == 1)",
            p = dir.lua("o.aseprite")
        ),
        &with_path,
    );
    assert!(o.new_documents.is_empty());

    // Closing the document: the front end closes the tab, nothing of the script's edits is kept.
    let before = e.sprite.clone();
    let steps = e.history_len().0;
    let o = good(
        &mut e,
        &format!(
            "sprite:setPixel(2, 2, 0xFF0000FF) sprite:close() assert(not pcall(sprite.saveAs, sprite, {}))",
            dir.lua("z.aseprite")
        ),
        &c,
    );
    assert!(o.close_original);
    assert_eq!(e.sprite, before);
    assert_eq!(e.history_len().0, steps);
    assert!(!dir.path("z.aseprite").exists());
}

#[test]
fn documents_are_limited() {
    let mut e = editor(2, 2);
    failing(&mut e, "for i = 1, 40 do Sprite(1, 1) end", "at most 32");
    failing(&mut e, "Sprite(0, 5)", "invalid size");
    failing(&mut e, "Sprite(70000, 5)", "invalid size");
    failing(&mut e, "Sprite('x')", "Sprite:");
    failing(&mut e, "Sprite(20000, 20000)", "invalid size");
    // 4096 x 4096 pixels is 64 MB per sprite (allocated lazily): eight fit in the 512 MB a run may start with.
    failing(&mut e, "for i = 1, 10 do Sprite(4096, 4096) end", "MB");
    failing(&mut e, "Sprite(2, 2, 9)", "color mode");
}

#[test]
fn sprite_methods_flatten_resize_crop() {
    let mut e = editor(6, 6);
    plain(
        &mut e,
        r#"
        sprite:setPixel(1, 1, 0xFF0000FF)
        local top = sprite:newLayer("top")
        sprite:setPixel(4, 4, 0x00FF00FF, top.index, 1)
        sprite:flatten()
        assert(#sprite.layers == 1)
        assert(sprite:getPixel(1, 1) == 0xFF0000FF and sprite:getPixel(4, 4) == 0x00FF00FF)
        sprite:crop(1, 1, 4, 4)
        assert(sprite.width == 4 and sprite.height == 4 and sprite:getPixel(0, 0) == 0xFF0000FF)
        sprite:crop(Rectangle(0, 0, 2, 2))
        assert(sprite.width == 2)
        sprite:resize(4, 6)
        assert(sprite.width == 4 and sprite.height == 6 and sprite:getPixel(0, 0) == 0xFF0000FF)
        sprite:resize(Size(2, 2))
        assert(sprite.width == 2)
        assert(not pcall(sprite.resize, sprite, 0, 5) and not pcall(sprite.crop, sprite, 0, 0, 0, 0))
        "#,
    );
}

// MARK: Enumerations, Version, Image extras

#[test]
fn enumerations_and_version() {
    let mut e = editor(4, 4);
    plain(
        &mut e,
        r#"
        assert(BlendMode.NORMAL == 0 and BlendMode.MULTIPLY == 2 and BlendMode.HSL_LUMINOSITY == 18)
        assert(app.blend.Multiply == BlendMode.MULTIPLY and app.blend.Screen == BlendMode.SCREEN)
        local layer = sprite.layers[1]
        for name, v in pairs(BlendMode) do
          layer.blendMode = v
          assert(layer.blendMode == v, name)
        end
        layer.blendMode = BlendMode.MULTIPLY
        assert(not pcall(function() layer.blendMode = 99 end))
        assert(FlipType.HORIZONTAL == 0 and FlipType.VERTICAL == 1)
        assert(AniDir.FORWARD == 0 and AniDir.PING_PONG_REVERSE == 3 and AniDir.PING_PONG == app.direction.PingPong)
        assert(SpriteSheetType.HORIZONTAL == 1 and SpriteSheetType.ROWS == 3 and SpriteSheetType.PACKED == 5)
        assert(ColorMode.RGB == 0 and ColorMode.GRAYSCALE == 1 and ColorMode.INDEXED == 2 and ColorMode.TILEMAP == 4)
        local t = sprite:newTag("a", 1, 1)
        t.aniDir = AniDir.REVERSE
        assert(t.direction == 1 and t.aniDir == AniDir.REVERSE and t.frames == 1 and t.fromFrame.frameNumber == 1)

        assert(Version("1.2.3") < Version("1.10.0") and Version("1.3-rc1") < Version("1.3.0"))
        assert(Version("1.3-beta2") < Version("1.3-rc1") and Version("2.0") > Version("1.99.99"))
        assert(Version("1.2.3").major == 1 and Version("1.2.3-beta4").prereleaseLabel == "beta")
        assert(Version("1.2.3-beta4").prereleaseNumber == 4)
        assert(tostring(Version("1.2.3")) == "1.2.3" and Version(2, 1, 0) == Version("2.1.0"))
        assert(Version("1.2.3") <= Version("1.2.3") and Version("1.2.3") >= Version("1.2.3"))
        assert(not pcall(Version, "x.y"))
        assert(app.apiVersion >= 21 and app.version >= Version("0.1.0") and app.version.major ~= nil)
        "#,
    );
    assert_eq!(e.sprite.layers[0].blend_mode, 1, "stored as Pixmith's (file format) number");
}

#[test]
fn image_flip_shrink_and_save() {
    let dir = Dir::new("imgextras");
    let mut e = editor(4, 4);
    let src = format!(
        r#"
        local img = Image(3, 2)
        img:putPixel(0, 0, 0xFF0000FF)
        img:flip()
        assert(img:getPixel(2, 0) == 0xFF0000FF and img:getPixel(0, 0) == 0)
        img:flip(FlipType.VERTICAL)
        assert(img:getPixel(2, 1) == 0xFF0000FF and img:getPixel(2, 0) == 0)
        assert(img:shrinkBounds() == Rectangle(2, 1, 1, 1))
        assert(Image(2, 2):shrinkBounds().isEmpty)
        assert(not pcall(img.flip, img, 7))
        local plain = Image(2, 2)
        plain:clear(Color(1, 2, 3))
        assert(plain:isPlain(Color(1, 2, 3)))
        plain:putPixel(1, 1, Color(9, 9, 9))
        assert(plain:shrinkBounds(Color(1, 2, 3)) == Rectangle(1, 1, 1, 1))
        img:saveAs({png})
        local loaded = Image{{ fromFile = {png} }}
        assert(loaded:isEqual(img))
        img:saveAs({bmp})
        assert(app.fs.isFile({bmp}))
        "#,
        png = dir.lua("img.png"),
        bmp = dir.lua("img.bmp"),
    );
    good(&mut e, &src, &cfg(&dir));
}

// MARK: app.command

#[test]
fn commands_frames_layers_and_navigation() {
    let mut e = editor(4, 4);
    plain(
        &mut e,
        r#"
        local spr = app.activeSprite
        app.command.NewFrame()
        assert(#spr.frames == 2 and app.activeFrame.frameNumber == 2)
        app.command.GotoFirstFrame() assert(app.activeFrame.frameNumber == 1)
        app.command.GotoNextFrame() assert(app.activeFrame.frameNumber == 2)
        app.command.GotoNextFrame() assert(app.activeFrame.frameNumber == 1)
        app.command.GotoLastFrame() assert(app.activeFrame.frameNumber == 2)
        app.command.GotoPreviousFrame() assert(app.activeFrame.frameNumber == 1)
        app.command.GotoFrame{ frame = 2 } assert(app.activeFrame.frameNumber == 2)
        app.command.RemoveFrame()
        assert(#spr.frames == 1 and app.activeFrame.frameNumber == 1)
        assert(not pcall(app.command.RemoveFrame), "the last frame stays")

        app.command.NewLayer{ name = "Fx" }
        assert(#spr.layers == 2 and app.activeLayer.name == "Fx")
        sprite:setPixel(0, 0, 0xFF0000FF)                       -- draws on the active layer
        app.command.NewLayer{ name = "Grp", group = true }
        assert(spr.layers[3].isGroup and spr.layers[3].name == "Grp" and app.activeLayer.name == "Fx")
        app.command.DuplicateLayer()
        assert(app.activeLayer.name == "Fx copy" and sprite:getPixel(0, 0, app.activeLayer.stackIndex) == 0xFF0000FF)
        app.command.MergeDownLayer()
        assert(app.activeLayer.name == "Fx" and #spr.layers == 3)
        app.command.GotoPreviousLayer() assert(app.activeLayer.name == "Layer 1")
        app.command.GotoNextLayer() assert(app.activeLayer.name == "Fx")
        app.command.RemoveLayer()
        assert(#spr.layers == 2)
        app.command.FlattenLayers()
        assert(#spr.layers == 1)
        assert(not pcall(app.command.RemoveLayer), "the last drawable layer stays")
        "#,
    );
}

#[test]
fn commands_undo_redo_and_transactions() {
    let mut e = editor(4, 4);
    plain(
        &mut e,
        r#"
        sprite:setPixel(1, 1, 0x0000FFFF)
        app.command.NewFrame()
        assert(#sprite.frames == 2)
        app.command.Undo()
        assert(#sprite.frames == 1 and sprite:getPixel(1, 1) == 0x0000FFFF)
        app.command.Undo()
        assert(sprite:getPixel(1, 1) == 0)
        assert(not pcall(app.command.Undo), "nothing before the script started")
        app.command.Redo()
        assert(sprite:getPixel(1, 1) == 0x0000FFFF)
        app.command.Redo()
        assert(#sprite.frames == 2)
        assert(not pcall(app.command.Redo))
        app.undo() assert(#sprite.frames == 1)
        app.redo() assert(#sprite.frames == 2)

        -- a failing transaction is rolled back, a good one keeps its changes
        local ok = pcall(app.transaction, function()
          sprite:setPixel(0, 0, 0xFF0000FF)
          error("no")
        end)
        assert(not ok and sprite:getPixel(0, 0) == 0)
        local r = app.transaction("Paint", function() sprite:setPixel(0, 0, 0x00FF00FF) return 42 end)
        assert(r == 42 and sprite:getPixel(0, 0) == 0x00FF00FF)
        app.transaction(function() sprite:setPixel(3, 3, 0x00FF00FF) end)
        app.command.Undo()                                       -- undoes the whole transaction
        assert(sprite:getPixel(3, 3) == 0 and sprite:getPixel(0, 0) == 0x00FF00FF)
        "#,
    );
    assert_eq!(e.history_len().0, 1, "the whole run is one undo step");
    assert_eq!(e.history_label(0), Some("Paint"));
}

#[test]
fn commands_transform_the_sprite() {
    let mut e = editor(3, 2);
    plain(
        &mut e,
        r#"
        sprite:setPixel(0, 0, 0xFF0000FF)
        app.command.FlipHorizontal()
        assert(sprite:getPixel(2, 0) == 0xFF0000FF and sprite:getPixel(0, 0) == 0)
        app.command.FlipVertical()
        assert(sprite:getPixel(2, 1) == 0xFF0000FF)
        app.command.Rotate{ angle = 90 }
        assert(sprite.width == 2 and sprite.height == 3 and sprite:getPixel(0, 2) == 0xFF0000FF)
        app.command.Rotate{ angle = 180 }
        assert(sprite:getPixel(1, 0) == 0xFF0000FF)
        app.command.Rotate{ angle = 270 }
        assert(sprite.width == 3 and sprite.height == 2)
        assert(not pcall(app.command.Rotate, { angle = 45 }))
        app.command.Flip{ orientation = "horizontal", target = "canvas" }
        app.command.Flip{ orientation = "vertical" }                -- the cel (selection) only
        assert(not pcall(app.command.Flip, {}))

        app.command.CanvasSize{ left = 1, top = 2, right = 3, bottom = 4 }
        assert(sprite.width == 3 + 1 + 3 and sprite.height == 2 + 2 + 4)
        app.command.SpriteSize{ width = 14, height = 16 }
        assert(sprite.width == 14 and sprite.height == 16)
        app.command.SpriteSize{ scale = 0.5 }
        assert(sprite.width == 7 and sprite.height == 8)
        app.command.ChangePixelFormat{ format = "gray" }
        assert(sprite.colorMode == ColorMode.GRAY)
        app.command.ChangePixelFormat{ format = "rgb" }
        assert(sprite.colorMode == ColorMode.RGB)
        app.command.ChangePixelFormat{ format = "indexed" }
        assert(sprite.colorMode == ColorMode.INDEXED)
        assert(not pcall(app.command.ChangePixelFormat, { format = "cmyk" }))
        "#,
    );
}

#[test]
fn commands_filters_selection_and_cels() {
    let mut e = editor(8, 8);
    plain(
        &mut e,
        r#"
        sprite:setPixel(4, 4, 0xFF0000FF)
        app.command.ReplaceColor{ from = Color(255, 0, 0), to = Color(0, 0, 255), tolerance = 0 }
        assert(sprite:getPixel(4, 4) == 0x0000FFFF)
        app.command.InvertColor()
        assert(sprite:getPixel(4, 4) == 0xFFFF00FF)
        app.command.Outline{ color = Color(0, 255, 0) }
        assert(sprite:getPixel(3, 3) == 0x00FF00FF and sprite:getPixel(5, 4) == 0x00FF00FF)
        assert(sprite:getPixel(2, 2) & 0xFF == 0, "no outline two pixels away")

        -- AutocropSprite keeps the box around all content
        app.command.AutocropSprite()
        assert(sprite.width == 3 and sprite.height == 3 and sprite:getPixel(1, 1) == 0xFFFF00FF)

        -- selection commands
        app.command.SelectAll()
        assert(not sprite.selection.isEmpty and sprite.selection.bounds == Rectangle(0, 0, 3, 3))
        app.command.DeselectMask()
        assert(sprite.selection.isEmpty)
        sprite.selection:select(Rectangle(0, 0, 2, 3))
        app.command.InvertMask()
        assert(sprite.selection.bounds == Rectangle(2, 0, 1, 3))
        app.command.Clear()                                        -- clears the selected column only
        assert(sprite:getPixel(2, 0) == 0 and sprite:getPixel(1, 1) == 0xFFFF00FF)
        app.command.DeselectMask()
        app.command.MaskContent()
        assert(not sprite.selection.isEmpty)
        app.command.ClearCel()
        assert(sprite:cel(1, 1).isEmpty)
        assert(not pcall(app.command.Zoom), "user interface commands do not exist")
        local ok, err = pcall(app.command.Zoom)
        assert(err:find("not available"), err)
        "#,
    );
}

#[test]
fn commands_files_and_sheets() {
    let dir = Dir::new("cmdfiles");
    let mut e = editor(2, 2);
    let src = format!(
        r#"
        sprite:setPixel(0, 0, 0xFF0000FF)
        sprite:newFrame(true)
        app.command.SaveFileCopyAs{{ filename = {copy} }}
        assert(app.fs.isFile({copy}) and sprite.filename == "")
        app.command.SaveFileAs{{ filename = {saved} }}
        assert(sprite.filename == {saved})
        app.command.SaveFile()
        app.command.ExportSpriteSheet{{
          type = SpriteSheetType.HORIZONTAL, textureFilename = {tex}, dataFilename = {data}
        }}
        assert(app.fs.isFile({tex}) and app.fs.isFile({data}))
        app.command.NewFile{{ width = 5, height = 6 }}
        assert(app.activeSprite.width == 5 and #app.sprites == 2)
        app.command.DuplicateSprite()
        assert(#app.sprites == 3)
        app.command.CloseFile()
        app.command.CloseFile()
        assert(#app.sprites == 1 and app.activeSprite == sprite)
        app.command.OpenFile{{ filename = {copy} }}
        assert(#app.sprites == 2 and app.activeSprite.width == 2)
        assert(not pcall(app.command.ExportSpriteSheet, {{}}))
        "#,
        copy = dir.lua("copy.aseprite"),
        saved = dir.lua("saved.aseprite"),
        tex = dir.lua("sheet.png"),
        data = dir.lua("sheet.json"),
    );
    let o = good(&mut e, &src, &cfg(&dir));
    assert_eq!(o.original_path.as_deref(), Some(dir.path("saved.aseprite").as_path()));
    let (w, h, _) = crate::formats::import::decode_png(&std::fs::read(dir.path("sheet.png")).unwrap()).unwrap();
    assert_eq!((w, h), (4, 2), "two 2x2 frames side by side");
    assert!(std::fs::read_to_string(dir.path("sheet.json")).unwrap().contains("\"frames\""));
    assert_eq!(o.new_documents.len(), 1);
}

#[test]
fn layer_frame_and_cel_objects() {
    let mut e = editor(4, 4);
    plain(
        &mut e,
        r#"
        local layer = sprite:newLayer("two")
        local cel = sprite:newCel(layer, 1, Image(2, 2), Point(1, 1))
        assert(cel.layer == layer and cel.frame == sprite.frames[1] and cel.sprite == sprite)
        assert(cel.layer.name == "two" and cel.frame.frameNumber == 1 and cel.frameNumber == 1)
        assert(layer.stackIndex == 2 and layer.sprite == sprite and layer.isImage and not layer.isGroup)
        assert(app.activeLayer == sprite.layers[1] and app.activeFrame == sprite.frames[1])
        assert(app.activeCel == nil, "the active cel is empty")
        sprite:setPixel(0, 0, 0xFF0000FF)
        assert(app.activeCel ~= nil and app.activeImage ~= nil and app.site.sprite == sprite)
        assert(app.site.layer == app.activeLayer and app.site.frame.frameNumber == 1)
        layer.isVisible = false
        assert(layer.isVisible == false and not sprite.layers[2].visible)
        -- layers can be found by name; the old numbers keep working
        sprite:deleteLayer("two")
        assert(#sprite.layers == 1)
        local f = sprite:newFrame()
        sprite:deleteFrame(f)
        assert(#sprite.frames == 1)
        local t = sprite:newTag("x", 1, 1)
        sprite:deleteTag(t)
        assert(#sprite.tags == 0)
        local e2 = sprite:newEmptyFrame(1)
        assert(#sprite.frames == 2 and e2.frameNumber == 1 and sprite:cel(1, 2).isEmpty == false)
        sprite.layers[1].data = "hello"
        assert(sprite.layers[1].data == "hello")
        "#,
    );
}

// MARK: Community-style scripts, unchanged

#[test]
fn community_script_palette_swap() {
    let mut e = editor(4, 4);
    plain(
        &mut e,
        r#"
        -- Grayscale palette + swap the red and blue channels of the active cel.
        if app.apiVersion < 1 then return app.alert("This script requires a newer version") end
        local spr = app.activeSprite
        if not spr then return app.alert("There is no active sprite") end

        local pal = spr.palettes[1]
        app.transaction("Grayscale palette", function()
          for i = 0, #pal - 1 do
            local c = pal:getColor(i)
            local v = math.floor(0.299 * c.red + 0.587 * c.green + 0.114 * c.blue + 0.5)
            pal:setColor(i, Color{ r = v, g = v, b = v, a = c.alpha })
          end
        end)

        sprite:setPixel(0, 0, 0xFF0000FF)
        sprite:setPixel(1, 0, 0x00FF00FF)
        local cel = app.activeCel
        if not cel then return app.alert("There is no active cel") end
        local img = cel.image:clone()
        for it in img:pixels() do
          local px = it()
          local r, g, b, a = app.pixelColor.rgbaR(px), app.pixelColor.rgbaG(px), app.pixelColor.rgbaB(px), app.pixelColor.rgbaA(px)
          it(app.pixelColor.rgba(b, g, r, a))
        end
        cel.image = img
        app.refresh()
        for i = 0, #pal - 1 do
          local c = pal:getColor(i)
          assert(c.red == c.green and c.green == c.blue, "palette entry " .. i)
        end
        "#,
    );
    assert_eq!(&e.sprite.render(0)[..4], &[0, 0, 255, 255], "red became blue");
    assert_eq!(&e.sprite.render(0)[4..8], &[0, 255, 0, 255], "green stayed");
    assert!(e.sprite.palette.iter().all(|c| c[0] == c[1] && c[1] == c[2]));
}

#[test]
fn community_script_export_layers() {
    let dir = Dir::new("exportlayers");
    let mut e = editor(4, 4);
    e.sprite.layers[0].name = "Bottom".into();
    let src = format!(
        r#"
        -- Export every layer to its own PNG next to the sprite (a popular community script).
        sprite:setPixel(0, 0, 0xFF0000FF)
        local top = sprite:newLayer("Top")
        sprite:setPixel(3, 3, 0x00FF00FF, top.index, 1)

        local spr = app.activeSprite
        local dir = {dir}
        local names = {{}}
        for _, layer in ipairs(spr.layers) do
          local copy = Sprite(spr)
          for _, l in ipairs(copy.layers) do l.isVisible = (l.name == layer.name) end
          local file = app.fs.joinPath(dir, "sheet-" .. layer.name .. ".png")
          copy:saveCopyAs(file)
          copy:close()
          names[#names + 1] = app.fs.fileName(file)
        end
        app.activeSprite = spr
        print(table.concat(names, ","))
        "#,
        dir = dir.lua(""),
    );
    let o = good(&mut e, &src, &cfg(&dir));
    assert_eq!(o.messages, vec!["sheet-Bottom.png,sheet-Top.png"]);
    assert!(o.new_documents.is_empty(), "every copy was closed");
    assert_eq!(o.active, ActiveDoc::Original);
    let read = |name: &str| crate::formats::import::decode_png(&std::fs::read(dir.path(name)).unwrap()).unwrap();
    let (w, _, bottom) = read("sheet-Bottom.png");
    assert_eq!(&bottom[..4], &[255, 0, 0, 255]);
    assert_eq!(&bottom[(3 * w + 3) * 4..][..4], &[0, 0, 0, 0], "the top layer is hidden in this copy");
    let (_, _, top) = read("sheet-Top.png");
    assert_eq!(&top[..4], &[0, 0, 0, 0]);
    assert_eq!(&top[(3 * w + 3) * 4..][..4], &[0, 255, 0, 255]);
}

#[test]
fn community_script_batch_generate_sprites() {
    let dir = Dir::new("batch");
    let mut e = editor(2, 2);
    let src = format!(
        r#"
        -- Generate three gradient sprites and save them (batch new sprite + newCel).
        local out = {out}
        for i = 1, 3 do
          local spr = Sprite(8, 8, ColorMode.RGB)
          local img = Image(spr.spec)
          for it in img:pixels() do
            it(app.pixelColor.rgba(i * 40, it.x * 30, it.y * 30, 255))
          end
          spr:newCel(spr.layers[1], 1, img, Point(0, 0))
          spr:saveAs(app.fs.joinPath(out, "gen" .. i .. ".aseprite"))
          spr:close()
        end
        local files = app.fs.listFiles(out)
        table.sort(files)
        print(#files, files[1], files[3])
        "#,
        out = dir.lua(""),
    );
    let o = good(&mut e, &src, &cfg(&dir));
    assert_eq!(o.messages, vec!["3\tgen1.aseprite\tgen3.aseprite"]);
    for i in 1..=3u8 {
        let s = crate::formats::pxmt::open_file(dir.path(&format!("gen{i}.aseprite"))).unwrap();
        let px = s.render(0);
        let at = |x: usize, y: usize| &px[(y * 8 + x) * 4..][..4];
        assert_eq!(at(0, 0), &[i * 40, 0, 0, 255]);
        assert_eq!(at(7, 5), &[i * 40, 210, 150, 255]);
    }
    assert_eq!(e.history_len().0, 0);
}

#[test]
fn community_script_pixel_art_with_commands_and_dialog_free_flow() {
    let mut e = editor(8, 8);
    plain(
        &mut e,
        r#"
        -- Add an outline layer around the sprite's art, then trim and recolor (typical "polish" script).
        local spr = app.activeSprite
        local main = app.activeLayer
        local img = app.activeImage
        sprite:setPixel(3, 3, 0xFF0000FF)
        sprite:setPixel(4, 3, 0xFF0000FF)
        app.command.NewLayer{ name = "Outline" }
        local outline = app.activeLayer
        assert(outline.name == "Outline" and outline ~= main)
        local cel = spr:newCel(outline, app.activeFrame, Image(spr.spec), Point(0, 0))
        local src = main:cel(1).image
        local dst = cel.image or Image(spr.spec)
        local seen = 0
        for it in src:pixels() do
          if app.pixelColor.rgbaA(it()) > 0 then
            seen = seen + 1
            for _, d in ipairs({ { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } }) do
              local x, y = it.x + d[1], it.y + d[2]
              if app.pixelColor.rgbaA(src:getPixel(x, y)) == 0 then dst:putPixel(x + src.cel.position.x, y + src.cel.position.y, Color(0, 0, 0)) end
            end
          end
        end
        assert(seen == 2)
        spr:newCel(outline, 1, dst, Point(0, 0))
        app.command.MergeDownLayer()
        assert(#spr.layers == 1)
        app.command.AutocropSprite()
        assert(spr.width == 4 and spr.height == 3, spr.width .. "x" .. spr.height)
        app.command.ReplaceColor{ from = Color(0, 0, 0), to = Color(10, 10, 10) }
        assert(spr:getPixel(0, 1) == 0x0A0A0AFF)
        "#,
    );
}

// MARK: C ABI

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap()
}

#[test]
fn run_script_named_reports_documents_through_the_c_abi() {
    unsafe {
        let doc = Box::into_raw(Box::new(editor(2, 2)));
        let src = cstr("local s = Sprite(3, 2) s.layers[1].name = 'n' print(sprite.filename)");
        let (name, path) = (cstr("named-script"), cstr("/some/doc.aseprite"));
        let mut out: *mut c_char = std::ptr::null_mut();
        assert!(pixmith_run_script_named(doc, src.as_ptr(), 0, 0, name.as_ptr(), path.as_ptr(), &mut out));
        let msg = CStr::from_ptr(out).to_str().unwrap().to_string();
        crate::ffi::pixmith_string_free(out);
        assert_eq!(msg, "/some/doc.aseprite");

        let json = host::pixmith_script_outcome(0, 0);
        let text = CStr::from_ptr(json).to_str().unwrap().to_string();
        crate::ffi::pixmith_string_free(json);
        let j = crate::formats::json::parse(&text).unwrap();
        let new = j.get("new").and_then(|v| v.as_array()).unwrap();
        assert_eq!(new.len(), 1, "{text}");
        assert_eq!(new[0].get("modified"), Some(&crate::formats::json::Value::Bool(true)));
        assert_eq!(j.get("active").and_then(|v| v.as_f64()), Some(0.0));
        assert_eq!(j.get("closeOriginal"), Some(&crate::formats::json::Value::Bool(false)));

        let d = host::pixmith_script_take_doc(0, 0, 0);
        assert!(!d.is_null());
        let made = &*d;
        assert_eq!((made.sprite.width, made.sprite.height, made.sprite.layers[0].name.as_str()), (3, 2, "n"));
        drop(Box::from_raw(d));
        assert!(host::pixmith_script_take_doc(0, 0, 0).is_null(), "taken once");
        assert!(host::pixmith_script_take_doc(0, 0, 5).is_null());

        // A run without documents leaves "{}".
        let plain = cstr("print('x')");
        assert!(pixmith_run_script_at(doc, plain.as_ptr(), 0, 0, std::ptr::null_mut()));
        let json = host::pixmith_script_outcome(0, 0);
        assert_eq!(CStr::from_ptr(json).to_str().unwrap(), "{}");
        crate::ffi::pixmith_string_free(json);
        drop(Box::from_raw(doc));
    }
}
