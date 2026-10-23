// Saved-state tracking (core/src/history.rs).
#pragma once
#include "pixmith.h"

// True when the document differs from the last saved (or opened) state.
bool pixmith_is_dirty(const PixmithDoc *doc);
void pixmith_mark_saved(PixmithDoc *doc);
// Call after every edit that did not start with pixmith_begin_stroke / a checkpoint.
void pixmith_touch(PixmithDoc *doc);

// Transactions: all undo checkpoints inside collapse into one undo step. They nest; every begin needs an end.
void pixmith_begin_transaction(PixmithDoc *doc);
void pixmith_end_transaction(PixmithDoc *doc);

// Names the most recent undo step (only if it is still unnamed). Call right after the operation.
void pixmith_checkpoint_label(PixmithDoc *doc, const char *label);

// Undo history: applied steps (oldest first), then undone steps (the next one to redo first).
void pixmith_history_counts(const PixmithDoc *doc, uint32_t *undo, uint32_t *redo);
// Label of step `index` ("Edit" when unnamed); free with pixmith_string_free. NULL if out of range.
char *pixmith_history_label(const PixmithDoc *doc, uint32_t index);
// Undoes or redoes until exactly `position` steps are applied.
bool pixmith_history_goto(PixmithDoc *doc, uint32_t position);

// Memory budget for undo history beyond the live document (default 1 GiB). Older steps are dropped to fit.
void pixmith_set_undo_budget(PixmithDoc *doc, uint64_t bytes);
// Bytes only the undo/redo history keeps alive.
uint64_t pixmith_undo_bytes(const PixmithDoc *doc);
