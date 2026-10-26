// Import/export additions to the C interface (core/src/{import,gif_export,sheet,sheet_import,animation}.rs).
#pragma once
#include "pixmith.h"

PixmithDoc *pixmith_import_png(const char *path);
PixmithDoc *pixmith_import_sheet(const char *path, uint32_t cell_w, uint32_t cell_h);
// Returns the new layer's index, or -1 on failure (see pixmith_last_error).
int32_t pixmith_import_png_layer(PixmithDoc *doc, uint32_t above, uint32_t frame, const char *path);

bool pixmith_export_gif(const PixmithDoc *doc, uint32_t scale, const char *path);
// `base` has no extension; writes <base>-001.png, <base>-002.png, ...
bool pixmith_export_png_sequence(const PixmithDoc *doc, uint32_t scale, const char *base);
// layout: 0 horizontal, 1 vertical, 2 grid with `columns` columns.
bool pixmith_export_sheet_ex(const PixmithDoc *doc, uint32_t layout, uint32_t columns, uint32_t padding,
                          uint32_t scale, bool write_json, const char *path);

// Opens an animated GIF / an Aseprite JSON sprite sheet as a new sprite (null on failure).
// `png_path` may be null: the image named in the JSON's meta.image (next to the JSON) is used.
PixmithDoc *pixmith_import_gif(const char *path);
PixmithDoc *pixmith_import_sheet_json(const char *json_path, const char *png_path);
bool pixmith_export_apng(const PixmithDoc *doc, uint32_t scale, const char *path);

// Unified export (core/src/export_plan.rs). Enumerations:
//   format: 0 PNG (one frame), 1 GIF, 2 APNG, 3 sprite sheet, 4 PNG sequence (path is a base name without extension)
//   layers: 0 visible layers, 1 only `layer` (shown even if hidden), 2 each visible layer in its own file
//   trim: 0 none, 1 per frame (animations use one rectangle for all frames), 2 one rectangle for the whole animation
//   layout: 0 horizontal, 1 vertical, 2 grid with `columns` columns
// `tag` is an index (or -1 for all frames); with `apply_direction` the tag's direction (reverse, ping-pong) is
// expanded into the frame order. `split_tags` writes one file per tag, `layers == 2` one per layer; names get
// "-<layer>-<tag>" appended before the extension.
typedef struct PixmithExport {
    uint32_t format;
    uint32_t scale;
    int32_t tag;
    bool apply_direction;
    uint32_t layers;
    uint32_t layer;
    bool split_tags;
    uint32_t trim;
    bool ignore_empty;
    uint32_t frame;
    uint32_t layout;
    uint32_t columns;
    uint32_t padding;
    uint32_t border;
    uint32_t extrude;
    bool write_json;
} PixmithExport;

// Writes the export; returns the number of files written, or -1 (see pixmith_last_error).
int32_t pixmith_export_run(const PixmithDoc *doc, const PixmithExport *options, const char *path);
// Warnings of the last pixmith_export_run / pixmith_export_run_ex / pixmith_export_gif on the calling thread, one per
// line ("" when there are none). A warning does not fail the export (the files are written); today it is a GIF over
// 50 MB. The string stays valid until the next export on this thread; do not free it.
const char *pixmith_export_warnings(void);
// Number of output files (sheets/animations/images; a PNG sequence counts once per job) or -1 if invalid.
int32_t pixmith_export_job_count(const PixmithDoc *doc, const PixmithExport *options);
