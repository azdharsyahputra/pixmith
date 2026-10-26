// Filters / FX (core/src/filters.rs). Colors are 0xRRGGBBAA passed as int32 bit patterns.
#pragma once
#include "pixmith.h"

// Filter kinds and their integer parameters (`params`):
enum {
    PIXMITH_FILTER_REPLACE_COLOR = 0,       // from color, to color, tolerance 0...255
    PIXMITH_FILTER_INVERT = 1,              // none
    PIXMITH_FILTER_DESATURATE = 2,          // none
    PIXMITH_FILTER_HSL = 3,                 // hue -180...180 (degrees), saturation -100...100, lightness -100...100
    PIXMITH_FILTER_BRIGHTNESS_CONTRAST = 4, // brightness -100...100, contrast -100...100
    PIXMITH_FILTER_CHANNEL_OFFSET = 5,      // red, green, blue offsets -255...255
    PIXMITH_FILTER_OUTLINE = 6,             // color, inside (0 outside / 1 inside), eight-connected (0/1)
    PIXMITH_FILTER_DROP_SHADOW = 7,         // dx, dy, color
    PIXMITH_FILTER_DESPECKLE = 8,           // eight-connected (0/1)
    PIXMITH_FILTER_BLUR = 9,                // none (3x3, alpha-aware)
    PIXMITH_FILTER_SHARPEN = 10,            // none (3x3, alpha-aware)
    PIXMITH_FILTER_REMOVE_BACKGROUND = 13,  // tolerance 0...255, flags (1 shadow, 2 global), [shadow depth 1...95 percent], [background 0xRRGGBBAA, alpha 0 = detect]; see pixmith_trace.h
    PIXMITH_FILTER_CURVES = 11,             // five curves (RGB master, R, G, B, A), each: point count, then x, y pairs (0...255)
    PIXMITH_FILTER_CLEAN_EDGES = 12,        // all optional, in this order: pass flags (1 halo, 2 despeckle, 4 thin outline,
                                            // 8 lone light pixels; default 7), halo rings (2), halo luma gain (40),
                                            // halo max saturation % (30), despeckle min size (4), hole size (0 = keep holes),
                                            // outline max thickness (3), outline max luma (70), fixed outline color on (0/1),
                                            // its color, its tolerance
};

// Targets: 0 = the cel at (layer, frame), 1 = every frame of `layer`, 2 = every layer of `frame`.
// The rectangle (x, y, w, h) limits where pixels are written; w == 0 means the whole canvas.

// Applies a filter as a single undo step. Returns false (and records nothing) if no pixel changed.
bool pixmith_filter_apply(PixmithDoc *doc, uint32_t kind, uint32_t target, uint32_t layer, uint32_t frame,
                       int32_t x, int32_t y, uint32_t w, uint32_t h,
                       const int32_t *params, size_t param_count);

// Renders `frame` as it would look with the filter applied, leaving the document and undo history
// untouched. `out` needs width * height * 4 bytes.
bool pixmith_filter_preview(const PixmithDoc *doc, uint32_t kind, uint32_t target, uint32_t layer,
                         uint32_t frame, int32_t x, int32_t y, uint32_t w, uint32_t h,
                         const int32_t *params, size_t param_count, uint8_t *out, size_t len);

// Nearest-neighbor resize of the whole sprite (all frames and layers) as a single undo step.
bool pixmith_scale_sprite(PixmithDoc *doc, uint32_t width, uint32_t height);

// Writes the 256-entry lookup table of one curve (`n` control points as x, y pairs) to `out`.
bool pixmith_curve_lut(const int32_t *xy, size_t n, uint8_t *out);

// Sprite Lint (read only). Reports problems of the cels chosen by `target` (as above) without changing the
// document. `params` (all optional, in this order): speck size (4), positions listed per category (64), check
// palette (0/1), outline max luma (70), fixed outline color on (0/1), its color, its tolerance, stray tolerance (48),
// category bit mask (all; bit 0 light_edge, then holes, specks, no_outline, strays, semi_transparent, off_palette).
// Returns NULL on a bad argument; otherwise text to release with pixmith_string_free: a line "cel <layer> <frame>"
// (0-based) starts a cel; a line "<category> <count>" starts a category (light_edge, holes, specks, no_outline,
// strays, semi_transparent, off_palette); the lines below it are "x y size".
char *pixmith_sprite_lint(const PixmithDoc *doc, uint32_t target, uint32_t layer, uint32_t frame,
                          const int32_t *params, size_t param_count);
