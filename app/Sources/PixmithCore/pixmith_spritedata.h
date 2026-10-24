// Color profile, cel z-index and reference layers (core/src/document/spritedata.rs).
#pragma once
#include "pixmith.h"

// Color profile of the sprite (.aseprite chunk 0x2007). Pixels are never converted.
// kind: 0 none, 1 sRGB, 2 embedded ICC; -1 for an invalid document.
int32_t pixmith_color_profile_kind(const PixmithDoc *doc);
// "None", "sRGB" or "Embedded ICC (name)". Free with pixmith_string_free.
char *pixmith_color_profile_label(const PixmithDoc *doc);
uint32_t pixmith_color_profile_icc_size(const PixmithDoc *doc);
// 0 removes the profile, 1 assigns sRGB (one undo step). An ICC profile can only be kept, not assigned.
bool pixmith_color_profile_set(PixmithDoc *doc, uint32_t kind);

// Cel z-index: the cel draws as if its layer sat z places higher (lower when negative); order is
// layer index + z-index, ties by layer index. False when the cel is empty.
bool pixmith_cel_z_index(const PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t *out);
bool pixmith_cel_set_z_index(PixmithDoc *doc, uint32_t layer, uint32_t frame, int32_t z);

// Reference layers (layer flag 0x40): drawn in the editor, never in exports or sprite sheets. A new one is locked
// (drawing is refused until unlocked).
bool pixmith_layer_is_reference(const PixmithDoc *doc, uint32_t layer);
bool pixmith_layer_set_reference(PixmithDoc *doc, uint32_t layer, bool on);
bool pixmith_layer_is_locked(const PixmithDoc *doc, uint32_t layer);
bool pixmith_layer_set_locked(PixmithDoc *doc, uint32_t layer, bool locked);
// Imports an image scaled to fit the canvas into a new locked reference layer above `above`. Returns its index or -1.
int32_t pixmith_layer_new_reference(PixmithDoc *doc, uint32_t above, const char *path);
// The same for an image the caller decoded: w * h * 4 bytes of straight-alpha RGBA.
int32_t pixmith_layer_new_reference_rgba(PixmithDoc *doc, uint32_t above, const char *name, const uint8_t *rgba,
                                         uint32_t w, uint32_t h);
// How opaque the editor draws reference layers, in percent (0 hides them; default 100). pixmith_render and
// pixmith_render_rect follow it, exports ignore it.
uint32_t pixmith_reference_display(const PixmithDoc *doc);
void pixmith_set_reference_display(PixmithDoc *doc, uint32_t percent);
