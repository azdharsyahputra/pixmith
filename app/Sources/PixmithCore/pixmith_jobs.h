// Background work on copies of a document (core/src/jobs.rs).
//
// A PixmithDoc must only be used by one thread at a time. To work off the main thread, take a snapshot
// on the main thread, hand the snapshot to the worker, and free it with pixmith_free when done.
#pragma once
#include "pixmith.h"

// Independent copy of the sprite and selection (no undo history). NULL for NULL.
PixmithDoc *pixmith_snapshot(PixmithDoc *doc);
// Applies what a worker did to a snapshot (e.g. a script run) to `doc` as one undo step named like
// the snapshot's own step. False, recording nothing, if the snapshot still equals the document.
bool pixmith_job_apply(PixmithDoc *doc, PixmithDoc *job);
// Asks the running script to stop (true), or clears the request before a new run (false).
void pixmith_script_cancel(bool cancel);
