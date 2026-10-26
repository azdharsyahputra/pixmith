# Architecture

Pixmith is two layers: a platform-independent Rust library that owns the document and every pixel operation behind a C ABI, and a native front end that shows it and turns input into calls on the library. This repository has two front ends: a SwiftUI/AppKit app for macOS (`app/`) and a cross-platform egui app for Windows, Linux and macOS (`desktop/`); both use the same entry points. Front ends for other platforms would sit next to them.

```
core/  (Rust, crate pixmith_core)          app/  (Swift package, macOS 13)
  src/document/   Sprite, Editor           Sources/PixmithCore/*.h   C declarations + module.modulemap
  src/formats/    .aseprite, PNG, GIF      Sources/Pixmith/<area>/    SpriteModel, canvas, panels, menus
  src/*/          features + C ABI    <-->   (links libpixmith_core.a)
  src/ffi/        panic guard, errors
  src/bin/pixmith-cli.rs  command line
```

`./build.sh` builds the static library (`cargo build --release`), builds the Swift package (which links `core/target/release/libpixmith_core.a`) and assembles `build/Pixmith.app`. `scripts/package.sh` does the same per architecture and packs a dmg.

## Front ends

| | `app/` (macOS) | `desktop/` (Windows, Linux, macOS) |
| --- | --- | --- |
| Language and UI | Swift, SwiftUI and AppKit | Rust, egui through eframe (wgpu), Phosphor icons, `rfd` for native file dialogs |
| How it reaches the core | C headers in `Sources/PixmithCore`, linking `libpixmith_core.a` | `pixmith-core` as a Cargo path dependency, calling the same `pixmith_*` functions |
| Document handle | `SpriteModel` wraps `OpaquePointer` | `Doc` (`desktop/src/doc.rs`) owns the `*mut Editor` and frees it on drop |
| Design tokens | `DesignSystem/DesignSystem.swift` | `desktop/src/theme.rs`, same values |
| Build | `./build.sh` | `cd desktop && cargo build --release` |
| Release artifact | `Pixmith-<version>.dmg` | `Pixmith-<version>-windows-x64.zip`, `Pixmith-<version>-linux-x64.tar.gz` |

The two apps are meant to be indistinguishable apart from the platform's native file dialogs and fonts. The desktop app keeps the same threading rules as the macOS app: one thread owns a `Doc`, and `unsafe` stays inside `doc.rs` and `calls/`, which expose safe methods grouped by the window region that uses them. `desktop/tests/screens.rs` renders the window headlessly so it can be compared with the macOS screenshots in `docs/design/screens/`. See [`desktop/README.md`](../desktop/README.md) for the build instructions and the current state.

## Core (`core/src`)

`lib.rs` carries the crate docs and the module tree; the modules fall into these groups:

| Directory | Holds |
| --- | --- |
| `document/` | The sprite model and `Editor` (`mod.rs`), layers, slices, tilemaps, color modes. |
| `history/` | Modified-state tracking (`mod.rs`), the compact undo store (`store.rs`), pixel buffer reuse (`buffer_pool.rs`). |
| `render/` | Partial rendering for viewers (`view.rs`) and dirty-region tracking (`dirty.rs`). |
| `paint/` | Tools that change pixels: shapes, ink, curves, font, filters, transform, selection. |
| `palette/` | Palette editing, built-in presets, palette files. |
| `colorpick.rs` | Color-picker math for the inspector's selectors: HSV/HSL conversions, color-wheel geometry and bitmaps, harmony hues, tangent-space normal encoding. Pure functions; C ABI `pixmith_cp_*` in `pixmith_colorpick.h`, called directly by the egui app. |
| `formats/` | `.pxmt`, `.aseprite`, PNG, GIF, APNG, sprite sheets, JSON, export planning, durable file writes. |
| `extensions/` | `.pixmith-extension` packages (scripts, palettes, themes): a strict zip reader, manifest and theme parsing, install, list, enable and uninstall into `<app data>/Extensions/` (see [`EXTENSIONS.md`](EXTENSIONS.md)). |
| `project.rs` | Project folders of canvases (`.pxmt` plus a `.pixmith` folder) and their C ABI. |
| `gamedev/` | Auto-tiling, tweening, the Godot 4 exporter. |
| `ffi/` | The C ABI plumbing and the document-level entry points. |
| `scripting.rs`, `jobs.rs` | The Lua host and snapshots for background work. |

### Document model

