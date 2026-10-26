# Lua host contract: file permissions and script-created documents

What a front end (macOS SwiftUI `app/` through the C ABI, egui `desktop/` through the Rust crate) has to
provide so that Aseprite-style scripts work: scripts that read or write files, run commands, and create,
open, save or switch documents. Sessions, dialogs and events are in [LUA_DIALOG.md](LUA_DIALOG.md); the script
API itself is in [LUA_API.md](LUA_API.md). Everything here lives in `core/src/scripting/` (`access.rs`,
`sprites.rs`, `host.rs`).

Two things are asked of a front end:

1. **A permission handler** (section 1): a function that decides whether a script may touch a path or run a command.
2. **Taking the documents a run produced** (section 2): open the new documents as tabs, activate the right one,
   adopt a path the script saved the current document to.

Both are optional to start with: without a handler every file or command request is denied (the script gets a
Lua error `permission denied`), and a front end that ignores section 2 simply never shows the new documents.

---

## 1. File and command permissions ("ask first")

### What is guarded

Every one of these asks, unless the path is already allowed (below):

| Script call | Request |
| --- | --- |
| `io.open(p, mode)`, `io.lines(p)`, `io.input(p)`, `io.output(p)`, `file:...` on those | `Read`, or `Write` when the mode can write (`w`, `a`, `r+`, `w+`, `a+`) |
| `os.remove(p)`, `os.rename(a, b)` | `Write` on `p`, resp. on `a` and on `b` (two requests) |
| `os.execute(cmd)`, `io.popen(cmd)` | `Execute`, with `path` = the command line |
| `os.tmpname()` | none; the returned path is allowed (read and write) for the rest of the run |
| `app.fs.listFiles(d)`, `isFile`, `isDirectory`, `fileSize` | `Read` on the path |
| `app.fs.makeDirectory`, `makeAllDirectories`, `removeDirectory` | `Write` |
| `Image{ fromFile = p }`, `app.open(p)`, `Sprite{ fromFile = p }` | `Read` |
| `sprite:saveAs(p)`, `sprite:saveCopyAs(p)`, `image:saveAs(p)` | `Write` (the file or, for numbered frame files, the first one) |

Pure path arithmetic (`app.fs.filePath`, `joinPath`, `normalizePath`, `pathSeparator`, ...) and
`app.fs.currentPath` / `appPath` / `tempPath` / `userDocsPath` / `userConfigPath` never ask.

`path` in a request is absolute and lexically normalized (`.` and `..` removed, no symlink resolution); a relative
path from the script is resolved against the process working directory first. For `Execute` it is the raw command.

### Resolution order in the core

For a request `(script_name, path, access)`:

1. Denied outright if `path` is empty or contains a NUL.
2. Allowed if `path` is under a pre-allowed prefix (`allowed_paths`, below), or under a path granted
   `AllowOnce` earlier in the same run (a prefix match on whole path components: a grant for `/tmp/out` covers
   `/tmp/out` and `/tmp/out/a.png`, not `/tmp/out2`), or under a path returned by `os.tmpname` in this run.
   `Execute` is never covered by prefixes, only by an exact earlier grant of the same command.
3. Allowed if this script has been granted `AllowAlwaysForScript` for this access class earlier in the app run.
   A `Write` grant also covers `Read`; `Execute` is its own class; the grant is for every path.
4. Otherwise the handler is called. No handler set means `Deny`.

"A run" is one `pixmith_run_script*` call, or one transaction of a session (the main chunk or one event). The
script key for "always" is the script name: the `name` passed to `pixmith_script_start` / `Spec::Script` (the
front end should use the file name or path, so that two scripts with the same display name are not merged).

### Rust

```rust
// pixmith_core::scripting::access
pub enum Access { Read, Write, Execute }
pub struct PermissionRequest { pub script_name: String, pub path: String, pub access: Access }
pub enum Permission { Deny, AllowOnce, AllowAlwaysForScript }

pub fn set_permission_handler(h: Option<Arc<dyn Fn(&PermissionRequest) -> Permission + Send + Sync>>);

/// Process-wide prefixes that are always allowed for Read and Write (for the MCP server's `allow_paths`).
pub fn set_allowed_paths(paths: Vec<PathBuf>);
pub fn allowed_paths() -> Vec<PathBuf>;

/// Forget every "always" grant (for tests, or a "Reset script permissions" menu item).
pub fn reset_grants();
```

The handler is global (one per process), is called **on the thread that runs the script** (a session worker
thread, or the caller's thread for the synchronous `run*` functions), and may block while it shows a dialog.
It must be `Send + Sync`: a front end marshals to its UI thread and waits for the answer. The core measures the
time spent inside the handler and does not count it toward the script's wall-clock limit (it is added to the
same "waited" total as `Dialog:show{ wait = true }`; the instruction and memory budgets are unaffected). Calling
`set_permission_handler` while scripts run is allowed; the next request uses the new handler.

