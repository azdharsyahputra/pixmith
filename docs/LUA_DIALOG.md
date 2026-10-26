# Lua Dialog, events and plugins: the contract

Status: **v1** (contract for the core, the egui front end and the macOS renderer). Anything marked
*(not in v1)* is deliberately left out; do not render or send it.

Pixmith's Lua follows Aseprite 1.3 (`Dialog`, `app.events`, `sprite.events`, plugin `init(plugin)` /
`exit(plugin)`), inside Pixmith's sandbox (`docs/LUA_API.md`: no `os`, `io`, `require`; instruction,
time and memory limits). This file defines

1. the Lua API a script sees,
2. the JSON messages that cross the core to front-end boundary,
3. the threading and transaction model,
4. the C ABI (`app/Sources/PixmithCore/pixmith_script_ui.h`),
5. what a front end has to do.

A front end never runs Lua. It renders dialogs the core describes and sends back what the user did.

---

## 1. Lua API

### 1.1 Dialog

```lua
local dlg = Dialog("Palette ramp")            -- or Dialog{ title = "...", onclose = fn, notitlebar = false }
dlg:label  { id = "info",  text = "Pick two colors" }
   :color  { id = "from",  label = "From", color = 0x1D2B53FF }
   :color  { id = "to",    label = "To",   color = 0xFFA300FF }
   :newrow()
   :slider { id = "steps", label = "Steps", min = 2, max = 32, value = 8 }
   :separator{ text = "Options" }
   :check  { id = "dither", text = "Dither", selected = false }
   :button { id = "ok",     text = "&OK", focus = true }
   :button { id = "cancel", text = "Cancel" }
   :show{ wait = true }
local d = dlg.data                            -- { from=..., to=..., steps=8, dither=false, ok=true, cancel=false }
if d.ok then ... end
```

Every builder method returns the dialog, so calls chain. `Dialog()` with no arguments is allowed (empty
title).

**Layout.** A dialog is a list of rows. Widgets are added left to right into the current row;
`dlg:newrow()` (or `dlg:newrow{ always = true }`, accepted and treated the same) starts the next row;
`separator` always sits in a row of its own. A widget with a `label = "..."` gets that text in a label
column on the left of its row (renderers align the label columns of all rows of a dialog). Radio buttons
in one row form one group; `newrow` starts a new group.

**Common widget options:** `id` (string; widgets without one get a generated `_wN` id), `label`, `focus`
(initial keyboard focus, one per dialog is honored), `enabled` (default true), `visible` (default true),
`hexpand` (the widget takes the free width of its row; default true for entry, number, slider, color,
combobox, file and false otherwise).

| Method | Options | `dlg.data[id]` | Callbacks |
| --- | --- | --- | --- |
| `label{ text }` | | none | |
| `button{ text, focus }` | | `true` if it was the button that closed the dialog, else `false` | `onclick` |
| `check{ text, selected }` | | boolean | `onclick` |
| `radio{ text, selected }` | | boolean | `onclick` |
| `entry{ text }` | | string | `onchange` |
| `number{ text or value, decimals }` | `decimals` 0..6 (default 0) | number | `onchange` |
| `slider{ min, max, value }` | integers | integer | `onchange` (every change), `onrelease` (drag ends) |
| `color{ color }` | color is `0xRRGGBBAA` | integer `0xRRGGBBAA` | `onchange` |
| `combobox{ option, options }` | `options` list of strings, `option` the selected one | the selected string | `onchange` |
| `file{ open, save, filename, filetypes, title }` | `open=true` (default) picks an existing file, `save=true` a file to write; `filetypes` is a list of extensions without the dot | string path (empty if none) | `onchange` |
| `shades{ colors }` | list of colors, shown as swatches | none | `onclick` (`ev.color`) |
| `separator{ text }` | text optional | none | |
| `newrow{ always }` | | | |

