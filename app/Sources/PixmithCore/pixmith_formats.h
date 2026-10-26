// Image formats beyond PNG and GIF: WebP, JPEG, BMP, TGA, ICO, PCX and FLI/FLC
// (core/src/formats/raster/*.rs). Shared by the macOS app, the egui app (same Rust functions) and pixmith-cli.
//
// Import: the file's format is detected from its content, so one function opens every supported file. PNG and GIF go
// through the same entry points. Animated files (WebP, GIF, FLI/FLC) get one frame per image with its duration; the
// other formats give a one-frame sprite. The sprite is a new document without a file name.
//
// Export: there are two entry points.
//   - pixmith_export_image: one frame (or the animation) of a document straight to a file. The format comes from the
//     options ("format") or the file extension.
//   - pixmith_export_run_ex: the unified export of pixmith_io.h (tags, layers, trim, scale, one file per layer or tag)
//     with these formats added to PixmithExport.format:
//         5 WebP   6 JPEG   7 BMP   8 TGA   9 ICO   10 PCX   11 FLC (FLI is read, FLC is written)
//     WebP and FLC write one animation per job; the others write PixmithExport.frame (counted within the export).
//
// Options are a JSON object (null or "" for the defaults); unknown keys are ignored:
//   "format":     "webp" | "jpg" | "bmp" | "tga" | "ico" | "pcx" | "flc"   (pixmith_export_image only)
//   "quality":    1-100  JPEG quality (default 90) and WebP quality when lossy (default 90)
//   "jpeg_quality", "webp_quality": set one of the two
//   "lossless":   WebP, true (default) or false
//   "animated":   WebP, true (default): all frames as an animation; false: only one frame
//   "background": "#rrggbb" or [r, g, b]. Transparent pixels are flattened onto it by the formats without alpha: JPEG,
//                 PCX, FLC, and BMP/TGA when written with 8 or 24 bits (default white)
//   "bits":       BMP/TGA 8, 24 or 32; PCX 8 or 24. 0 or absent chooses (alpha: 32, up to 256 colors: 8, else 24)
//   "rle":        TGA run-length encoding (default true)
//   "ico_sizes":  [16, 32, 48, 256] up to 8 sizes from 1 to 256. The sprite is scaled by a whole number to fit each
//                 square and centered (reduced if larger). Without sizes the icon has the sprite's own size (<= 256).
// Import options (pixmith_import_image_ex): {"ico_size": 32} opens that size of an icon (default: the largest).
//   {"mode": "trace_grid", ...} opens the first image as pixel art traced onto its grid; the options are those of
//   pixmith_trace_grid (pixmith_trace.h): pitch, colors, remove_background, shadow, close_outline, ...
#pragma once
#include "pixmith.h"
#include "pixmith_io.h"

// Opens any supported image file as a new sprite; null on failure (see pixmith_last_error).
PixmithDoc *pixmith_import_image(const char *path);
PixmithDoc *pixmith_import_image_ex(const char *path, const char *options_json);

// Describes an image file as JSON: {"format":"ico","name":"ICO","animated":false,"sizes":[16,32,256]}.
// "sizes" is present for icons only (the larger side of each image, ascending). Free the result with
// pixmith_string_free; null if the file cannot be read or is not an image.
char *pixmith_image_info(const char *path);

// Writes `frame` (0-based) of the document at `scale` (1-64) as an image file; PIXMITH_ALL_FRAMES writes the whole
// animation in the animated formats (WebP, FLC) and the first frame in the others.
#define PIXMITH_ALL_FRAMES 0xFFFFFFFFu
bool pixmith_export_image(const PixmithDoc *doc, uint32_t frame, uint32_t scale, const char *path,
                          const char *options_json);

// pixmith_export_run with the image format options as JSON. Returns the number of files written, or -1.
int32_t pixmith_export_run_ex(const PixmithDoc *doc, const PixmithExport *options, const char *image_options_json,
                              const char *path);
