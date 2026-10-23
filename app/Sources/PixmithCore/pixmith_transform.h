// Free transform API (core/src/transform.rs). Coordinates are continuous canvas coordinates.
#pragma once
#include "pixmith.h"

bool pixmith_xf_begin(PixmithDoc *doc, uint32_t layer, uint32_t frame, bool rotsprite);
bool pixmith_xf_active(const PixmithDoc *doc);
void pixmith_xf_commit(PixmithDoc *doc);
bool pixmith_xf_cancel(PixmithDoc *doc);
void pixmith_xf_set_rotsprite(PixmithDoc *doc, bool on);
// Box point (handle direction -1..1; 0, 0 is the center) that typed width/height keep fixed.
void pixmith_xf_set_anchor(PixmithDoc *doc, int32_t hx, int32_t hy);
// 18 doubles: 8 handle positions (TL, T, TR, L, R, BL, B, BR) as x, y pairs, then the pivot.
bool pixmith_xf_geometry(const PixmithDoc *doc, double *out18);
// 7 doubles: x, y, width, height, angle, skew x, skew y (angles in degrees).
bool pixmith_xf_fields(const PixmithDoc *doc, double *out7);
void pixmith_xf_set_field(PixmithDoc *doc, uint32_t field, double value);
void pixmith_xf_drag_begin(PixmithDoc *doc);
// kind: 0 move, 1 scale (flag keeps aspect), 2 rotate (flag snaps to 15 degrees), 3 skew, 4 pivot.
void pixmith_xf_drag(PixmithDoc *doc, uint32_t kind, int32_t hx, int32_t hy, bool flag,
                  double sx, double sy, double px, double py);
