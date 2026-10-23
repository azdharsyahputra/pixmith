// Godot 4 TileSet export (core/src/godot/tileset.rs).
#pragma once
#include "pixmith.h"

typedef struct PixmithGodotOptions {
    // Terrain mode used when the document has no auto-tile data: 0 blob (Match Corners and Sides),
    // 1 corners, 2 sides.
    uint32_t mode;
    // 0 none, 1 full tile square, 2 polygons traced from opaque pixels.
    uint32_t collision;
    uint32_t margin;
    uint32_t separation;
    // Pixels of edge replication around each tile (raises margin/separation as needed).
    uint32_t extrude;
    // Atlas page size limit; 0 = 4096. More pages are written when tiles do not fit.
    uint32_t max_texture;
    // Also write <out>.tscn with TileMapLayer nodes painted from the tilemap layers (frame 1).
    bool write_scene;
    float terrain_color[4];
    // Null or empty = use the tileset's name.
    const char *terrain_name;
} PixmithGodotOptions;

// Writes <out_tres> (TileSet, format 3) and a PNG atlas with the same name next to it.
// `project_root` is the folder containing project.godot; output outside it is refused.
// Returns false on failure (see pixmith_last_error).
bool pixmith_export_godot_tileset(const PixmithDoc *doc, uint32_t tileset_index, const char *project_root, const char *out_tres);
bool pixmith_export_godot_tileset_ex(const PixmithDoc *doc, uint32_t tileset_index, const char *project_root,
                                  const char *out_tres, const PixmithGodotOptions *options);

// Describes what an export would write, without writing anything (null on failure).
// Release with pixmith_string_free.
char *pixmith_godot_tileset_preview(const PixmithDoc *doc, uint32_t tileset_index, const char *project_root,
                                 const char *out_tres, const PixmithGodotOptions *options);

// Warnings of the last export on this thread (empty string if none). Release with pixmith_string_free.
char *pixmith_godot_tileset_warning(void);
