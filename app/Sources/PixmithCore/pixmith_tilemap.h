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
