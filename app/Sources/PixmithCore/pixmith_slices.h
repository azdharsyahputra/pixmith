// Slices and user data API (core/src/slices.rs).
#pragma once
#include "pixmith.h"

uint32_t pixmith_slice_count(const PixmithDoc *doc);
char *pixmith_slice_name(const PixmithDoc *doc, uint32_t i);
// Writes 13 ints: x, y, w, h, has_center, cx, cy, cw, ch, has_pivot, px, py, key_frame.
// False when the slice has no key at or before `frame`.
bool pixmith_slice_get(const PixmithDoc *doc, uint32_t i, uint32_t frame, int32_t *out13);
int32_t pixmith_slice_add(PixmithDoc *doc, int32_t x, int32_t y, uint32_t w, uint32_t h);
// No undo step of its own: call pixmith_begin_stroke once before a drag.
bool pixmith_slice_set_bounds(PixmithDoc *doc, uint32_t i, uint32_t frame, int32_t x, int32_t y, uint32_t w, uint32_t h);
// center: 4 ints (x, y, w, h relative to the slice) or NULL; pivot: 2 ints or NULL.
bool pixmith_slice_set_props(PixmithDoc *doc, uint32_t i, uint32_t frame, const char *name,
                          const int32_t *center, const int32_t *pivot);
bool pixmith_slice_remove(PixmithDoc *doc, uint32_t i);

uint32_t pixmith_ud_tag_count(const PixmithDoc *doc);
char *pixmith_ud_tag_name(const PixmithDoc *doc, uint32_t i);

// User data targets: kind 0 sprite, 1 layer (a), 2 cel (a = frame, b = layer), 3 tag (a), 4 slice (a).
char *pixmith_userdata_text(const PixmithDoc *doc, uint32_t kind, uint32_t a, uint32_t b);
bool pixmith_userdata_color(const PixmithDoc *doc, uint32_t kind, uint32_t a, uint32_t b, uint32_t *out);
bool pixmith_userdata_set(PixmithDoc *doc, uint32_t kind, uint32_t a, uint32_t b, const char *text,
                       bool has_color, uint32_t rgba);