- `document/mod.rs`: `Sprite` holds the canvas size, layers (bottom to top, groups are layers with a nesting level), per-frame durations, `cels[frame][layer]` as `Option<Arc<Cel>>`, tags and the palette. A `Cel` is a full-canvas straight-alpha RGBA buffer, so every tool and the compositor work on the same representation regardless of file color mode. `Arc` lets undo snapshots share unchanged buffers.
- `Editor` wraps a `Sprite` with the undo and redo stacks of whole-sprite snapshots. `checkpoint()` must be called once before any change that should be undoable. It also carries the selection state (`paint/selection.rs`), the saved-state tracker (`history/mod.rs`), a label per undo step and a transaction counter: inside `begin_transaction` / `end_transaction` only the first checkpoint is recorded, so a multi-part operation undoes as one step.
- `Sprite` also holds the tilesets, the color mode and transparent index, linked-cel bookkeeping, and `extra` (the sprite's user data and its slices).
- `history/mod.rs`: gives every document state an id parallel to the undo and redo stacks. "Modified" means the current id differs from the id recorded at the last save, so undoing back to the saved state is clean. Edits that skip the undo stack call `pixmith_touch`. Its C ABI also exposes transactions, step labels and the history list (`pixmith_history_*`) used by the History panel.
- `document/spritedata.rs`: the color profile (`ColorProfile`: none, sRGB or an embedded ICC profile, kept as read and written back by `ase.rs`, never applied to pixels), cel z-indexes (`Cel::z_index`, applied by `layers::drawing_order`) and reference layers (layer flag 0x40; excluded from `Sprite::render` and so from every export, drawn only by `pixmith_render` / `pixmith_render_rect` at `Editor::reference_display` percent; a locked one refuses `cel_mut`), with their C ABI (`pixmith_spritedata.h`).
- `document/slices.rs`: slices (bounds per frame, 9-patch center, pivot) and user data (text, color, raw property bytes) with their .aseprite chunks.

### File formats

- `formats/pxmt.rs`: Pixmith's own `.pxmt` document format (specification: [`PXMT_FORMAT.md`](PXMT_FORMAT.md)): magic, version and CRC-checked chunks `META` (JSON facts: size, frame and layer counts, timestamps), `THMB` (PNG thumbnail, at most 256 px) and `SPRT` (the sprite as an `.aseprite` file produced by `ase.rs`, so every feature round-trips; tweens, terrains and Godot export settings already live in user data). `pixmith_open` recognizes `.pxmt` by its magic; `pixmith_save` writes it when the path ends in `.pxmt`; `pixmith_save_aseprite` always writes `.aseprite`; `pixmith_file_thumbnail` previews any document file. Readers can fetch `META` and `THMB` without decoding `SPRT`.
- `formats/ase.rs`: `.aseprite` reader and writer, written from the public format specification. Reads RGBA, grayscale and indexed files, raw, compressed and linked cels, layers and groups, tags, new and old palettes, tilemaps, slices and user data; unknown chunk types are skipped. Writes cels cropped to their content and links cels that share a buffer.
- `document/color_mode.rs`: color mode (RGBA, grayscale, indexed) as a constraint on the pixel values; the writer encodes pixels according to the mode.
- `document/tilemap.rs`: tilesets and tilemap layers; the rendered pixels stay in `Cel::pixels` and edits are written back into the shared tile images.
- `palette/io.rs`: GIMP `.gpl` and `.hex` palettes, sorting.
- `formats/export.rs`: single-frame PNG and the simple sheet used by the original menu item.
- `formats/gif.rs`: animated GIF, numbered PNG sequences, atomic file writes shared with the other exporters.
- `formats/animation.rs`: animated PNG export (the `png` crate's APNG support) and GIF import, which composes each frame onto the logical screen following the GIF disposal methods.
- `formats/sheet.rs`: sprite sheet layouts and Aseprite-style JSON metadata.
- `formats/json.rs`: a small JSON parser and serializer (no dependencies) that keeps member order.
- `formats/sheet_import.rs`: sprite sheet PNG plus Aseprite JSON (array or hash format) into a sprite: frame rectangles, durations, trimmed frames, upscaled sheets, tags, slices.
- `formats/import.rs`: PNG decoding, sprite from PNG, sprite from sheet cells, PNG as a new layer.

### Projects

`project.rs` implements the project browser's file operations (declared for Swift in `pixmith_project.h`; the egui app calls the Rust functions directly). A project is a folder of canvases (`*.pxmt`, plus `*.aseprite` / `*.ase` found there) with a `.pixmith/` folder holding `project.json` (name, creation time, canvas order; unknown members are preserved) and `trash/`. `scan` lists canvases in saved order, then new files by name, reading only the `META` chunk of a `.pxmt` or the leading headers of an `.aseprite`; unreadable files are listed with an error. `new_canvas`, `duplicate_canvas`, `rename_canvas` (rejects empty names and `/ \ :` and the other characters Windows forbids; keeps the extension; refuses taken names, compared case-insensitively), `trash_canvas` / `restore_canvas` (project-local `.pixmith/trash/`, names made unique) and `import` (`.aseprite` / `.ase` / `.png` to `.pxmt`) keep the saved order in step. All writes are atomic (`durable`).

### Editing features

- `paint/selection.rs`: selection mask (marquee, wand, lasso, polygon, set operations, modify), selection-aware drawing, move and transform, clipboard and floating paste, crop and canvas resize.
- `paint/shapes.rs`: pencil strokes with pixel-perfect mode and pen pressure, line, rectangle, ellipse, spray, flood fill, mirroring. The painter it defines is shared by the tools below, so ink modes, brushes and symmetry apply to them as well.
- `paint/retouch.rs`: the Blur and Jumble brush tools (`pixmith_retouch_begin/to/size/end_doc`, declared in `pixmith_shapes.h`): a stroke of dabs that average (alpha-weighted 3x3, mixed by strength) or randomly swap the pixels under the global brush, with mirroring, selection clipping and a seeded RNG.
- `paint/curves.rs`: Bezier curves and polygons, outline or filled.
- `paint/font.rs`: the built-in 5x8 bitmap font and text stamping.
- `paint/transform.rs`: free transform sessions (scale, rotate, skew, move with a live preview, RotSprite-style resampling).
- `paint/ink.rs`: ink modes, custom brushes, gradients and dither patterns.
- `document/layers.rs`: layer hierarchy operations, blend modes, tags, frame reordering, linked cels.
- `paint/filters.rs`: color adjustments, color curves, outline, drop shadow, blur, sharpen, sprite scaling.
- `jobs.rs`: document snapshots for background work (see Threading) and applying a job's result.
- `palette/edit.rs`: palette reordering, insertion, removal, ramps and entry edits, keeping indexed sprites consistent; `palette/presets.rs`: built-in palettes.
- `scripting.rs`: the embedded Lua host (API in `docs/LUA_API.md`).
- `document/iso.rs`, `document/isoext.rs`: isometric grid math (cell <-> pixel, draw order) and how isometric tilemap data is stored in files; rendering and editing live in `document/tilemap.rs` (see `docs/PXMT_FORMAT.md`).
- `gamedev/`: `autotile.rs` (terrain auto-tiling for tilemap layers), `tween.rs` (keyframed layer transforms) and `godot/` (Godot 4 SpriteFrames and TileSet export); see `docs/SPEC-game.md`.

### C ABI

Every feature module exposes its functions as `#[no_mangle] pub unsafe extern "C" fn pixmith_*` next to the Rust implementation; the entry points for the document as a whole (open, save, render, layer and frame basics, undo) are in `ffi/document.rs`. Conventions:

- A document is an opaque `*mut Editor` (`PixmithDoc` on the C side); every function tolerates null pointers and out-of-range indices.
- Colors are `0xRRGGBBAA` `u32`; indices are `u32`; booleans report success.
- Failing functions record a message that Swift reads with `pixmith_last_error()` (`ffi/mod.rs`). Strings returned to the caller are released with `pixmith_string_free`.
- File writes go through `formats/durable.rs`: a temporary file in the same directory, `fsync` of the file and then of the directory, an atomic rename, the original's permissions kept, symbolic links written through, and stale `*.pixmith-tmp` files cleaned up. A failed or interrupted save leaves the original untouched.
- The release profile unwinds on panic, and every entry point body runs inside `crate::ffi::guarded(name, doc, || ...)` (inserted mechanically; a test fails if one is missing). A panic returns false/0/NULL, sets `pixmith_last_error()` to "internal error: ...", rewinds the document to its last undo step (the undone work stays on the redo stack) and bumps `pixmith_panic_count()`; `pixmith_last_panic()` describes it so the app can warn the user and offer to save a copy (`pixmith_safety.h`).

Each module's declarations live in a header under `app/Sources/PixmithCore/` (`pixmith.h` for `ffi/document.rs`, `pixmith_selection.h`, `pixmith_shapes.h`, `pixmith_ink.h`, `pixmith_layers.h`, `pixmith_filters.h`, `pixmith_curves.h`, `pixmith_transform.h`, `pixmith_warp.h` (the pixel-art warp), `pixmith_tilemap.h`, `pixmith_colormode.h`, `pixmith_colorpick.h` (pure color-picker math, no document), `pixmith_palette_edit.h`, `pixmith_slices.h`, `pixmith_io.h` for import and export, `pixmith_history.h`, `pixmith_project.h` for `.pxmt` saving, thumbnails and projects). `module.modulemap` lists them and links `pixmith_core`, so Swift sees them as `import PixmithCore`.
Each module's declarations live in a header under `app/Sources/PixmithCore/` (`pixmith.h` for `ffi/document.rs`, `pixmith_selection.h`, `pixmith_shapes.h`, `pixmith_ink.h`, `pixmith_layers.h`, `pixmith_filters.h`, `pixmith_trace.h` (background removal and trace to a pixel grid), `pixmith_curves.h`, `pixmith_transform.h`, `pixmith_tilemap.h`, `pixmith_colormode.h`, `pixmith_colorpick.h` (pure color-picker math, no document), `pixmith_palette_edit.h`, `pixmith_slices.h`, `pixmith_io.h` for import and export, `pixmith_history.h`, `pixmith_project.h` for `.pxmt` saving, thumbnails and projects). `module.modulemap` lists them and links `pixmith_core`, so Swift sees them as `import PixmithCore`.

### Live mode

`ffi/live.rs` is the core's half of Pixmith Live (`docs/LIVE_PROTOCOL.md`): `pixmith_encode_document` gives a document as `.pxmt` bytes, `pixmith_apply_document` (and `pixmith_apply_document_selection`) swaps decoded `.pxmt` or `.aseprite` bytes in as one labelled undo step through the ordinary `checkpoint` and `label_last`, validating before it touches anything, so bad bytes leave the document as it was. `pixmith_live_selection` and `pixmith_state_id` let a front end report the selection and notice that a document changed. The network side belongs to each front end: `desktop/src/live/` (a `TcpListener` and one thread per connection) and `app/Sources/Pixmith/App/Live.swift` (BSD sockets, same structure). Both hand every request after `hello` to the UI thread, which answers it between frames, so documents are still only touched by their owning thread (see Threading below). Each document tab has a stable id and a revision that goes up whenever it changes (the tab's own revision counter combined with `pixmith_state_id`, so a change that forgot to bump the counter is still seen); `documents.apply` is refused with a stale-revision error when the client's base revision is not the current one.

### Threading

An `Editor` (`PixmithDoc`) is not `Sync`, and the core keeps some per-thread state (`pixmith_last_error`, the stroke and ink settings). The rules:

- **A document is used by one thread at a time.** The UI thread owns the documents of the open tabs. Never call into the same `PixmithDoc` from two threads at once, not even for read-only calls such as `pixmith_render`, `pixmith_save` or `pixmith_export_*` (they flush pending linked-cel edits and so write to the document).
- **Background work runs on a snapshot.** `pixmith_snapshot(doc)` (`jobs.rs`) copies the sprite and selection into an independent document (cel buffers are shared copy-on-write, so it is cheap) that a worker thread may use freely; release it with `pixmith_free`. Autosave, exports and Lua scripts work this way (`Autosaver`, `exportInBackground`, `runScript`).
- **Results come back through the main thread.** A job that changes its snapshot, like a script, is applied with `pixmith_job_apply(doc, snapshot)` on the main thread as one undo step. Imports create a brand new document on the worker and open it from the main thread.
- **Read errors where they were raised.** `pixmith_last_error()` is thread-local, so a worker reads it right after the failing call and passes the text to the main thread.
- **The UI is blocked while a job that must not be raced runs.** `Background.run` (`ProgressSheet.swift`) shows a modal sheet on the document window (with Cancel for scripts, via `pixmith_script_cancel`); menu commands are disabled while any sheet is open (`SheetState`).

### Command line

`src/bin/pixmith-cli.rs` is a second binary in the same crate, using the library directly (no FFI). It parses arguments with the standard library only and reads `.pxmt` and `.aseprite` alike (through `formats::pxmt::read_document`), converts between them (`convert <in> <out>`) and exports PNG, GIF, APNG, sprite sheets with JSON and PNG sequences. Integration tests are in `core/tests/cli.rs`.

## App (`app/Sources/Pixmith`)

`SpriteModel` (an `ObservableObject` singleton) wraps the `PixmithDoc` pointer. Pixel data never crosses into Swift except for rendering and the eyedropper; Swift keeps UI state (current tool, layer, frame, colors, zoom). After every mutation it calls `changed()`, which touches the document's state tracker and bumps `revision`, the value SwiftUI views and the canvas observe.

Sources are grouped by area; `Sources/PixmithMain/PixmithApp.swift` is the `@main` entry (`AppDelegate`, top-level menu commands) and everything else is the `PixmithKit` library, so the `pixmith-qa` runner can drive it.

| Folder | Files | Role |
| --- | --- | --- |
| `App/` | `ContentView`, `MainWindow`, `Documents`, `FileActions`, `Preferences`, `Shortcuts`, `ViewSettings`, `Scripts` | Window layout (title row, tool rail, canvas, timeline, inspector), document tabs, open/save/import/export dialogs, dirty tracking, autosave and crash recovery, preferences, the rebindable single-key shortcut registry, view-only settings, the Lua script folder and runner. |
| `Model/` | `SpriteModel`, `RenderCache`, `ChangeCoalescer` | The model around the Rust document (queries, editing calls, playback, the `Tool` enum), rendered frame buffers with dirty-rect updates, and `revision` publishing coalesced to one per display refresh. |
| `Canvas/` | `CanvasView`, `CanvasScroller`, `CanvasOverlays`, `Selection`, `FreeTransform`, `ShapeTools`, `SlicesView` | The AppKit canvas: drawing and mouse handling, the zooming scroll view, overlays (grid, selection, guides, tiling), selection tools and clipboard, free transform, curve/polygon/contour/text sessions, the slice tool. |
| `Tools/` | `ToolsPanel`, `TopBar`, `DrawingTools`, `InkTools` | The tool rail, the context bar with the active tool's options, colors, ink modes and brushes, the Drawing menu. |
| `Panels/` | `Inspector`, `ColorPanel`, `PaletteEditor`, `Timeline`, `TagsView`, `LayerTools`, `LayerDrag`, `HistoryPanel`, `TilemapView`, `AutotileView`, `TweenView`, `PreviewWindow`, `ViewOptionsWindow` | Inspector sections (Color, Palette, Layer, Cel, Tileset, Slices, Sprite), the timeline (layers column, cel grid, tags), layer dialogs and drag reordering, the History panel, tilemap and terrain (auto-tile) editing, the tween panel with dope sheet and Bezier editor, the preview and view options windows. |
| `Dialogs/` | `ExportOptionsSheet`, `GodotExport`, `GodotTilesetExport`, `ColorModeView`, `FiltersView`, `CurvesEditor`, `ProgressSheet` | File > Export…, Godot SpriteFrames and TileSet export, color mode, FX sheets and the curves editor, `Background.run` (work off the main thread behind a progress sheet). |
| `App/Live.swift` | `LiveServer`, `LiveHandler`, `LiveChip` | Pixmith Live: the loopback server, the main-thread request handler, discovery file and the "AI connected" chip. |
| `DesignSystem/` | `DesignSystem` | Design tokens (spacing, radii, type, adaptive colors) and the reusable controls every view is built from: `IconButton`, `ToolButton`, `DSChip`, `DSSegmented`, `DSPopup`, `DSNumberField`, `DSSlider`, `Swatch`, `InspectorSection`, `DSDialog`. |

### Dirty tracking and recovery

`isDirty` asks the core (`pixmith_is_dirty`). `Autosaver` (in `App/FileActions.swift`) runs a timer; while the document is dirty it saves a copy and a small JSON file with the original path to `~/Library/Application Support/Pixmith/Recovery/<uuid>.*`. The copy is removed on save, on "Don't Save", on a clean quit and when the document is replaced, so files found at launch come from a crash and are offered for restoring.

## Adding a feature

1. Put the logic in a new module under the fitting directory in `core/src` (for example `paint/`) with `#[cfg(test)]` tests and `pub mod` it in that directory's `mod.rs`.
2. Expose it with `#[no_mangle] extern "C"` functions in the same module, call `Editor::checkpoint()` once before undoable changes, and declare the functions in a new header that you add to `module.modulemap`.
3. Wrap the calls in a `SpriteModel` extension in a new Swift file, call `changed()` after mutations, and add UI and menu entries from that file.

## Tests

`cargo test` in `core/` runs the unit tests next to each module, `tests/cli.rs` (the command-line tool), `tests/pxmt.rs` (every fixture round-tripped through `.pxmt`, thumbnails, damaged and hostile files, the C entry points, `pixmith-cli convert`), `tests/project.rs` (project scan, order, new, duplicate, rename, trash, restore, import, the C ABI; temp directories only) and `tests/compat.rs`, which reads real `.aseprite` files written by Aseprite (fixtures and licenses in `tests/fixtures/aseprite/SOURCES.md`), compares renderings with the PNGs Aseprite exported, checks the facts the fixtures' source projects document, round-trips every file through the writer, runs the exporters and the CLI on them, and feeds damaged copies to the reader to make sure it never panics.
