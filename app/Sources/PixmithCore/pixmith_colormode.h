// Color modes (core/src/colormode.rs). Colors are 0xRRGGBBAA.
#pragma once
#include "pixmith.h"

// Modes: 0 = RGBA, 1 = grayscale, 2 = indexed.
uint32_t pixmith_cm_get_mode(const PixmithDoc *doc);
uint32_t pixmith_cm_transparent_index(const PixmithDoc *doc);
// One undo step. dither: Bayer matrix size (0 = none, 2, 4, 8). False if nothing changed.
bool pixmith_cm_convert(PixmithDoc *doc, uint32_t mode, uint32_t dither);
// Constrains a drawing color to the color mode (nearest palette color when indexed).
uint32_t pixmith_cm_snap_color(const PixmithDoc *doc, uint32_t rgba);
// Snaps every pixel to the palette (indexed) or to gray (grayscale), one undo step. False if
// nothing changed.
bool pixmith_cm_conform_all(PixmithDoc *doc);
// First palette index exactly equal to rgba, or -1.
int32_t pixmith_cm_palette_index_of(const PixmithDoc *doc, uint32_t rgba);
// One undo step; in indexed mode pixels using the entry are recolored.
bool pixmith_cm_palette_set(PixmithDoc *doc, uint32_t index, uint32_t rgba);
// Replaces the palette with the sprite's colors (0 = exact up to 256, else quantized to N).
// Returns the new palette size, 0 on failure.
uint32_t pixmith_cm_palette_from_sprite(PixmithDoc *doc, uint32_t max_colors);
