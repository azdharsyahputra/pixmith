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
