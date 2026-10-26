// Lua dialogs, events and plugins (core/src/scripting/host.rs). The contract is docs/LUA_DIALOG.md.
//
// A script or plugin is a "session": a Lua state on its own core thread. The front end polls JSON messages
// (dialog.open, dialog.modify, dialog.close, state, txn, command.*, session.end ...) and sends JSON events back.
// Every function may be called from any thread except pixmith_script_apply, which belongs to the thread that
// owns the PixmithDoc. Strings returned by the core are released with pixmith_string_free.
#pragma once
#include <stdbool.h>
#include <stdint.h>
#include "pixmith.h"

// Starts a script session. `doc` (may be NULL) is snapshotted for the main chunk; `doc_tag` is any number the
// front end uses to find the document again, echoed in `txn` messages. `layer`/`frame` are 0-based. `name` is a
// display name. Returns the session id, 0 on failure (see pixmith_last_error; at most 64 sessions at once).
uint64_t pixmith_script_start(PixmithDoc *doc, uint64_t doc_tag, const char *source, const char *name,
                              uint32_t layer, uint32_t frame);

// Loads a plugin: runs `source`, then calls its global init(plugin). `dir` is plugin.path (may be NULL),
// `prefs_path` the JSON file behind plugin.preferences (may be NULL: not persisted). Returns the session id or 0.
uint64_t pixmith_plugin_load(const char *source, const char *name, const char *dir, const char *prefs_path);

// Queued messages of ALL sessions as a JSON array ("[]" when none), oldest first. Waits up to timeout_ms for the
// first message (0 = do not wait). Free with pixmith_string_free.
char *pixmith_script_poll(uint32_t timeout_ms);

// Delivers one event (docs/LUA_DIALOG.md 2.2) to a session. `doc` (may be NULL) is snapshotted when the event
// starts a transaction. False if the session does not exist or the JSON is invalid.
bool pixmith_script_send(uint64_t session, PixmithDoc *doc, uint64_t doc_tag, uint32_t layer, uint32_t frame,
                         const char *event_json);

// Delivers an app.event / sprite.event to every session that listens for it. Returns how many received it.
uint32_t pixmith_script_broadcast(PixmithDoc *doc, uint64_t doc_tag, uint32_t layer, uint32_t frame,
                                  const char *event_json);

// Applies the result of transaction `txn` (a `txn` message with commit=true) to `doc` as one undo step.
// 1 applied, 0 nothing to apply, -1 conflict (the document changed meanwhile; result discarded),
// -2 unknown session or transaction. `doc == NULL` discards the result (returns 0).
int32_t pixmith_script_apply(uint64_t session, uint64_t txn, PixmithDoc *doc);

// The drawing colors scripts read as app.fgColor / app.bgColor (0xRRGGBBAA). Call whenever they change.
void pixmith_script_set_colors(uint32_t fg, uint32_t bg);

// Stops a session: interrupts Lua, runs a plugin's exit(plugin), saves its preferences, queues command.remove,
// dialog.close and session.end. Returns at once.
void pixmith_script_stop(uint64_t session);

// Number of live sessions.
uint32_t pixmith_script_count(void);

// File and command permissions, and documents created by scripts: docs/LUA_HOST_API.md.

// access: 1 = read, 2 = write, 4 = execute. Returns 0 deny, 1 allow once, 2 always for this script.
// Called on the script's thread; may block. `script` and `path` are valid only during the call.
typedef uint32_t (*pixmith_script_permission_fn)(void *ctx, const char *script, const char *path, uint32_t access);

// Installs (or, with fn == NULL, removes) the handler. Without one every file or command request is denied.
void pixmith_script_set_permission_handler(pixmith_script_permission_fn fn, void *ctx);

// Forgets every "always allow" answer given for any script.
void pixmith_script_reset_grants(void);

// Replaces the process-wide pre-allowed prefixes (read and write). count may be 0.
void pixmith_script_set_allowed_paths(const char *const *paths, uint32_t count);

// Registers the file of the document the front end calls `doc_tag` (sprite.filename). path == NULL forgets it.
void pixmith_script_set_doc_path(uint64_t doc_tag, const char *path);

// Ownership of a new document that a script created or opened passes to the caller (release with pixmith_free, or
// keep it as a tab). NULL if unknown, already taken or dropped. session = 0, txn = 0 selects the last synchronous
// run on this thread.
PixmithDoc *pixmith_script_take_doc(uint64_t session, uint64_t txn, uint32_t index);

// JSON of the documents of the last synchronous run on this thread (session = 0, txn = 0), "{}" when it used none.
// Free with pixmith_string_free.
char *pixmith_script_outcome(uint64_t session, uint64_t txn);
