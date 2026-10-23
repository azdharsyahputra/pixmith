// Selection, transform, clipboard and canvas-size API (core/src/selection.rs).
#pragma once
#include "pixmith.h"

bool pixmith_sel_active(const PixmithDoc *doc);
bool pixmith_sel_bounds(const PixmithDoc *doc, int32_t *out4); // x, y, w, h
bool pixmith_sel_contains(const PixmithDoc *doc, int32_t x, int32_t y);
void pixmith_sel_rect(PixmithDoc *doc, int32_t x0, int32_t y0, int32_t x1, int32_t y1);
void pixmith_sel_all(PixmithDoc *doc);
void pixmith_sel_clear(PixmithDoc *doc);
void pixmith_sel_invert(PixmithDoc *doc);
// Returns the segment count; writes up to cap segments of 4 ints (x0, y0, x1, y1) when out is non-null.
size_t pixmith_sel_outline(const PixmithDoc *doc, int32_t *out, size_t cap);

bool pixmith_sel_move_begin(PixmithDoc *doc, uint32_t layer, uint32_t frame);
void pixmith_sel_move_to(PixmithDoc *doc, int32_t dx, int32_t dy);
void pixmith_sel_move_end(PixmithDoc *doc);
bool pixmith_sel_nudge(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t dx, int32_t dy);

bool pixmith_sel_delete(PixmithDoc *doc, uint32_t layer, uint32_t frame);
void pixmith_sel_copy_bounds(const PixmithDoc *doc, int32_t *out4);
bool pixmith_sel_copy(const PixmithDoc *doc, uint32_t layer, uint32_t frame, uint8_t *out, size_t len);
bool pixmith_sel_paste(PixmithDoc *doc, uint32_t layer, uint32_t frame, uint32_t w, uint32_t h,
                    const uint8_t *data, int32_t x, int32_t y);
// op: 0 flip horizontal, 1 flip vertical, 2 rotate clockwise, 3 rotate counter-clockwise.
bool pixmith_sel_transform(PixmithDoc *doc, uint32_t layer, uint32_t frame, uint32_t op);

bool pixmith_resize_canvas(PixmithDoc *doc, uint32_t w, uint32_t h, int32_t ox, int32_t oy);
bool pixmith_crop_to_selection(PixmithDoc *doc);

// Interactive selection: mode 0 replace, 1 add, 2 subtract. begin records one undo step; rect/poly
// then update the selection live (combined with the selection that existed at begin) until end.
void pixmith_sel_begin(PixmithDoc *doc, uint32_t mode);
void pixmith_sel_end(PixmithDoc *doc);
void pixmith_sel_poly(PixmithDoc *doc, const int32_t *pts, size_t n); // n (x, y) pairs
bool pixmith_sel_wand(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y,
                   uint32_t tolerance, bool contiguous, uint32_t mode);
// kind: 0 expand, 1 contract, 2 border.
bool pixmith_sel_modify(PixmithDoc *doc, uint32_t kind, uint32_t n);

// A paste floats (stays movable via pixmith_sel_move_*) until the next edit or commit.
bool pixmith_float_active(const PixmithDoc *doc);
void pixmith_float_commit(PixmithDoc *doc);
bool pixmith_float_cancel(PixmithDoc *doc);