Pre-allowed prefixes can also be given per run: `scripting::RunConfig { script_name, doc_path, allowed_paths }`
with `scripting::run_in(e, source, active, limits, cancel, &cfg)`. `run`, `run_with` and `run_with_limits` use
`RunConfig::default()` (script name `"script"`, no document path, only the global prefixes).

### C ABI

```c
// access: 1 = read, 2 = write, 4 = execute. Returns 0 deny, 1 allow once, 2 always for this script.
// Called on the script's thread; may block. `script` and `path` are valid only during the call.
typedef uint32_t (*pixmith_script_permission_fn)(void *ctx, const char *script, const char *path, uint32_t access);

// Installs (or, with fn == NULL, removes) the handler. `ctx` is passed back unchanged.
void pixmith_script_set_permission_handler(pixmith_script_permission_fn fn, void *ctx);

// Replaces the process-wide pre-allowed prefixes (read and write). count may be 0.
void pixmith_script_set_allowed_paths(const char *const *paths, uint32_t count);
```

Any other return value from the handler is treated as 0 (deny). A panic or a NULL result from the handler (Rust)
is a deny. "Always" is remembered by the core for the lifetime of the process, not persisted.

### What the user should see

One dialog per request the core cannot answer itself: "`<script>` wants to read/write `<path>` / run `<command>`"
with **Deny**, **Allow once**, **Always allow this script** (the last wording should say what it grants: every
file of that kind, for this session of the app). Closing the dialog is a deny.

---

## 2. Documents created, opened, saved or closed by a script

A script can create documents (`Sprite(w, h)`, `Sprite(ImageSpec)`, `Sprite{ fromFile = }`), open them
(`app.open(path)`), save them (`sprite:saveAs`, `sprite:saveCopyAs`, done inside the core after the permission
check), switch (`app.activeSprite = s`) and close them (`sprite:close()`). The core keeps these documents in the
script's Lua state; what the front end sees is the result of the run.

`app.sprites` lists the document the script was started on (if any) followed by the documents the script created
or opened, so a script cannot see the other tabs of the front end. `app.open(path)` of the path the script's own
document has (or of a file the run already opened) returns that document; `Sprite{ fromFile }` always loads a new one.

### The document the script runs on ("original")

* It is changed exactly as today: one undo step through `pixmith_script_apply` / the Rust `Outcome`.
* `sprite.filename` is the path the front end registered for it (below), or `""`.
* `sprite:saveAs(path)` with a document format (`.aseprite`, `.ase`, `.pxmt`) writes the file and makes `path` the
  document's file. The front end should adopt it (`originalPath`) and, when `originalSaved` is true, treat the
  document as clean after applying the commit (the saved state is the state at the end of the run).
  `saveAs` to a non-document format (`.png`, `.gif`, ...) is an export and does not change the document's path.
* `sprite:close()` on it sets `closeOriginal`; the front end closes the tab without a save prompt (the core drops
  the document's changes in that case, no commit is produced, `pixmith_script_apply` has nothing to apply).

Registering a document's path (needed for `sprite.filename`, `app.open` returning the original, and for the
"directory of the document" defaults):

```rust
pub fn host::set_doc_path(doc_tag: u64, path: Option<PathBuf>);   // call when a document is opened, saved as, renamed, closed
pub struct scripting::RunConfig { pub script_name: String, pub doc_path: Option<PathBuf>, pub allowed_paths: Vec<PathBuf> }
```
```c
// `doc_tag` is the same number passed to pixmith_script_start/send/broadcast. path == NULL forgets it.
void pixmith_script_set_doc_path(uint64_t doc_tag, const char *path);
```

`DocSnap::of(e, tag, layer, frame)` and the C start/send/broadcast functions read the registered path for
`tag`. The synchronous `pixmith_run_script*` have no tag: use `pixmith_run_script_named` below.

### New documents

```rust
// pixmith_core::scripting
pub struct NewDocument {
    pub sprite: Sprite,
    pub path: Option<PathBuf>,   // set if the script opened it from a document file (.aseprite, .ase, .pxmt) or saved
                                 // it with saveAs to one; images (PNG, GIF, ...) it opened arrive untitled (None)
    pub modified: bool,          // false when `sprite` is exactly what is in `path` (open it clean)
}
pub enum ActiveDoc {
    Unchanged,       // the script did not switch documents: leave the active tab alone
    Original,        // the script made the document it ran on active again (or never left it)
    New(usize),      // index into `new_documents`
}
pub struct Outcome {
    pub messages: Vec<String>,
    pub select: Option<(usize, usize)>,
    pub error: Option<String>,
    // new:
    pub new_documents: Vec<NewDocument>,   // in creation order, closed ones removed; empty on error
    pub active: ActiveDoc,
    pub close_original: bool,
    pub original_path: Option<PathBuf>,    // the original was saved with saveAs to a document format
    pub original_saved: bool,              // ... and has not changed since
}
```

