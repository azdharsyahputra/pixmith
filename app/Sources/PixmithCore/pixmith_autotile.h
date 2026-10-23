// Auto-tiling for tilemap layers (core/src/autotile.rs).
// State lives in the layer's user data (`pixmith.autotile.v1:` line). Modes: 0 sides, 1 blob, 2 corners (dual grid).
// Pixel coordinates are canvas pixels; the core converts them to cells. Ring directions for peers:
// 0 N, 1 NE, 2 E, 3 SE, 4 S, 5 SW, 6 W, 7 NW. Terrain ids are 1...15 (0 = none); priority = order.
// Calls marked "stroke" do not checkpoint: call pixmith_begin_stroke first (one undo step per gesture).
#pragma once
#include "pixmith.h"

// Terrains. rgba = 0xRRGGBBAA. Undoable.
bool pixmith_autotile_set_terrain(PixmithDoc *doc, uint32_t layer, const char *name, uint32_t rgba, int32_t mode);
bool pixmith_autotile_set_mode(PixmithDoc *doc, uint32_t layer, int32_t mode);
uint32_t pixmith_autotile_add_terrain(PixmithDoc *doc, uint32_t layer, const char *name, uint32_t rgba);
bool pixmith_autotile_remove_terrain(PixmithDoc *doc, uint32_t layer, uint32_t id);
bool pixmith_autotile_edit_terrain(PixmithDoc *doc, uint32_t layer, uint32_t id, const char *name, uint32_t rgba);
bool pixmith_autotile_move_terrain(PixmithDoc *doc, uint32_t layer, uint32_t id, uint32_t to);
uint32_t pixmith_autotile_terrain_count(const PixmithDoc *doc, uint32_t layer);
uint32_t pixmith_autotile_terrain_id(const PixmithDoc *doc, uint32_t layer, uint32_t index);
char *pixmith_autotile_terrain_name(const PixmithDoc *doc, uint32_t layer, uint32_t id);   // free with pixmith_string_free
uint32_t pixmith_autotile_terrain_color(const PixmithDoc *doc, uint32_t layer, uint32_t id);
int32_t pixmith_autotile_mode(const PixmithDoc *doc, uint32_t layer);                      // -1 = no terrain data

// Painting (stroke). terrain 0 erases. mirror: bit 0 horizontal, bit 1 vertical symmetry.
bool pixmith_autotile_paint(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, uint32_t terrain);
bool pixmith_autotile_paint_brush(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, uint32_t terrain, uint32_t size, bool round, uint32_t mirror);
bool pixmith_autotile_rect(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x0, int32_t y0, int32_t x1, int32_t y1, uint32_t terrain, uint32_t mirror);
bool pixmith_autotile_flood(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, uint32_t terrain, uint32_t mirror);
uint32_t pixmith_autotile_terrain_at(const PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y);   // eyedropper
// Manual override (stroke): tile != 0 sets, 0 clears (auto result returns). Stamping with the tile tool also sets one.
bool pixmith_autotile_set_override(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, uint32_t tile);
bool pixmith_autotile_clear_overrides(PixmithDoc *doc, uint32_t layer, uint32_t frame);   // undoable
bool pixmith_autotile_rebuild(PixmithDoc *doc, uint32_t layer);                            // undoable "Rebuild all"

// Tile data. Undoable. mask < 0 removes the tile's data.
bool pixmith_autotile_set_tile_mask(PixmithDoc *doc, uint32_t layer, uint32_t tile, int32_t mask);
bool pixmith_autotile_set_tile_weight(PixmithDoc *doc, uint32_t layer, uint32_t tile, float weight);
bool pixmith_autotile_set_tile_terrain(PixmithDoc *doc, uint32_t layer, uint32_t tile, uint32_t terrain);
bool pixmith_autotile_set_tile_peer(PixmithDoc *doc, uint32_t layer, uint32_t tile, uint32_t dir, uint32_t terrain);
bool pixmith_autotile_set_tile_sym(PixmithDoc *doc, uint32_t layer, uint32_t tile, uint32_t sym);   // 1 flipX, 2 flipY, 4 diagonal
int32_t pixmith_autotile_tile_mask(const PixmithDoc *doc, uint32_t layer, uint32_t tile);          // -1 = none
int32_t pixmith_autotile_tile_terrain(const PixmithDoc *doc, uint32_t layer, uint32_t tile);       // 0 none, -1 no data
int32_t pixmith_autotile_tile_peer(const PixmithDoc *doc, uint32_t layer, uint32_t tile, uint32_t dir);
uint32_t pixmith_autotile_tile_sym(const PixmithDoc *doc, uint32_t layer, uint32_t tile);
float pixmith_autotile_tile_weight(const PixmithDoc *doc, uint32_t layer, uint32_t tile);

// Options (undoable setters).
bool pixmith_autotile_set_edge_same(PixmithDoc *doc, uint32_t layer, bool same);
bool pixmith_autotile_edge_same(const PixmithDoc *doc, uint32_t layer);
bool pixmith_autotile_set_seed(PixmithDoc *doc, uint32_t layer, uint32_t seed);
uint32_t pixmith_autotile_seed(const PixmithDoc *doc, uint32_t layer);

// Missing tiles: top-left canvas pixel (x, y pairs) of fallback cells; returns the cell count.
size_t pixmith_autotile_missing_cells(const PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t *out, size_t cap);
char *pixmith_autotile_missing_report(const PixmithDoc *doc, uint32_t layer, uint32_t frame);       // newline separated; free with pixmith_string_free

// Templates (undoable). reduced = only flip/rotation representatives (6 for sides/corners).
uint32_t pixmith_autotile_assign_masks_from_template(PixmithDoc *doc, uint32_t layer);
uint32_t pixmith_autotile_generate_template(PixmithDoc *doc, uint32_t layer, bool reduced);
