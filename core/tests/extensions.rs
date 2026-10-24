//! Extension packages made by real tools: the template packed by `scripts/extension-template/pack.sh` (zip or
//! python, deflated entries) installs, and hostile packages written with python's zipfile install nothing.

use pixmith_core::extensions::Store;
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-ext-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn have(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok() || Command::new(tool).arg("-v").output().is_ok()
}

#[test]
fn the_template_packs_and_installs() {
    let dir = temp("template");
    let out = dir.join("t.pixmith-extension");
    let status = Command::new("sh")
        .arg(repo_root().join("scripts/extension-template/pack.sh"))
        .arg(repo_root().join("scripts/extension-template"))
        .arg(&out)
        .output()
        .expect("sh runs");
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));

    let store = Store::new(dir.join("Extensions"));
    let ext = store.install_file(&out).expect("the template installs");
    assert_eq!(ext.manifest.name, "sample-extension");
    assert_eq!(ext.error, None);
    assert!(ext.active());
    assert_eq!(ext.palettes.len(), 1);
    assert_eq!(ext.palettes[0].name, "Sunset");
    assert_eq!(ext.palettes[0].colors.len(), 12);
    assert_eq!(ext.themes.len(), 1);
    assert!(ext.themes[0].theme.styles_dark() && ext.themes[0].theme.styles_light());
    assert_eq!(ext.themes[0].theme.get("accent", true), Some(0xFF6AC1FF));
    assert_eq!(ext.scripts.len(), 1);
    let source = std::fs::read_to_string(&ext.scripts[0].path).unwrap();
    assert!(source.contains("function init(plugin)"));
    assert!(!ext.dir.join("pack.sh").exists(), "the packing script is not part of the package");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hostile_zips_from_python_install_nothing() {
    if !have("python3") {
        eprintln!("python3 not found; skipping");
        return;
    }
    let dir = temp("hostile");
    let script = r#"
import sys, zipfile, stat
d = sys.argv[1]
good = '{"name":"x","version":"1"}'
def make(name, entries, attrs=None):
    with zipfile.ZipFile(f"{d}/{name}.pixmith-extension", "w", zipfile.ZIP_DEFLATED) as z:
        for n, data in entries:
            z.writestr(n, data)
make("traversal", [("package.json", good), ("../../escaped.txt", "x")])
make("absolute", [("package.json", good), ("/tmp/pixmith-escaped.txt", "x")])
make("backslash", [("package.json", good), ("..\\escaped.txt", "x")])
make("bomb", [("package.json", good), ("big.bin", b"\0" * (64 * 1024 * 1024))])
make("many", [("package.json", good)] + [(f"f{i}.txt", "x") for i in range(600)])
# A symlink entry.
with zipfile.ZipFile(f"{d}/link.pixmith-extension", "w") as z:
    z.writestr("package.json", good)
    info = zipfile.ZipInfo("link")
    info.create_system = 3
    info.external_attr = (stat.S_IFLNK | 0o777) << 16
    z.writestr(info, "/etc/passwd")
"#;
    let status = Command::new("python3").arg("-I").arg("-c").arg(script).arg(&dir).status().unwrap();
    assert!(status.success());
    let store = Store::new(dir.join("Extensions"));
    for name in ["traversal", "absolute", "backslash", "bomb", "many", "link"] {
        let r = store.install_file(&dir.join(format!("{name}.pixmith-extension")));
        assert!(r.is_err(), "{name} was installed");
    }
    assert!(store.list().is_empty());
    assert!(!dir.join("escaped.txt").exists() && !dir.parent().unwrap().join("escaped.txt").exists());
    assert!(!Path::new("/tmp/pixmith-escaped.txt").exists());
    let leftovers = std::fs::read_dir(dir.join("Extensions")).map(|d| d.count()).unwrap_or(0);
    assert_eq!(leftovers, 0, "no staging folders left behind");
    let _ = std::fs::remove_dir_all(dir);
}
