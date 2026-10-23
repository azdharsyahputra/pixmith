// Incremental rendering for viewers (core/src/view.rs).
#pragma once
#include "pixmith.h"

// Composites the rectangle (x, y, w, h) of `frame` into `out` (w * h * 4 bytes, tightly packed, straight-alpha
// RGBA). Equals the same rectangle of pixmith_render. False if the rectangle is empty or outside the canvas, or
// `len` is too small.
bool pixmith_render_rect(const PixmithDoc *doc, uint32_t frame, uint32_t x, uint32_t y, uint32_t w, uint32_t h,
                      uint8_t *out, size_t len);

// What changed since the previous call (which starts over): 0 nothing, 1 a rectangle of one frame (`frame` and
// `rect` = x, y, w, h are written), 2 anything. A new document reports 2 first. Operations that do not report
// their extent count as 2, so a rectangle can be trusted.
int32_t pixmith_take_dirty_rect(PixmithDoc *doc, uint32_t *frame, int32_t *rect);
