// Tilemap layers and tilesets (core/src/tilemap.rs).
// A tile value is a tile index in the low 29 bits plus flip flags:
// 0x80000000 flip X, 0x40000000 flip Y, 0x20000000 diagonal flip. Tile index 0 is empty.
#pragma once
#include "pixmith.h"

bool pixmith_layer_is_tilemap(const PixmithDoc *doc, uint32_t layer);
int32_t pixmith_layer_tileset(const PixmithDoc *doc, uint32_t layer);

uint32_t pixmith_tileset_count(const PixmithDoc *doc);
uint32_t pixmith_tileset_tile_count(const PixmithDoc *doc, uint32_t tileset);
uint32_t pixmith_tileset_tile_width(const PixmithDoc *doc, uint32_t tileset);
uint32_t pixmith_tileset_tile_height(const PixmithDoc *doc, uint32_t tileset);
int32_t pixmith_tileset_base_index(const PixmithDoc *doc, uint32_t tileset);
void pixmith_tileset_set_base_index(PixmithDoc *doc, uint32_t tileset, int32_t base);
char *pixmith_tileset_name(const PixmithDoc *doc, uint32_t tileset);
bool pixmith_tileset_tile_pixels(const PixmithDoc *doc, uint32_t tileset, uint32_t tile, uint8_t *out, size_t len);
uint32_t pixmith_tileset_add_tile(PixmithDoc *doc, uint32_t tileset);

uint32_t pixmith_add_tilemap_layer(PixmithDoc *doc, uint32_t above, uint32_t tile_w, uint32_t tile_h);
bool pixmith_tile_stamp(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, uint32_t tile);
uint32_t pixmith_tile_at(const PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y);
bool pixmith_convert_to_tilemap(PixmithDoc *doc, uint32_t layer, uint32_t tile_w, uint32_t tile_h);
bool pixmith_convert_to_normal(PixmithDoc *doc, uint32_t layer);

// With flip_aware, flipped and transposed copies of a tile share it (using flip flags).
bool pixmith_convert_to_tilemap_ex(PixmithDoc *doc, uint32_t layer, uint32_t tile_w, uint32_t tile_h, bool flip_aware);
uint32_t pixmith_tileset_duplicate_tile(PixmithDoc *doc, uint32_t tileset, uint32_t tile);
bool pixmith_tileset_delete_tile(PixmithDoc *doc, uint32_t tileset, uint32_t tile);
bool pixmith_layer_assign_tileset(PixmithDoc *doc, uint32_t layer, uint32_t tileset);
bool pixmith_tileset_rename(PixmithDoc *doc, uint32_t tileset, const char *name);
// Writes the grid offset (x, y) inside the canvas, each in [0, tile size), into out[2].
bool pixmith_tile_grid_origin(const PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t *out);

// Isometric tilemaps (core/src/document/tilemap.rs, iso.rs, isoext.rs). 2:1 diamond cells, Godot DIAMOND_DOWN: cell
// (i, j) has its box at (gx + (i - j) * cell_w / 2, gy + (i + j) * cell_h / 2), where (gx, gy) is the layer's grid
// offset. The cel's grid is addressed by cell (i, j), not by pixel. Tile images may be taller than the diamond; the
// pivot of a tile is the point of its image (from the top-left) that sits on the diamond's center.
// pixmith_tile_grid_origin returns false for isometric layers (they have no square grid).
uint32_t pixmith_add_iso_tilemap_layer(PixmithDoc *doc, uint32_t above, uint32_t tile_w, uint32_t tile_h,
                                       uint32_t cell_w, uint32_t cell_h, int32_t offset_x, int32_t offset_y);
bool pixmith_tileset_is_iso(const PixmithDoc *doc, uint32_t tileset);
// Writes cell_w, cell_h into out[2]. False for square tilesets.
bool pixmith_tileset_iso_cell(const PixmithDoc *doc, uint32_t tileset, uint32_t *out);
// cell_w = 0 makes the tileset square again. Only while no layer shows a tile of it.
bool pixmith_tileset_set_iso(PixmithDoc *doc, uint32_t tileset, uint32_t cell_w, uint32_t cell_h);
bool pixmith_tile_pivot(const PixmithDoc *doc, uint32_t tileset, uint32_t tile, int32_t *out);
bool pixmith_tile_pivot_is_set(const PixmithDoc *doc, uint32_t tileset, uint32_t tile);
bool pixmith_tile_set_pivot(PixmithDoc *doc, uint32_t tileset, uint32_t tile, int32_t x, int32_t y);
bool pixmith_tile_clear_pivot(PixmithDoc *doc, uint32_t tileset, uint32_t tile);
// The pivot as Godot's TileData.texture_origin (offset from the image center), into out[2].
bool pixmith_tile_texture_origin(const PixmithDoc *doc, uint32_t tileset, uint32_t tile, int32_t *out);
bool pixmith_layer_is_iso(const PixmithDoc *doc, uint32_t layer);
// Canvas pixel of cell (0, 0)'s box, into out[2]. Layers at other elevations use other offsets.
bool pixmith_layer_grid_offset(const PixmithDoc *doc, uint32_t layer, int32_t *out);
bool pixmith_layer_set_grid_offset(PixmithDoc *doc, uint32_t layer, int32_t x, int32_t y);
// The cell (i, j) containing a canvas pixel (iso diamond; for square tilemaps pixel / tile size), into out[2].
bool pixmith_layer_cell_at(const PixmithDoc *doc, uint32_t layer, int32_t x, int32_t y, int32_t *out);
bool pixmith_layer_cell_origin(const PixmithDoc *doc, uint32_t layer, int32_t i, int32_t j, int32_t *out);
uint32_t pixmith_tile_in_cell(const PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t i, int32_t j);
// Like pixmith_tile_stamp, by cell. Call pixmith_begin_stroke first.
bool pixmith_tile_stamp_cell(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t i, int32_t j, uint32_t tile);
// The tile you see at a pixel (front-most visible tile): out[3] = i, j, tile value. False when empty.
bool pixmith_tile_hit_at(const PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, int32_t *out);
// Grid math without a document (overlays): cell containing a pixel / pixel of a cell's box corner, into out[2].
bool pixmith_iso_cell_at(int32_t w, int32_t h, int32_t gx, int32_t gy, int32_t x, int32_t y, int32_t *out);
bool pixmith_iso_cell_origin(int32_t w, int32_t h, int32_t gx, int32_t gy, int32_t i, int32_t j, int32_t *out);