`button` without `onclick` closes the dialog (like Aseprite): `data.<id>` is then `true`. A `button` with
`onclick` leaves the dialog open (the callback may call `dlg:close()`).

**Callbacks** run on the script's Lua state, receive one table `ev` (`ev.id`, `ev.value`, plus `ev.color`
for `color`/`shades`), and are called after `dlg.data` already holds the new value. `ev` is optional to
use. Unknown fields in a constructor table are ignored (Aseprite parity); wrongly typed known fields raise
a Lua error.

`not in v1:` `canvas`, `image`, `tab`/`endtabs`, `menu`/`menuItem`, `dlg.bounds`-driven layout (the field
is accepted and ignored), `Dialog{ parent = }`. Calling them raises `"<name> is not supported by Pixmith"`.

#### Dialog methods and fields

- `dlg:show{ wait = true }`
  - `wait=true` (default): **modal**. The call returns when the dialog is closed (by a button without
    `onclick`, `dlg:close()`, or the user closing the window). While it waits, widget callbacks and other
    events are served on the same Lua state. Returns `dlg`.
  - `wait=false`: **non-modal**. The call returns at once; the dialog lives on after the script's main
    chunk finished. Callbacks keep running as events arrive (see 3). Returns `dlg`.
  - `bounds`, `autoscrollbars` accepted, ignored.
  - Showing a dialog that is already open raises an error. A closed dialog can be shown again.
  - Without a UI (`pixmith-cli`, the plain `pixmith_run_script*` calls), `show` returns immediately and the
    dialog counts as closed with default data; `app.isUIAvailable` is `false`.
- `dlg:close()` closes it (calls `onclose` once). No effect when not open.
- `dlg.data` (get): a fresh table of the current values by id. `dlg.data = { id = value, ... }` (set)
  assigns widget values; unknown ids are ignored, wrong types raise an error.
- `dlg:modify{ id = "x", ... }` changes a widget while the dialog is open or not. Accepted keys by widget:
  `text`, `enabled`, `visible`, `focus`, `selected` (check/radio), `value` (slider/number),
  `min`, `max` (slider), `decimals`, `color`, `colors` (shades), `option`, `options` (combobox),
  `filename`, `label`. `id` may also be a list of ids to apply the same change. Unknown id raises an error.
  Title: `dlg:modify{ title = "New" }` (no `id`).
- `dlg:repaint()` accepted, no-op. `dlg.bounds` read/write accepted, stored, not used by renderers.
- `Dialog{ onclose = fn }`: called after the dialog closed, whichever way.

#### Alerts