A script that fails produces no new documents and no path/close changes (like it produces no commit), but files it
already wrote stay written.

Which sprite is active after the run: every `Sprite(...)`, `Sprite{fromFile}` and `app.open` makes its sprite the
active one (as in Aseprite); `app.activeSprite = s` or `s:close()` of the active one changes it. At the end
`active` says which. A front end opens every `new_documents` entry as a new tab (in order), then activates the
one named by `active` (`Unchanged` / `Original`: the tab the script ran on; `New(i)`: tab `i`). When a script
leaves the new documents unsaved they are untitled and modified (the front end asks to save on close).
If `path` is set and a tab for that path is already open the front end may focus it instead of opening a second.

#### Session transactions (`pixmith_script_poll` / `Host`)

The `txn` message of a transaction that used documents carries one more key, `docs` (absent otherwise):

```json
{"type":"txn","session":3,"txn":7,"doc":42,"ok":true,"commit":true, "...": "...",
 "docs": {"new":[{"index":0,"path":"/Users/me/a.aseprite","modified":false},
                 {"index":1,"path":null,"modified":true}],
          "active": 1,
          "closeOriginal": false,
          "originalPath": null,
          "originalSaved": false}}
```

`active` is `null` (unchanged), `"original"`, or the index of an entry of `new`. The front end then takes the
documents:

```rust
impl Host { pub fn take_doc(&self, session: u64, txn: u64, index: u32) -> Option<Editor>; }
```
```c
// Ownership of a new document passes to the caller (release with pixmith_free, or keep it as a tab).
// NULL if unknown, already taken, or dropped (the core keeps the documents of the last 64 transactions).
PixmithDoc *pixmith_script_take_doc(uint64_t session, uint64_t txn, uint32_t index);
```

Order for one `txn` message: `pixmith_script_apply` (if `commit`), then `pixmith_script_take_doc` for each `new`
entry, open the tabs, then activate `active`, then handle `closeOriginal` / `originalPath`. `take_doc` and
`apply` are independent; a conflicting commit (-1) does not discard the new documents.

#### Synchronous runs (`pixmith_run_script*`)

The same data is available after a synchronous run, from the thread that made the call, until the next
synchronous run on that thread:

```c
// Like pixmith_run_script_at, with a script name (permission requests, "always" key; NULL = "script") and the
// document's path (sprite.filename, may be NULL).
bool pixmith_run_script_named(PixmithDoc *doc, const char *source, uint32_t layer, uint32_t frame,
                              const char *script_name, const char *doc_path, char **out_messages);

// JSON of the last synchronous run: {"new":[...],"active":...,"closeOriginal":...,"originalPath":...,
// "originalSaved":...} (same shape as `docs` above, "{}" when it used no documents). Free with pixmith_string_free.
// (session, txn) must be (0, 0).
char *pixmith_script_outcome(uint64_t session, uint64_t txn);

// session = 0, txn = 0 selects the last synchronous run on this thread.
PixmithDoc *pixmith_script_take_doc(uint64_t session, uint64_t txn, uint32_t index);
```

The existing `pixmith_run_script`, `pixmith_run_script_at`, `pixmith_run_script_limits` keep their signatures and
behavior (the structured `PIXMITH-SELECT` line is unchanged); they too leave their outcome for
`pixmith_script_outcome(0, 0)`.

### Limits on what a script can create

At most 32 documents alive in one run, each within the usual canvas limits (the same ones `pixmith_new` applies);
the Rust memory of these documents is not covered by the Lua memory limit, so the core also caps the total at
512 MB of pixel data per run. Exceeding a cap raises a Lua error.

### Implementation checklist

macOS app (`app/Sources/Pixmith/App/ScriptSessions.swift`):

1. Call `pixmith_script_set_permission_handler` once with a function that dispatches to the main actor, shows an
   `NSAlert` and returns the answer (use a semaphore: the callback runs on a script thread).
2. Call `pixmith_script_set_doc_path(tag, path)` when a document gains, changes or loses its file.
3. In the `txn` handler, after `pixmith_script_apply`, read `docs`: take each new doc with
   `pixmith_script_take_doc`, open it as a tab (clean if `modified == false`), then activate / close / adopt the path.

egui desktop (`desktop/src/calls/script_ui.rs`, `gamedev.rs`):

1. `scripting::access::set_permission_handler(Some(..))`: send the request to the UI over a channel and block on the
   reply channel.
2. `host::set_doc_path(tag, path)` as above; use `Outcome.new_documents` / `active` for synchronous runs and the
   `docs` object of `txn` messages plus `Host::take_doc` for sessions.
