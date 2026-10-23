//! Writes the sprites used by scripts/godot-check.sh and a manifest of what Godot must find.
//! Usage: cargo run --example godot_fixtures -- <out dir>

use pixmith_core::document::{Sprite, Tag};
use pixmith_core::formats::ase;
use pixmith_core::formats::sheet::json_str;
use pixmith_core::gamedev::godot::sprite_frames::expand_frames;

fn tag(name: &str, from: u16, to: u16, direction: u8, repeat: u16) -> Tag {
    Tag { name: name.into(), from, to, direction, repeat, color: [0; 3], user_data: Default::default() }
}

fn hero() -> Sprite {
    let mut s = Sprite::new(8, 8);
    for f in 1..6 {
        s.add_frame(f - 1, false);
    }
    for f in 0..6 {
        // Each frame: a distinct 2x2 block so frames never dedupe and trim has something to crop.
        let (x, y) = ((1 + f % 3 * 2) as i32, (2 + f / 3 * 3) as i32);
        let color = [40 * f as u8 + 20, 200 - 20 * f as u8, 90, 255];
        s.draw_line(0, f, x, y, x + 1, y, color, 1);
        s.draw_line(0, f, x, y + 1, x + 1, y + 1, color, 1);
    }
    s.durations = vec![100, 150, 50, 100, 100, 80];
    s.tags = vec![
        tag("idle", 0, 1, 0, 0),
        tag("run", 0, 3, 2, 0),
        tag("hit", 4, 5, 1, 1),
        tag("\u{8d70}\u{308b} \"x\"", 2, 4, 3, 0),
    ];
    s
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: godot_fixtures <out dir>");
    std::fs::create_dir_all(&dir).unwrap();
    let s = hero();
    std::fs::write(format!("{dir}/hero.aseprite"), ase::write(&s)).unwrap();

    let mut anims = Vec::new();
    for t in &s.tags {
        let ms: Vec<String> = expand_frames(t.from as usize, t.to as usize, t.direction)
            .iter()
            .map(|&f| s.durations[f].to_string())
            .collect();
        anims.push(format!("{{\"name\":{},\"loop\":{},\"ms\":[{}]}}", json_str(&t.name), t.repeat == 0, ms.join(",")));
    }
    std::fs::write(
        format!("{dir}/manifest.json"),
        format!("{{\"canvas\":[{},{}],\"animations\":[{}]}}\n", s.width, s.height, anims.join(",")),
    )
    .unwrap();
}
