// Live mode support (core/src/ffi/live.rs): a document as bytes and back, for the Pixmith Live protocol
// (docs/LIVE_PROTOCOL.md), which lets a local client such as the MCP server edit the documents open in the app.

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
