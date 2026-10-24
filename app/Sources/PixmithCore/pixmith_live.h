// Live mode support (core/src/ffi/live.rs): a document as bytes and back, for the Pixmith Live protocol
// (docs/LIVE_PROTOCOL.md), which lets a local client such as the MCP server edit the documents open in the app.

#pragma once
#include "pixmith.h"
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// The document as .pxmt bytes (sprite, thumbnail, metadata). *len receives the size. NULL on failure (see
// pixmith_last_error). Free with pixmith_bytes_free.
uint8_t *pixmith_encode_document(const PixmithDoc *doc, size_t *len);

// Frees bytes returned by pixmith_encode_document.
void pixmith_bytes_free(uint8_t *bytes, size_t len);

// Replaces the document's sprite with `bytes` (.pxmt or .aseprite) as ONE undo step named `label`. The selection
// is kept unless the canvas size changes. False (document unchanged) if the bytes can't be decoded or validated.
bool pixmith_apply_document(PixmithDoc *doc, const uint8_t *bytes, size_t len, const char *label);

// The selection's bounding rectangle (*x, *y, *w, *h) and its mask: one byte per pixel inside the rectangle, row-major,
// 1 selected. *len = w * h. NULL when nothing is selected. Free with pixmith_bytes_free.
uint8_t *pixmith_live_selection(const PixmithDoc *doc, int32_t *x, int32_t *y, uint32_t *w, uint32_t *h, size_t *len);

// pixmith_apply_document that also handles the selection in the same undo step. mode: 0 keep it (cleared when the
// canvas size changes), 1 clear it, 2 replace it with the region x, y, w, h and `mask` (w * h bytes, nonzero = selected,
// in the coordinates of the new document).
bool pixmith_apply_document_selection(PixmithDoc *doc, const uint8_t *bytes, size_t len, const char *label,
                                      uint32_t mode, int32_t x, int32_t y, uint32_t w, uint32_t h,
                                      const uint8_t *mask, size_t mask_len);

// Identifies the state the document is in. It differs after every edit, undo, redo and pixmith_touch, so a front end
// can tell that a document changed (the Live revision is bumped when it does). 0 for NULL.
uint64_t pixmith_state_id(const PixmithDoc *doc);
