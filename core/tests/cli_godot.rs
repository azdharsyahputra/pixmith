//! Integration tests for the Godot export commands of `pixmith-cli`.

use pixmith_core::document::{Sprite, Tag};
use pixmith_core::formats::ase;
use pixmith_core::gamedev::godot;
use std::path::PathBuf;
use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pixmith-cli")).args(args).output().unwrap()
}

fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixmith-cli-godot-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("project.godot"), "config_version=5\n").unwrap();
    dir
}

fn sample() -> Sprite {
    let mut s = Sprite::new(4, 4);
    s.draw_line(0, 0, 1, 1, 1, 1, [255, 0, 0, 255], 1);
    s.add_frame(0, true);
    s.draw_line(0, 1, 2, 2, 2, 2, [0, 0, 255, 255], 1);
    s.durations = vec![100, 150];
    s.tags.push(Tag {
        name: "run".into(),
        from: 0,
        to: 1,
        direction: 2,
        repeat: 0,
        color: [0; 3],
        user_data: Default::default(),
    });
    s
}

#[test]
fn exports_godot_files_with_all_options() {
    let dir = project("full");
    let input = dir.join("hero.aseprite");
    std::fs::write(&input, ase::write(&sample())).unwrap();
    let tres = dir.join("art/hero.tres");
    let out = cli(&[
        "export",
        input.to_str().unwrap(),
        "--godot-root",
        dir.to_str().unwrap(),
        "--godot-tres",
        tres.to_str().unwrap(),
        "--padding",
        "1",
        "--extrude",
        "--trim",
        "--max-texture",
        "2048",
        "--packing",
        "grid",
        "--godot-tscn",
        "--per-animation-png",
        "--scale",
        "2",
    ]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Nearest"));
    let text = std::fs::read_to_string(&tres).unwrap();
    assert!(text.contains("\"speed\": 20.0") && text.contains("res://art/hero_run.png"), "{text}");
    assert!(dir.join("art/hero_run.png").exists() && dir.join("art/hero.tscn").exists());
}

#[test]
fn godot_validation_and_settings() {
    let dir = project("val");
    let input = dir.join("hero.aseprite");
    std::fs::write(&input, ase::write(&sample())).unwrap();
    let i = input.to_str().unwrap();
    // Missing pair, bad values, outside root.
    assert_eq!(cli(&["export", i, "--godot-root", dir.to_str().unwrap()]).status.code(), Some(2));
    assert_eq!(cli(&["export", i, "--extrude", "--png", "x.png"]).status.code(), Some(2));
    assert_eq!(
        cli(&["export", i, "--godot-root", dir.to_str().unwrap(), "--godot-tres", "a.tres", "--packing", "x"])
            .status
            .code(),
        Some(2)
    );
    let outside = std::env::temp_dir().join("outside.tres");
    let out = cli(&["export", i, "--godot-root", dir.to_str().unwrap(), "--godot-tres", outside.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("outside"));
    assert!(!outside.exists());
    // Stored settings: none yet -> error; then saved in the document -> works.
    assert_eq!(cli(&["export", i, "--godot-settings"]).status.code(), Some(1));
    let mut s = sample();
    let st = godot::settings::Settings {
        root: dir.to_str().unwrap().into(),
        tres: dir.join("art/saved.tres").to_str().unwrap().into(),
        options: godot::Options { emit_tscn: true, ..Default::default() },
    };
    godot::settings::store(&mut s, &st.to_json());
    std::fs::write(&input, ase::write(&s)).unwrap();
    assert!(cli(&["export", i, "--godot-settings"]).status.success());
    assert!(dir.join("art/saved.tres").exists() && dir.join("art/saved.tscn").exists());
}
