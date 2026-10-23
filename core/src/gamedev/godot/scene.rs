//! Optional ready-to-use `.tscn`: an AnimatedSprite2D with nearest filtering.

use super::sprite_frames::{ext_uid, header_uid, quote};

/// `player_idle` -> `PlayerIdle`; keeps letters and digits only.
pub fn node_name(stem: &str) -> String {
    let mut out = String::new();
    let mut up = true;
    for c in stem.chars() {
        if c.is_alphanumeric() {
            if up {
                out.extend(c.to_uppercase());
            } else {
                out.push(c);
            }
            up = false;
        } else {
            up = true;
        }
    }
    if out.is_empty() {
        "Sprite".into()
    } else {
        out
    }
}

/// `tres_path` is the `res://` path of the SpriteFrames, `tres_text` its content (for its uid).
/// `previous` is the old scene text: its scene uid and its uid for the SpriteFrames are kept.
pub fn tscn(stem: &str, tres_path: &str, tres_text: &str, first_animation: &str, previous: Option<&str>) -> String {
    let scene_uid = previous.and_then(header_uid).map(|u| format!(" uid=\"{u}\"")).unwrap_or_default();
    let res_uid = header_uid(tres_text)
        .or_else(|| previous.and_then(|p| ext_uid(p, tres_path)))
        .map(|u| format!(" uid=\"{u}\""))
        .unwrap_or_default();
    format!(
        "[gd_scene format=3{scene_uid}]\n\n\
         [ext_resource type=\"SpriteFrames\"{res_uid} path={} id=\"1_frames\"]\n\n\
         [node name={} type=\"AnimatedSprite2D\"]\n\
         texture_filter = 1\n\
         sprite_frames = ExtResource(\"1_frames\")\n\
         animation = &{}\n\
         autoplay = {}\n",
        quote(tres_path),
        quote(&node_name(stem)),
        quote(first_animation),
        quote(first_animation)
    )
}
