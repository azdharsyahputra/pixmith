// Palette editing and presets (core/src/palette_edit.rs, palette_presets.rs). Colors are 0xRRGGBBAA.
#pragma once
#include "pixmith.h"

// Moves entries to just before `dest` (dest == palette count appends). One undo step.
// Returns the new index of the first moved entry, or -1 if nothing moved.
int32_t pixmith_pe_move(PixmithDoc *doc, const uint32_t *indices, uint32_t count, uint32_t dest);
bool pixmith_pe_insert(PixmithDoc *doc, uint32_t at, uint32_t rgba);
// Inserts a copy of each entry right after it. Returns how many were added.
uint32_t pixmith_pe_duplicate(PixmithDoc *doc, const uint32_t *indices, uint32_t count);
bool pixmith_pe_remove(PixmithDoc *doc, const uint32_t *indices, uint32_t count);
// Live edit of one entry; begin = true on the first call of an edit session (one undo step).
// Returns whether the entry changed.
bool pixmith_pe_set_live(PixmithDoc *doc, uint32_t index, uint32_t rgba, bool begin);
// Inserts n colors between entries `from` and `to` after the later one (HSV or RGB).
// Returns the index of the first new color, or -1.
int32_t pixmith_pe_ramp(PixmithDoc *doc, uint32_t from, uint32_t to, uint32_t n, bool hsv);

uint32_t pixmith_pe_preset_count(void);
char *pixmith_pe_preset_name(uint32_t index); // free with pixmith_string_free
bool pixmith_pe_preset_load(PixmithDoc *doc, uint32_t index);
