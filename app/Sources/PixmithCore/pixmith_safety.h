// Crash safety (core/src/lib.rs): panics inside the core never abort the app.
#pragma once
#include "pixmith.h"

// Every entry point catches panics: it returns false/0/NULL, sets pixmith_last_error() to "internal error: ..."
// and rewinds the document it was working on to its last undo step (the undone work stays on the redo stack).
// The count only grows; when it changes after a call, tell the user and offer to save a copy.
uint32_t pixmith_panic_count(void);
// Description of the most recent panic (empty if none). Valid until the next call on the same thread.
const char *pixmith_last_panic(void);
