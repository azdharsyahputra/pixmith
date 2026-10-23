// C interface of the Rust core (core/src/lib.rs). Colors are 0xRRGGBBAA.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct PixmithDoc PixmithDoc;

PixmithDoc *pixmith_new(uint32_t width, uint32_t height);
PixmithDoc *pixmith_open(const char *path);
void pixmith_free(PixmithDoc *doc);
const char *pixmith_last_error(void);

bool pixmith_save(const PixmithDoc *doc, const char *path);
bool pixmith_export_png(const PixmithDoc *doc, uint32_t frame, uint32_t scale, const char *path);
bool pixmith_export_sheet(const PixmithDoc *doc, uint32_t scale, const char *path);

uint32_t pixmith_width(const PixmithDoc *doc);
uint32_t pixmith_height(const PixmithDoc *doc);
uint32_t pixmith_layer_count(const PixmithDoc *doc);
uint32_t pixmith_frame_count(const PixmithDoc *doc);

char *pixmith_layer_name(const PixmithDoc *doc, uint32_t layer);
void pixmith_string_free(char *s);
bool pixmith_layer_visible(const PixmithDoc *doc, uint32_t layer);
bool pixmith_layer_is_group(const PixmithDoc *doc, uint32_t layer);
uint32_t pixmith_layer_level(const PixmithDoc *doc, uint32_t layer);
uint8_t pixmith_layer_opacity(const PixmithDoc *doc, uint32_t layer);
void pixmith_set_layer_visible(PixmithDoc *doc, uint32_t layer, bool visible);
void pixmith_set_layer_opacity(PixmithDoc *doc, uint32_t layer, uint8_t opacity);
void pixmith_rename_layer(PixmithDoc *doc, uint32_t layer, const char *name);
uint32_t pixmith_add_layer(PixmithDoc *doc, uint32_t above);
bool pixmith_remove_layer(PixmithDoc *doc, uint32_t layer);

uint32_t pixmith_add_frame(PixmithDoc *doc, uint32_t after, bool duplicate);
bool pixmith_remove_frame(PixmithDoc *doc, uint32_t frame);
uint16_t pixmith_frame_duration(const PixmithDoc *doc, uint32_t frame);
void pixmith_set_frame_duration(PixmithDoc *doc, uint32_t frame, uint16_t ms);

bool pixmith_render(const PixmithDoc *doc, uint32_t frame, uint8_t *out, size_t len);
void pixmith_begin_stroke(PixmithDoc *doc);
void pixmith_draw_line(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x0, int32_t y0,
                    int32_t x1, int32_t y1, uint32_t rgba, uint32_t size);
void pixmith_fill(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t x, int32_t y, uint32_t rgba);
bool pixmith_undo(PixmithDoc *doc);
bool pixmith_redo(PixmithDoc *doc);

uint32_t pixmith_palette_count(const PixmithDoc *doc);
uint32_t pixmith_palette_color(const PixmithDoc *doc, uint32_t index);
void pixmith_palette_add(PixmithDoc *doc, uint32_t rgba);
