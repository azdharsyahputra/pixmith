// Layers, blend modes, tags, frame moves and linked cels (core/src/layers.rs).
#pragma once
#include "pixmith.h"

// Layer operations return the layer's new index, or -1 if the operation does not apply.
int32_t pixmith_layer_move(PixmithDoc *doc, uint32_t layer, bool up);
int32_t pixmith_layer_duplicate(PixmithDoc *doc, uint32_t layer);
int32_t pixmith_layer_merge_down(PixmithDoc *doc, uint32_t layer);
bool pixmith_layer_can_merge_down(const PixmithDoc *doc, uint32_t layer);
int32_t pixmith_layer_add_group(PixmithDoc *doc, uint32_t above);
int32_t pixmith_layer_group(PixmithDoc *doc, uint32_t layer);
int32_t pixmith_layer_into_group(PixmithDoc *doc, uint32_t layer, uint32_t group);
int32_t pixmith_layer_out_of_group(PixmithDoc *doc, uint32_t layer);
uint16_t pixmith_layer_blend_mode(const PixmithDoc *doc, uint32_t layer);
bool pixmith_layer_set_props(PixmithDoc *doc, uint32_t layer, const char *name, uint8_t opacity,
                          uint16_t blend_mode, bool visible);

bool pixmith_frame_move(PixmithDoc *doc, uint32_t from, uint32_t to);
int32_t pixmith_cel_opacity(const PixmithDoc *doc, uint32_t layer, uint32_t frame);
bool pixmith_cel_set_opacity(PixmithDoc *doc, uint32_t layer, uint32_t frame, uint8_t opacity);
bool pixmith_cel_is_linked(const PixmithDoc *doc, uint32_t layer, uint32_t frame);
bool pixmith_cel_link(PixmithDoc *doc, uint32_t layer, uint32_t from, uint32_t to);
bool pixmith_cel_unlink(PixmithDoc *doc, uint32_t layer, uint32_t frame);

// Tag color is 0xRRGGBB; direction is 0 forward, 1 reverse, 2 ping-pong, 3 ping-pong reverse.
typedef struct {
    uint32_t from;
    uint32_t to;
    uint8_t direction;
    uint16_t repeat;
    uint32_t color;
} PixmithTag;

uint32_t pixmith_tag_count(const PixmithDoc *doc);
bool pixmith_tag_get(const PixmithDoc *doc, uint32_t index, PixmithTag *out);
char *pixmith_tag_name(const PixmithDoc *doc, uint32_t index);
int32_t pixmith_tag_add(PixmithDoc *doc, const char *name, PixmithTag info);
bool pixmith_tag_update(PixmithDoc *doc, uint32_t index, const char *name, PixmithTag info);
bool pixmith_tag_remove(PixmithDoc *doc, uint32_t index);

// Playback state carried between pixmith_play_next calls; start from pixmith_play_start.
typedef struct {
    bool reversed;
    uint32_t passes;
    int32_t tag;
} PixmithPlayState;

void pixmith_play_start(PixmithPlayState *state);
uint32_t pixmith_play_next(const PixmithDoc *doc, uint32_t frame, PixmithPlayState *state, bool within_tag);

// Timeline cel grid. pixmith_cel_grid fills layers * frames bytes (index layer * frames + frame):
// bit 0 has a cel, bit 1 has visible pixels, bit 2 is linked. Rectangles use inclusive bounds.
bool pixmith_cel_grid(const PixmithDoc *doc, uint8_t *out, size_t len);
bool pixmith_cels_clear(PixmithDoc *doc, uint32_t l0, uint32_t l1, uint32_t f0, uint32_t f1);
bool pixmith_cels_copy(const PixmithDoc *doc, uint32_t l0, uint32_t l1, uint32_t f0, uint32_t f1);
bool pixmith_cels_has_clip(void);
bool pixmith_cels_paste(PixmithDoc *doc, uint32_t layer, uint32_t frame);
bool pixmith_cels_move(PixmithDoc *doc, uint32_t l0, uint32_t l1, uint32_t f0, uint32_t f1,
                    int32_t dl, int32_t df, bool copy);
