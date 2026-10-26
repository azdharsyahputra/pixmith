// Background removal and trace-to-pixel-grid for "pseudo-pixel" art from AI image tools: soft upscaled edges,
// drifting grid, noise, a white paper background and a drop shadow (core/src/paint/background.rs, trace.rs).
// Options are a JSON object; every key is optional and unknown keys are ignored. Both functions return their report as
// JSON (free it with pixmith_string_free), or null on failure (pixmith_last_error); on failure the document is unchanged.
#pragma once
#include "pixmith.h"

// Makes the background of the cel at (layer, frame) transparent, inside the selection, as one undo step.
// The background colour is the dominant border colour. It is removed from the edges inwards, so equal colours enclosed
// by the figure stay.
//   options: {"tolerance": 28,         largest per-channel difference to the background (0...255)
//             "global": false,         also remove background-coloured pixels enclosed by the figure
//             "shadow": false,         also remove a soft shadow connected to the removed background
//             "shadow_depth": 55,      how much darker than the background a shadow may be (percent, 1...95)
//             "color": "#ffffff"}      use this background colour instead of detecting it
//   report:  {"color": "#ffffff" or null, "background": n, "global": n, "shadow": n, "shadow_skipped": false,
//             "removed": n}          (pixel counts)
// The same operation is the filter PIXMITH_FILTER_REMOVE_BACKGROUND of pixmith_filter_apply / pixmith_filter_preview.
char *pixmith_remove_background(PixmithDoc *doc, uint32_t layer, uint32_t frame, const char *options_json);

// Traces the document onto a real pixel grid as one undo step. The cel at (layer, frame) defines the grid, the palette
// and the crop; every image cel is resampled with them and the canvas becomes the traced size. RGB sprites without
// tilemap layers only.
//   options: {"pitch": "auto" | 7.3 | [7.3, 7.6],  cell size in source pixels (default "auto")
//             "min_pitch": 3, "max_pitch": 32,     range for "auto"
//             "phase": [x, y],                     offset of the first cell edge (default: detected)
//             "separate_axes": false,              find the x and y pitch independently
//             "colors": 16 | "auto" | "keep" | ["#rrggbb", ...],   k-means palette size / auto (min_colors..max_colors,
//                                                  default 2..32) / keep cell colours / snap to these colors
//             "remove_background": false,          plus the keys of pixmith_remove_background
//             "close_outline": false,              close one-pixel breaks in the dark outer outline
//             "crop": true,                        crop to the opaque cells
//             "seed": 1}                           k-means seed (same seed, same palette)
//   report:  {"width", "height", "pitch": [x, y], "phase": [x, y], "palette": ["#rrggbb", ...], "quality": 0...1,
//             "origin": [x, y], "grid_cells": [w, h], "closed_outline": n, "background": {...},
//             "warning": "..." (only when the grid is weak: check the quality or pass pitch)}
char *pixmith_trace_grid(PixmithDoc *doc, uint32_t layer, uint32_t frame, const char *options_json);
