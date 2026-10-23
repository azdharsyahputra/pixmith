// Ink modes, custom brushes, gradients and pattern fills (core/src/ink.rs).
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "pixmith.h"

// mode: 0 simple, 1 alpha compositing, 2 lock alpha, 3 behind, 4 shading. The ramp holds the
// 0xRRGGBBAA colors shading steps through; reverse steps toward its start. Applies to later
// pencil, shape and spray calls.
void pixmith_ink_set(uint32_t mode, bool reverse, const uint32_t *ramp, size_t n);

void pixmith_brush_circle(bool circle);
bool pixmith_brush_from_selection(const PixmithDoc *doc, uint32_t layer, uint32_t frame);
void pixmith_brush_reset(void);
void pixmith_brush_info(int32_t *out); // [has_custom, width, height]

// kind: 0 linear, 1 radial. dither: 0 smooth, else Bayer matrix size 2, 4 or 8. With restore,
// rewinds to the pixmith_begin_stroke checkpoint first. Fills the selection, or the whole cel.
void pixmith_gradient(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x0, int32_t y0, int32_t x1,
                   int32_t y1, uint32_t c0, uint32_t c1, uint32_t kind, uint32_t dither, bool restore);
// pattern: 0 solid, 1 checker, 2 25%, 3 50%, 4 75% of color a (the rest is color b). Paints
// through the current ink; flags 1 / 2 also fill under the seed mirrored left/right, top/bottom.
void pixmith_fill_pattern(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, uint32_t a,
                       uint32_t b, uint32_t pattern, bool contiguous, uint32_t flags);
