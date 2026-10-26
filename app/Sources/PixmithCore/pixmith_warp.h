// Pixel-art warp (puppet / mesh warp) of a cel or selection (core/src/paint/warp.rs). A `cols` x `rows` grid of
// handles (each 2...8) lies over the selection bounds, or the canvas when nothing is selected; every handle carries a
// displacement in pixels, interpolated bilinearly (or with a Catmull-Rom spline when `smooth`), and pins add radial
// displacements. The result is resampled by majority vote: no blur, no new colors, nothing outside the selection changes.
#pragma once
#include "pixmith.h"

// Writes the region the handle grid covers: x, y, width, height (4 values), the selection bounds or the canvas.
bool pixmith_warp_bounds(const PixmithDoc *doc, int32_t *out);

// `offsets`: cols * rows (dx, dy) pairs, row by row (cols * rows * 2 doubles); the content at a handle moves by its
// displacement. `samples`: points per output pixel along each axis, 1...8 (4 is a good default). `pins`: 5 doubles per
// pin -- x, y (canvas coordinates), dx, dy, radius -- `pin_count` of them (at most 32); may be null with 0 pins.

// Warps the cel at (layer, frame), limited to the selection, as a single undo step. Returns false (and records nothing)
// for bad arguments or if no pixel changed.
bool pixmith_warp_apply(PixmithDoc *doc, uint32_t layer, uint32_t frame, uint32_t cols, uint32_t rows,
                        const double *offsets, uint32_t samples, bool smooth, const double *pins, size_t pin_count);

// Renders `frame` as it would look with the warp applied, leaving the document and undo history untouched.
// `out` needs width * height * 4 bytes.
bool pixmith_warp_preview(const PixmithDoc *doc, uint32_t layer, uint32_t frame, uint32_t cols, uint32_t rows,
                          const double *offsets, uint32_t samples, bool smooth, const double *pins, size_t pin_count,
                          uint8_t *out, size_t len);
