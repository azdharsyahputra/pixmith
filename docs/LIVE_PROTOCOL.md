# Pixmith Live protocol

Live mode lets a local tool, mainly the [Pixmith MCP server](https://github.com/azdharsyahputra/pixmith-mcp), drive the
Pixmith app that is open on the same computer. AI edits appear in the open window as they happen, and each one is a
normal undo step the user can take back with Cmd/Ctrl+Z. Both front ends implement it: the macOS app (`app/`) and the
Windows/Linux/macOS egui app (`desktop/`). When no app is running, the MCP server falls back to editing files on
its own (headless).

## Design in one paragraph

The app does not run the MCP tools itself. It exposes its open documents as bytes. For each editing call the
client fetches the document as `.pxmt` bytes, edits that copy with its own `pixmith-core` (the same code the app
uses), and sends the result back. The app swaps it in as one labelled undo step. A revision number per document
stops a client from overwriting edits the user made in the meantime: the apply is refused and the client fetches
again and retries.

## Discovery

While live mode is on, each running app instance writes `<app data>/Live/<pid>.json` and deletes it on exit.
`<app data>` is the folder the apps already use for scripts and recovery files:

| Platform | `<app data>` |
| --- | --- |
| macOS | `~/Library/Application Support/Pixmith` |
| Windows | `%APPDATA%\Pixmith` |
| Linux | `$XDG_DATA_HOME/pixmith`, else `~/.local/share/pixmith` |

When `PIXMITH_SUPPORT_DIR` is set, it replaces the platform folder (`$PIXMITH_SUPPORT_DIR/Pixmith`, lower-case
`pixmith` on Linux). Tests use this so they never touch the real folder.

The file is UTF-8 JSON, readable only by the user (mode 0600 on macOS and Linux):

```json
{"protocol": 1, "pid": 4242, "port": 51234, "token": "9f86d081884c7d659a2feaa0c55ad015",
 "app": "Pixmith", "version": "0.5.0", "front_end": "swift", "platform": "macos", "started": 1729760000}
```

- `front_end` is `swift` or `egui`; `platform` is `macos`, `windows` or `linux`.
- `token` is 128 random bits as lower-case hex. A new token is made at every launch.
- A client ignores, and may delete, files whose `pid` is no longer running or whose port refuses the connection.
- With several instances running, a client picks the newest `started` unless told otherwise.

## Transport

- TCP on `127.0.0.1:<port>`. The app binds only to the loopback address, on a port the OS picks.
- Messages are JSON-RPC 2.0 objects, one per line (UTF-8, `\n`-terminated, no embedded newlines). A message may be
  up to 64 MiB.
- Requests are answered in order. Notifications (requests without an `id`) are not used in protocol 1.
- The first request on a connection must be `hello`, with the token. Anything else first, or a wrong token, gets an
  error and the connection is closed. This also keeps web pages out: a browser cannot speak this protocol, and it
  cannot read the token.

## Methods

Document ids are strings chosen by the app and stable while the tab is open (for example `"doc-3"`).
`revision` is an integer per document that goes up whenever the document changes, whether the user or a client
changed it.

| Method | Params | Result |
| --- | --- | --- |
| `hello` | `{"token", "client": "pixmith-mcp", "client_version", "protocol": 1}` | `{"app", "version", "front_end", "platform", "protocol": 1}` |
| `documents.list` | `{}` | `{"documents": [Doc…], "active": "doc-3" or null}` |
| `documents.get` | `{"id"}` | `{"document": Doc, "pxmt": "<base64>"}` |
| `documents.apply` | `{"id", "base_revision", "pxmt": "<base64>", "label", "layer"?, "frame"?}` | `{"document": Doc}` |
| `documents.new` | `{"width", "height", "name"?}` | `{"document": Doc}`, opened in a new tab and made active |
| `documents.open` | `{"path"}` | `{"document": Doc}`, or the already open tab of that file |
| `documents.save` | `{"id", "path"?}` | `{"document": Doc}`, saved to `path` or to its own file; `.pxmt` / `.aseprite` by extension |
| `documents.activate` | `{"id"}` | `{"document": Doc}` |
| `documents.close` | `{"id", "discard": false}` | `{}`; refused with `-32011` when the document has unsaved changes and `discard` is false |
| `view.get` | `{}` | `{"document": id or null, "layer", "frame", "zoom", "tool"}` |
| `view.set` | `{"id"?, "layer"?, "frame"?}` | the same shape as `view.get` |

`Doc` is:

```json
{"id": "doc-3", "name": "hero.pxmt", "path": "/Users/me/game/hero.pxmt", "width": 32, "height": 32,
 "frames": 4, "layers": 3, "revision": 17, "dirty": true, "active": true, "layer": 2, "frame": 0}
```

- `path` is null for an unsaved document.
- `layer` and `frame` are the app's current layer and frame for that document, 0-based like the core.

### `documents.apply`

1. If `base_revision` differs from the document's current revision, the request fails with `-32009`
   (stale revision). The document is left unchanged.
2. Otherwise the app replaces the sprite with the decoded bytes. Both `.pxmt` and `.aseprite` bytes are accepted.
   The replacement is **one undo step** named `AI: <label>`. Selection, zoom and scroll are kept, except that the
   selection is cleared when the canvas size changed.
3. When `layer` or `frame` is given, the app makes it current, clamped to the new document.
4. The document becomes dirty and its revision goes up. The result carries the new `Doc`.

Applies run on the app's UI thread between frames. A stroke the user is drawing in the middle of an apply is
finished first. If a modal sheet is open (for example a save prompt), the app may refuse with `-32010` (busy), and
the client retries a little later.

## Errors

| Code | Meaning |
| --- | --- |
| `-32700`, `-32600`, `-32601`, `-32602` | Standard JSON-RPC errors: parse error, invalid request, unknown method, bad params |
| `-32001` | Unauthorized: no `hello` yet, or a wrong token |
| `-32004` | Unknown document id |
| `-32009` | Stale revision. Fetch again with `documents.get` and retry. |
| `-32010` | Busy: a modal sheet is open or a stroke is in progress |
| `-32011` | Unsaved changes: `documents.close` without `discard` |
| `-32000` | Anything else. `message` carries the core's error text, for example a file that cannot be read. |

## Core support

The apps encode and apply documents through two core calls, declared in `app/Sources/PixmithCore/pixmith_live.h`
and also available to Rust callers:

- `pixmith_encode_document(doc, &len) -> bytes` gives the document as `.pxmt` bytes. Free them with
  `pixmith_bytes_free`.
- `pixmith_apply_document(doc, bytes, len, label)` decodes `.pxmt` or `.aseprite` bytes and swaps them in as one
  undo step named `label`.

## User control

- Preferences have a switch, **Allow AI agents to control Pixmith (Live mode)**, which is on by default. Turning it
  off closes the port and removes the discovery file.
- While a client is connected, the title row shows a small "AI connected" chip. Its menu lists the client and offers
  **Disconnect**.