`app.alert(text)` keeps its old behavior (collected, shown when the script's transaction ends). The table
form shows a modal message and returns the 1-based number of the pressed button (0 if the window was
closed):

```lua
local n = app.alert{ title = "Delete?", text = "Delete the guides?", buttons = { "&Yes", "&No" } }
-- text may be a list of lines; buttons default to { "OK" }
```

`app.alert{...}` is built on a modal `Dialog`, so the front end needs no extra support.

### 1.2 Events

```lua
local l = app.events:on("sitechange", function(ev) print(app.activeFrame) end)   -- returns a listener id
app.events:off(l)            -- by listener id or by the function that was registered
sprite.events:on("change", function(ev) if ev.fromUndo then ... end end)
```

| Object | Event | `ev` fields | When the front end sends it |
| --- | --- | --- | --- |
| `app.events` | `sitechange` | `layer`, `frame` (1-based, the new site) | active document, layer or frame changed |
| `app.events` | `fgcolorchange`, `bgcolorchange` | `color` | the drawing colors changed |
| `app.events` | `beforecommand`, `aftercommand` | `name` (command name, e.g. `"Undo"`, `"Flip"`, see 5) | around menu commands |
| `sprite.events` | `change` | `fromUndo` (bool) | the user edited the document or used undo/redo |
| `sprite.events` | `filenamechange` | | the document was saved under another name |

`sprite.events` listeners belong to the document that was active when they were registered (the front
end's `doc_tag`, see 4); they fire only for that document. `app.events` listeners are global.
`ev:stopPropagation()` and canceling `beforecommand` are *not in v1*.

Event handlers run as a transaction (3.2): changes they make to `sprite` become one undo step each, and an
error in a handler rolls that handler back and reports the error; the listener stays registered.
The front end **must not** send `sprite.change` for changes that come from applying a script result (that
would make handlers trigger themselves).

### 1.3 Plugins

A plugin is a script that defines two global functions and is loaded once, not per run:

```lua
function init(plugin)
  plugin.preferences.count = (plugin.preferences.count or 0) + 1       -- persisted automatically
  plugin:newCommand{
    id = "palette_ramp", title = "Palette Ramp...", group = "file_scripts",
    onenabled = function() return sprite ~= nil end,                   -- optional, default enabled
    onclick = function() ... Dialog ... end,                           -- runs on the active document
  }
end
function exit(plugin) ... end                                          -- optional, runs when unloaded
```

- `plugin.name` (string), `plugin.path` (the plugin's folder, as text), `plugin.preferences` (a table of
  booleans, numbers, strings and nested tables with string keys or arrays; loaded before `init`, saved
  after each transaction that changed it and after `exit`; at most 1 MB of JSON, depth 16).
- `plugin:newCommand{ id, title, group, onclick, onenabled, onchecked }` registers a command. `id` and
  `title` are required, `group` defaults to `"file_scripts"` (the File > Scripts submenu); every other
  group is shown in the same submenu in v1. `plugin:deleteCommand(id)` removes it.
- `onclick` runs on a snapshot of the active document, as a transaction (3.2). `onenabled`/`onchecked` are
  re-evaluated after every transaction and event; they must not change the sprite.
- Plugins have no `sprite` outside a command or event (it is `nil` in `init`/`exit`; `app.sprite` is the
  same value). In commands and events `sprite` is the active document.
- `plugin:newMenuGroup`, `plugin:newMenuSeparator` *(not in v1)*.

Plain scripts (the "Run Script" kind) may use Dialogs and events too; a script that registers listeners or
leaves a non-modal dialog open stays loaded until all dialogs are closed and no listener is left (or the
user stops it).

### 1.4 Other additions

- `app.sprite`: the same as the global `sprite` (`nil` without a document).
- `app.isUIAvailable`: boolean.
- `app.fgColor`, `app.bgColor`: `Color` objects when read (the front end reports them as `0xRRGGBBAA`); a write takes a `Color` or an integer. A write is sent to the front end as `app.set`.
- `app.apiVersion` becomes 3.

---

## 2. Messages

All messages are UTF-8 JSON objects with a string `type`. Colors are numbers `0xRRGGBBAA`
(0..4294967295). Booleans are `true`/`false`. A front end ignores unknown fields and unknown `type`s.

### 2.1 Core to front end (polled)

Every message has `"session"` (number). Delivered in order, per session.

**`dialog.open`**

```json
{"type":"dialog.open","session":3,"dialog":{
  "id":"s3.d1","title":"Palette ramp","modal":true,"notitlebar":false,
  "rows":[
    {"widgets":[{"id":"info","kind":"label","text":"Pick two colors","enabled":true,"visible":true}]},
    {"widgets":[{"id":"from","kind":"color","label":"From","color":488714575,"enabled":true,"visible":true,"hexpand":true},
                {"id":"to","kind":"color","label":"To","color":4289790975,"enabled":true,"visible":true,"hexpand":true}]},
    {"widgets":[{"id":"steps","kind":"slider","label":"Steps","min":2,"max":32,"value":8,"enabled":true,"visible":true,"hexpand":true}]},
    {"widgets":[{"id":"_w5","kind":"separator","text":"Options"}]},
    {"widgets":[{"id":"ok","kind":"button","text":"&OK","focus":true,"enabled":true,"visible":true},
                {"id":"cancel","kind":"button","text":"Cancel","enabled":true,"visible":true}]}
  ]}}
```

Widget object, all kinds: `id` (string, unique in the dialog), `kind`, `label` (string or absent),
`enabled`, `visible`, `focus` (only when true), `hexpand`. Kind-specific fields:

| `kind` | fields |
| --- | --- |
| `label` | `text` |
| `button` | `text` |
| `check` | `text`, `selected` |
| `radio` | `text`, `selected`, `group` (number; radios with the same `group` in one dialog are exclusive) |
| `entry` | `text` |
| `number` | `value`, `decimals` |
| `slider` | `value`, `min`, `max` |
| `color` | `color` |
| `combobox` | `options` (array of strings), `option` (selected string or `null`) |
| `file` | `filename`, `open` (bool), `save` (bool), `filetypes` (array), `title` |
| `shades` | `colors` (array) |
| `separator` | `text` (may be empty) |

Button text may contain `&` marking a mnemonic letter (`"&OK"`); renderers strip it (and may underline
the letter). `&&` is a literal `&`.

**`dialog.modify`**: patches to an open dialog, sent when the script calls `:modify` or assigns
`dlg.data`, and when a radio selection changes elsewhere in its group.

```json
{"type":"dialog.modify","session":3,"dialog":"s3.d1","title":"New title",
 "widgets":{"steps":{"value":12,"max":64},"info":{"text":"Done"}}}
```

`widgets` maps widget id to a partial widget object (any of the fields in the table above, plus `enabled`,
`visible`, `focus`, `label`). A front end applies the patch to its copy and overwrites its local state for
those fields (also the text a user has typed into an entry). `title` is optional. The core **does not**
send patches for values the user changed (no echo), so a front end keeps its own edits.

**`dialog.close`**: `{"type":"dialog.close","session":3,"dialog":"s3.d1"}`. Remove the dialog. Sent when
the script closes it or a button without `onclick` was pressed. The front end must not answer.

**`state`**: `{"type":"state","session":3,"state":"running"}`, one of

- `running`: Lua is executing. Events sent now are queued; show progress with a Cancel button if this lasts
  more than about 300 ms and no modal dialog of this session is open.
- `waiting`: inside a modal `show`, waiting for the user.
- `idle`: the main chunk and every callback are done; the session stays loaded (non-modal dialogs,
  listeners, plugin).

Sent on every change of state. The first message of a session is `running`.

**`txn`**: one Lua call chain finished (the main chunk, a callback and everything it caused, a command, an
event handler).

```json
{"type":"txn","session":3,"txn":7,"doc":1002,"ok":true,"error":null,
 "messages":["hello"],"commit":true,"label":"Palette ramp","select":{"layer":2,"frame":1}}
```

- `doc`: the `doc_tag` the front end passed with the call that started this transaction (0 if none).
- `ok=false`: `error` holds the text; nothing is committed (the document stays as it was); show
  "Script error in <name>" with `error` and `messages`, except when `error` is exactly `"script cancelled"`
  (show nothing).
- `messages`: `print` and `app.alert(text)` output in order (show them in a notice if non-empty).
- `commit=true`: the transaction changed the document. The front end **must** call
  `pixmith_script_apply(session, txn, doc)` exactly once, on the UI thread, with the document that belongs
  to `doc` (or `NULL` to discard it, for example because the document was closed). One undo step named
  `label`. Apply before processing later messages of this session when convenient (they may depend on it).
- `select` (optional): select this layer and frame (1-based) after applying (`app.activeLayer`,
  `app.activeFrame` assigned by the script).

**`app.set`**: `{"type":"app.set","session":3,"fgColor":4278190335}` and/or `"bgColor"`: the script assigned
`app.fgColor`/`app.bgColor`.

**`command.add`** / **`command.update`** / **`command.remove`** (plugins):

```json
{"type":"command.add","session":4,"plugin":"my-plugin","command":
  {"id":"palette_ramp","title":"Palette Ramp...","group":"file_scripts","enabled":true,"checked":false}}
{"type":"command.update","session":4,"command":{"id":"palette_ramp","enabled":false}}
{"type":"command.remove","session":4,"id":"palette_ramp"}
```

A command is identified by `(session, id)`. Invoke it with a `command` event (2.2).

**`session.end`**: `{"type":"session.end","session":3}`. The session is gone (finished, stopped, crashed).
Always the last message of a session; close all its dialogs and remove its commands. After a poll returned
it, the id is invalid.

### 2.2 Front end to core (`pixmith_script_send` / `pixmith_script_broadcast`)

```json
{"type":"widget","dialog":"s3.d1","id":"steps","event":"change","value":12}
{"type":"widget","dialog":"s3.d1","id":"steps","event":"release"}
{"type":"widget","dialog":"s3.d1","id":"ok","event":"click"}
{"type":"widget","dialog":"s3.d1","id":"dither","event":"click","value":true}
{"type":"widget","dialog":"s3.d1","id":"pal","event":"click","color":4278190335}
{"type":"dialog.closed","dialog":"s3.d1"}
{"type":"command","id":"palette_ramp"}
{"type":"app.event","name":"sitechange","layer":2,"frame":1}
{"type":"app.event","name":"fgcolorchange","color":4278190335}
{"type":"app.event","name":"beforecommand","command":"Undo"}
{"type":"sprite.event","name":"change","fromUndo":false}
```

- `change` (entry, number, slider, color, combobox, file): `value` is the new value (string for entry,
  combobox and file; number for number and slider; color number for color). Sent on every edit (entries:
  on every text change; sliders: while dragging; combobox: on selection). Coalescing is the core's job:
  while a callback runs, several `change`s of the same widget collapse into the last.
- `click` (button, check, radio, shades): check and radio carry the new `value` (bool); shades carry `color`.
  A radio click always has `value:true`; the core clears the others of its group (and patches them).
- `release`: slider drag ended.
- `dialog.closed`: the user closed the window (x button, Esc). Sent once; the dialog is already gone from
  the front end. A dialog the core closed itself is not reported.
- Unknown dialog or widget ids are ignored (events can race with a close).
- For `widget` events the document argument is ignored when the session is inside a modal `show`
  (the core keeps working on its own copy, see 3.2) and used otherwise.

---

## 3. Threading and transactions

### 3.1 Sessions

Every script run and every plugin is a **session**: a Lua state on its **own worker thread inside the
core**. A Lua state never leaves its thread, so there is no locking in Lua and no `Send` hazard; the front
end only talks to the core through message queues. The ABI is therefore thread-safe: any front-end thread
may call any `pixmith_script_*` function except `pixmith_script_apply`, which must be called on the thread
that owns the document (the UI thread), like every other call that touches a `PixmithDoc`.

A session ends when its main chunk returned **and** it has no open dialog, no event listener and is not a
plugin; or when it errors; or on `pixmith_script_stop`; then `session.end` is queued.

### 3.2 Transactions and the document

A `PixmithDoc` is never shared with the worker. Every call that can start Lua code takes an optional `doc`:

1. On the calling thread the core takes a cheap snapshot of the sprite and selection (copy-on-write,
   same as `pixmith_snapshot`) and queues it with the event.
2. The worker runs the Lua code against that snapshot: the script sees `sprite` as before; `app.activeLayer`
   / `app.activeFrame` default to the `layer`/`frame` passed with the call.
3. When the Lua chain finishes the worker emits `txn` (with `commit=true` when the snapshot changed). The
   front end applies it with `pixmith_script_apply`, which makes **one undo step** if the document still
   equals the snapshot it was taken from. If the user edited the document in between, apply returns
   `-1`, discards the result and sets `pixmith_last_error`; the front end shows "The document changed
   while the script was running; its result was discarded."
4. A callback that fails raises its error, **that callback's changes are rolled back**, the error is added
   to the transaction (`ok=false` only if the main chunk, a command or an event handler itself failed;
   callbacks of an open modal dialog report their error in `messages` as `"error: ..."` and the dialog
   stays open).

**Modal wait.** While the main chunk sits in `show{wait=true}` the transaction stays open: widget events
run callbacks inside it on the same snapshot, so the whole script (dialog, callbacks, the code after
`show`) is **one undo step**, committed when the chunk ends, exactly like a script without UI. The front
end blocks editing of that document while a modal dialog is open, so there is no conflict.

**Non-modal.** The main chunk's changes are committed when it returns. After that, each widget event,
command or app event is its own transaction (own snapshot, own `txn`, own undo step).

**Limits.** The instruction and time budgets (300M instructions, 10 s of running time, 128 MB of Lua
memory) apply **per transaction**, not to the life of the session; callbacks served inside a modal `show`
belong to the transaction that contains it, and the time spent waiting there does not count. `pixmith_script_stop` (or `pixmith_script_cancel(true)` for the legacy calls) interrupts Lua at the
next instruction check; a blocked `show` returns at once; the transaction fails with `"script cancelled"`.
Nothing a script does can panic the app: panics on the worker are caught and end that session with an
error.

### 3.3 Typical flows

*Run a script with a modal dialog.* `start` -> `state running` -> `dialog.open modal` -> `state waiting`
-> user clicks `ok` (front end sends `widget click`) -> `state running`, callbacks, `dialog.close` ->
`txn commit=true` -> front end calls `apply` -> `session.end`.

*Plugin command.* Front end loads the plugin once with `pixmith_plugin_load` -> `command.add`,
`txn` (init) -> `state idle`. Menu click -> `send(session, doc, tag, layer, frame, {"type":"command"...})`
-> `state running` -> maybe `dialog.open` -> ... -> `txn` -> `apply` -> `state idle`.

*Live preview.* Non-modal dialog with `slider{ onchange = ... }` that edits the sprite: every `change` runs a
transaction. Front ends should send slider `change`s at most once per rendered frame.

---

## 4. C ABI

Declared in `app/Sources/PixmithCore/pixmith_script_ui.h`, implemented in `core/src/scripting/host.rs`.
All functions are panic-guarded (`ffi::guarded`); failures set `pixmith_last_error`. Strings returned by
the core are released with `pixmith_string_free`. All strings are UTF-8, NUL terminated.

```c
#include <stdbool.h>
#include <stdint.h>
#include "pixmith.h"

// Starts a script session. `doc` (may be NULL) is snapshotted for the main chunk; `doc_tag` is any number
// the front end uses to find the document again ("which tab"), echoed in `txn` messages. `layer`/`frame`
// are 0-based. `name` is a display name (e.g. the file name). Returns the session id, 0 on failure
// (e.g. too many sessions: at most 64 run at once).
uint64_t pixmith_script_start(PixmithDoc *doc, uint64_t doc_tag, const char *source, const char *name,
                              uint32_t layer, uint32_t frame);

// Loads a plugin: runs `source`, then calls its global init(plugin). `dir` is plugin.path (may be NULL),
// `prefs_path` the JSON file for plugin.preferences (may be NULL: not persisted). Returns the session id.
uint64_t pixmith_plugin_load(const char *source, const char *name, const char *dir, const char *prefs_path);

// Queued messages of ALL sessions as a JSON array (possibly "[]"), oldest first. Waits up to timeout_ms for
// the first message (0 = do not wait). Free with pixmith_string_free.
char *pixmith_script_poll(uint32_t timeout_ms);

// Delivers one event (2.2) to a session. `doc` (may be NULL) is snapshotted if the event starts a
// transaction. Returns false if the session does not exist, or the JSON is invalid.
bool pixmith_script_send(uint64_t session, PixmithDoc *doc, uint64_t doc_tag, uint32_t layer, uint32_t frame,
                         const char *event_json);

// Delivers an app.event / sprite.event (2.2) to every session that listens for it (cheap when none does).
// Returns how many sessions received it.
uint32_t pixmith_script_broadcast(PixmithDoc *doc, uint64_t doc_tag, uint32_t layer, uint32_t frame,
                                  const char *event_json);

// Applies the result of transaction `txn` (from a `txn` message with commit=true) to `doc` as one undo step.
// Returns 1 applied, 0 nothing to apply, -1 conflict (document changed; result discarded), -2 unknown
// session/transaction. `doc == NULL` discards the result (returns 0).
int32_t pixmith_script_apply(uint64_t session, uint64_t txn, PixmithDoc *doc);

// The drawing colors scripts read as app.fgColor / app.bgColor (0xRRGGBBAA). Call when they change.
void pixmith_script_set_colors(uint32_t fg, uint32_t bg);

// Stops a session: interrupts Lua, calls a plugin's exit(plugin), saves its preferences, queues
// command.remove / dialog.close / session.end. Returns at once.
void pixmith_script_stop(uint64_t session);

// Number of live sessions (for "keep polling" decisions).
uint32_t pixmith_script_count(void);
```

`pixmith_run_script`, `pixmith_run_script_at` and `pixmith_script_cancel` stay as they are for headless
use (CLI, tests, old callers): Dialog code cannot show a window there (`app.isUIAvailable == false`).

### Rust helper for a front end that is also Rust

`pixmith_core::scripting::host` exposes the same calls as safe-ish Rust functions with the same names
minus the prefix, and `pixmith_core::formats::json` parses/serializes the messages.

---

## 5. What a front end does

1. **Poll** `pixmith_script_poll(0)` once per UI tick while `pixmith_script_count() > 0` (~30 Hz is enough;
   request a repaint after a message arrived). Dispatch each message by `type` and `session`.
2. **Run Script / Scripts menu item:** `pixmith_script_start(doc, tag, source, name, layer, frame)`. Do not
   block the UI thread. Treat a `running` session of the active document as a busy job (no editing,
   Cancel button -> `pixmith_script_stop`) unless a modal dialog of it is open.
3. **Render** `dialog.open` with the design-system widgets (modal = centered, blocks the window;
   non-modal = movable tool window). Keep per-dialog widget state; apply `dialog.modify`; remove on
   `dialog.close` / `session.end`. Send widget events as in 2.2.
4. **`txn`:** show `error`/`messages` as a notice, call `pixmith_script_apply` when `commit` is true,
   then select `select`, mark the document changed.
5. **Plugins:** at launch (and when the extensions list changes) call `pixmith_plugin_load` for each
   enabled script plugin; build File > Scripts entries from `command.*`; a menu click is
   `pixmith_script_send(session, activeDoc, tag, layer, frame, {"type":"command","id":...})`.
   Unload with `pixmith_script_stop`.
6. **Events:** call `pixmith_script_broadcast` when
   - the active document, layer or frame changes -> `app.event sitechange`,
   - the foreground/background color changes -> `fgcolorchange`/`bgcolorchange`,
   - around menu commands -> `beforecommand`/`aftercommand` with `command` set to the menu title
     ("Undo", "Redo", "Flip Horizontal", ...; at least those of the Edit and Sprite menus),
   - the user edited the document or undid/redid -> `sprite.event change` (`fromUndo` true for undo/redo),
   - a document is saved as another file -> `sprite.event filenamechange`,
   and call `pixmith_script_set_colors` whenever the drawing colors change.
7. **Quit:** `pixmith_script_stop` every session (plugins' `exit` runs).

The reference renderer is `desktop/src/ui/lua_dialog.rs`; its calls into the core are in
`desktop/src/calls/script_ui.rs`.
