// Godot 4 export (core/src/godot/). Godot 3 (format=2) is not supported.
#pragma once
#include "pixmith.h"

// Writes <name>.tres (SpriteFrames, Godot 4 format=3) and the atlas PNG(s) next to it.
// `project_root` is the folder containing project.godot; `out_tres_path` must be inside it.
// `max_texture` 0 means 4096 (limit 16384). On success `*out_warnings` (may be null) receives
// newline separated warnings; release it with pixmith_string_free. On failure see pixmith_last_error.
bool pixmith_export_godot_sprite_frames(const PixmithDoc *doc, const char *project_root, const char *out_tres_path,
                                     uint32_t padding, bool extrude, bool trim, uint32_t max_texture,
                                     char **out_warnings);

// Full-option export. `settings_json` carries root, tres and all options (see core/src/godot/settings.rs).
bool pixmith_export_godot(const PixmithDoc *doc, const char *settings_json, char **out_warnings);

// Builds the export without writing it. Writes the first atlas page to `png_out_path`; returns the first
// .tres text, the files that would be written (newline separated) and the warnings (newline separated).
// Any out pointer may be null; release strings with pixmith_string_free.
bool pixmith_godot_preview(const PixmithDoc *doc, const char *settings_json, const char *png_out_path,
                        char **out_tres_text, char **out_files, char **out_warnings);

// Export settings stored in the document (versioned JSON in the sprite's user data), or null.
char *pixmith_godot_settings_get(const PixmithDoc *doc);
// Stores settings in the document; they are written with the next save. Call pixmith_touch afterwards.
bool pixmith_godot_settings_set(PixmithDoc *doc, const char *settings_json);
