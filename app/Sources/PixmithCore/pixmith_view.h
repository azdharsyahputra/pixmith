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

// Frame difference (core/src/render/diff.rs): which pixels `frame` changed compared with the frame before it.
typedef struct PixmithFrameDiff {
    uint32_t changed;       // not transparent in either frame, but the color or alpha differs
    uint32_t appeared;      // transparent before, not now
    uint32_t disappeared;   // not transparent before, transparent now
    int32_t reference_frame; // the frame compared against, -1 if there is none (then every count is 0)
    uint32_t x, y, w, h;    // bounding box of every differing pixel; w and h are 0 when none differs
} PixmithFrameDiff;

// Compares `frame` with the previous frame (frame 0 with the last one when `wrap`, else it has no reference).
// `layer` < 0 compares the composite of the visible layers, otherwise the cel of that non-group layer (visibility
// ignored). Writes the counts to `out` and, when `mask` is not null, width * height bytes (row-major: 0 same,
// 1 changed, 2 appeared, 3 disappeared; `len` is the size of `mask`). False for a bad frame, layer or document or a
// `mask` that is too small.
bool pixmith_frame_diff(const PixmithDoc *doc, uint32_t frame, int32_t layer, bool wrap, uint8_t *mask, size_t len,
                        PixmithFrameDiff *out);

// Like `pixmith_frame_diff` with a wrap mode instead of the flag: 0 = tag (a frame in a tag compares with the previous
// frame of the same tag, the first frame of a tag with the tag's last; frames outside tags wrap over the document),
// 1 = document (frame 0 with the last frame, `wrap == true` above), 2 = none (frame 0 has no reference). Other modes fail.
bool pixmith_frame_diff_mode(const PixmithDoc *doc, uint32_t frame, int32_t layer, int32_t mode, uint8_t *mask, size_t len,
                             PixmithFrameDiff *out);
