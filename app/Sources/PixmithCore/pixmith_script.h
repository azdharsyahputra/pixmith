// Embedded Lua scripting (core/src/script.rs).
#pragma once
#include "pixmith.h"

// Runs a script as one undo step. Alerts and print output are returned newline separated in
// *out_messages (free with pixmith_string_free; may be NULL on failure). On failure the document is
// unchanged and pixmith_last_error() holds the message. `layer` and `frame` (0-based) are the
// defaults scripts use for pixel calls.
bool pixmith_run_script(PixmithDoc *doc, const char *source, char **out_messages);
bool pixmith_run_script_at(PixmithDoc *doc, const char *source, uint32_t layer, uint32_t frame, char **out_messages);

// Like pixmith_run_script_at, with a script name (permission requests, the "always allow" key; NULL = "script") and
// the document's path (sprite.filename, may be NULL). Documents the script created are available afterwards from
// pixmith_script_outcome(0, 0) and pixmith_script_take_doc(0, 0, i) (pixmith_script_ui.h).
bool pixmith_run_script_named(PixmithDoc *doc, const char *source, uint32_t layer, uint32_t frame,
                              const char *script_name, const char *doc_path, char **out_messages);

// Scripts shipped with the app.
uint32_t pixmith_example_script_count(void);
char *pixmith_example_script_name(uint32_t index);
char *pixmith_example_script_source(uint32_t index);
