// Curves, polygons, bitmap text and pen pressure (core/src/curves.rs, font.rs, shapes.rs).
// Option flags are those of pixmith_shapes.h. With restore, the call first rewinds to the
// pixmith_begin_stroke checkpoint, so repeated calls preview in a single undo step.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "pixmith.h"

// pts: x0,y0 (start), x1,y1 (control 1), x2,y2 (control 2), x3,y3 (end).
void pixmith_curve(PixmithDoc *doc, uint32_t layer, uint32_t frame, const int32_t *pts, uint32_t rgba,
                uint32_t size, uint32_t flags, bool restore);
// Closed polygon through n points given as x,y pairs; flag 8 fills the interior, flag 32 leaves
// the path open (and unfilled).
void pixmith_polygon(PixmithDoc *doc, uint32_t layer, uint32_t frame, const int32_t *pts, size_t n,
                  uint32_t rgba, uint32_t size, uint32_t flags, bool restore);

// Built-in 5x8 font; each font pixel becomes scale x scale pixels. Top-left at (x, y).
void pixmith_text(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, const char *text,
               uint32_t scale, uint32_t rgba, uint32_t flags, bool restore);
void pixmith_text_size(const char *text, uint32_t scale, int32_t *out); // [width, height]
// Paints through a w*h coverage mask (values above 127 are ink), e.g. rasterized system fonts.
void pixmith_stamp_mask(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, uint32_t w,
                     uint32_t h, const uint8_t *mask, uint32_t rgba, uint32_t flags, bool restore);

// Changes color and brush size of the running freehand stroke (pen pressure).
void pixmith_stroke_style(PixmithDoc *doc, uint32_t rgba, uint32_t size);
