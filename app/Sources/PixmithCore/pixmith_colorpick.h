// Color-picker math (core/src/colorpick.rs). Colors are 0xRRGGBBAA; hue is in degrees (0..360), every other
// component in 0...1. Pure functions: no document, safe to call from any thread.
//
// Disc coordinates: the wheel and the normal disc are drawn in a square of side 2 centred on the origin; x grows to
// the right and y grows DOWN (screen orientation). On the color wheel hue 0 (red) is east and hue grows
// counter-clockwise; the radius is the saturation. On the normal disc the centre is the flat normal (0,0,1) = #8080FF
// and the rim is the horizon (z = 0).
#pragma once
#include "pixmith.h"

// RGB <-> HSV / HSL. `hint` is NULL or [hue, saturation] used where a component is undefined (gray hue, black
// saturation) so a drag does not jump. out3 receives [h, s, v] / [h, s, l].
void pixmith_cp_rgba_to_hsv(uint32_t rgba, const double *hint, double *out3);
uint32_t pixmith_cp_hsv_to_rgba(double h, double s, double v, double a);
void pixmith_cp_rgba_to_hsl(uint32_t rgba, const double *hint, double *out3);
uint32_t pixmith_cp_hsl_to_rgba(double h, double s, double l, double a);
// Rec. 601 luma, 0...1 (the Gray selector's value).
double pixmith_cp_luma(uint32_t rgba);

// Wheel geometry. hit -> out2 = [hue, saturation] (saturation clamps to 1 beyond the rim; hint_hue is returned at
// the exact centre). point -> out2 = [x, y].
void pixmith_cp_wheel_hit(double x, double y, double hint_hue, double *out2);
void pixmith_cp_wheel_point(double hue, double sat, double *out2);
// Harmony hues around `hue` (not including it). harmony: 0 none, 1 complementary, 2 triadic, 3 analogous,
// 4 split complementary, 5 square. `out` has room for 4; returns the count.
uint32_t pixmith_cp_harmony(double hue, uint32_t harmony, double *out);
// Renders the wheel into `out` (size*size*4 bytes, straight-alpha RGBA; the disc touches the edges and is
// anti-aliased). model: 0 = HSV (radius = saturation, value), 1 = HSL (lightness). False for a NULL buffer or a
// size of 0 or above 4096.
bool pixmith_cp_wheel_render(uint8_t *out, uint32_t size, uint32_t model, double value);

// Normal-map disc (tangent space, X right, Y up, Z out of the surface; stored as (n * 0.5 + 0.5) per channel).
bool pixmith_cp_normal_render(uint8_t *out, uint32_t size);
void pixmith_cp_normal_from_disc(double x, double y, double *out3);
void pixmith_cp_normal_disc_point(uint32_t rgba, double *out2);
// The normal stored in a color (normalized, lifted onto the upper hemisphere) and the color storing a normal.
void pixmith_cp_normal_decode(uint32_t rgba, double *out3);
uint32_t pixmith_cp_normal_encode(const double *n3, double a);
// Replaces X (axis 0), Y (axis 1) or Z (axis 2) of the normal in n3 and keeps it a valid unit normal.
void pixmith_cp_normal_set(double *n3, uint32_t axis, double v);
