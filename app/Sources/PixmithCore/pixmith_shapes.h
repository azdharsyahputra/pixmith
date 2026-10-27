// Drawing tools and palette files (core/src/shapes.rs, core/src/palette_io.rs).
// Option flags: 1 mirror left/right, 2 mirror top/bottom, 4 pixel-perfect, 8 filled, 16 constrain.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "pixmith.h"

// kind: 0 line, 1 rectangle, 2 ellipse. With restore, rewinds to the pixmith_begin_stroke
// checkpoint first, so repeated calls while dragging preview in a single undo step.
void pixmith_shape(PixmithDoc *doc, uint32_t kind, uint32_t layer, uint32_t frame, int32_t x0, int32_t y0,
                int32_t x1, int32_t y1, uint32_t rgba, uint32_t size, uint32_t flags, bool restore);
// Symmetry axes as twice their position in pixels from the left / top edge (even: between pixels, odd: through a
// pixel); a negative value is the canvas center. Applies to drawing on the calling thread.
void pixmith_set_mirror_axes(int32_t x2, int32_t y2);
// The end of a line from (x0, y0) toward (x1, y1) snapped to a pixel-art angle (0, 2:1, 45, 1:2, 90 degrees).
void pixmith_constrain_line(int32_t x0, int32_t y0, int32_t x1, int32_t y1, int32_t *out);
void pixmith_stroke_begin(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y,
                       uint32_t rgba, uint32_t size, uint32_t flags);
void pixmith_stroke_to(PixmithDoc *doc, int32_t x, int32_t y);
void pixmith_stroke_end(void);
void pixmith_stroke_end_doc(PixmithDoc *doc); // ends only that document's running stroke
// Drops the latest undo step and restores its state without leaving a redo step.
bool pixmith_rollback(PixmithDoc *doc);
void pixmith_spray(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, uint32_t rgba,
                uint32_t radius, uint32_t density, uint32_t seed, uint32_t flags);
// Blur (kind 0) and Jumble (kind 1) brush tools; the stroke is one undo step when begun after pixmith_begin_stroke.
// strength 1...100 (blur: how far toward the neighbourhood average; jumble: chance a pixel is swapped), seed drives
// Jumble, flags are the mirror bits. Uses the global brush shape. core/src/paint/retouch.rs
void pixmith_retouch_begin(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, uint32_t kind,
                        uint32_t size, uint32_t strength, uint32_t seed, uint32_t flags);
void pixmith_retouch_to(PixmithDoc *doc, int32_t x, int32_t y);
void pixmith_retouch_size(PixmithDoc *doc, uint32_t size); // pen pressure: resize the running stroke
void pixmith_retouch_end_doc(PixmithDoc *doc);
void pixmith_fill_mode(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y,
                    uint32_t rgba, bool contiguous);

bool pixmith_palette_load(PixmithDoc *doc, const char *path);
bool pixmith_palette_save(const PixmithDoc *doc, const char *path);
bool pixmith_palette_remove(PixmithDoc *doc, uint32_t index);
void pixmith_palette_sort(PixmithDoc *doc, uint32_t mode); // 0 hue, 1 luminance
