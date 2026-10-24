# Pixmith Lua scripting reference

Scripts are plain `.lua` files. Put them in `~/Library/Application Support/Pixmith/Scripts/` and run
them from **File > Scripts**, or pick any file with **Run Script…**. **Install Example Scripts**
copies the examples from `scripts/lua-examples/` into that folder.

A script runs on the active document, as **one undo step** (sprite and selection together). If it
raises an error the document is left exactly as it was. `app.alert` messages and `print` output
are shown in a dialog when the script ends.

## Conventions

| Thing | Numbering |
| --- | --- |
| Pixel coordinates (`x`, `y`) | 0-based, origin top-left |
| Palette indices | 0-based |
| Layers, frames, tags, cels | 1-based, layer 1 is the bottom of the stack |
| Colors | integers `0xRRGGBBAA` (`0xFF0000FF` is opaque red, `0` is transparent) |
| Blend modes | 0-18, see `app.blend` |

Optional `layer` / `frame` arguments default to the layer and frame that were active when the
script started (`sprite.activeLayer`, `sprite.activeFrame`).

**Handles** (`sprite.layers[i]`, `sprite.frames[i]`, `sprite.tags[i]`, `sprite:cel(l, f)`) refer to
a position, not to an object. Calls that delete or move layers, frames or tags invalidate the
handles you obtained before; using one afterwards raises a "stale handle" error. Fetch it again
from `sprite`. `deleteLayer`, `moveLayer`, `moveLayerIntoGroup`, `moveLayerOutOfGroup`,
`deleteFrame`, `duplicateFrame`, `deleteTag` / `tag:delete()` and `deleteSlice` / `slice:delete()`
invalidate; creating layers, groups, tags, slices or appending frames does not. Tilesets are
addressed by position too, and tile indices shift when a tile is deleted.

Errors raised by API calls can be caught with `pcall`; the error value is a string.

## Sandbox

Available: the base functions, `table`, `string`, `utf8`, `math`, `coroutine`. Not available:
`os`, `io`, `package`, `require`, `debug`, `dofile`, `loadfile`. `load` accepts text chunks only
(no bytecode). A run is stopped with an error after 300 million VM instructions, 10 seconds or
128 MB of Lua memory, whichever comes first. One very expensive `string.find`/`gsub` pattern is a
single VM instruction and cannot be interrupted, so avoid catastrophic patterns.

Scripts cannot grow the document without bound either: at most 1024 layers, 4096 frames, 4096
tags, 4096 slices and 64 MB of tile pixels per tileset.

## `sprite`

### Properties (read-only)

| Name | |
| --- | --- |
| `width`, `height` | canvas size |
| `layerCount`, `frameCount`, `paletteSize` | counts |
| `colorMode` | `"rgba"`, `"grayscale"` or `"indexed"` |
| `activeLayer`, `activeFrame` | 1-based, as chosen in the editor when the script started |
| `layers`, `frames`, `tags` | lists of handles, snapshots taken at access time |
| `selection` | selection handle, see below |

### Pixels

- `sprite:getPixel(x, y [, layer [, frame]])` returns a color; `0` outside the canvas or for empty cels.
- `sprite:setPixel(x, y, color [, layer [, frame]])` replaces a pixel (a transparent color erases).
  Out-of-canvas coordinates are ignored. The color is snapped to the sprite's color mode.
- `sprite:getPixels(layer, frame [, x, y, w, h])` returns a flat, row-major table of colors, 1-based
  (`px[(row * w) + col + 1]`). The rectangle defaults to the whole canvas.
- `sprite:setPixels(layer, frame, pixels [, x, y, w, h])` writes such a table back (replacing, not
  blending). The table must hold exactly `w * h` colors. This is the fast path for whole-cel work.
  Both calls move at most 4,194,304 pixels (2048 x 2048) at once and raise an error beyond that
  or when the rectangle leaves the canvas.
- `sprite:fillSelection(color [, layer [, frame]])` fills the selected pixels of a cel and returns how
  many were filled (0 when nothing is selected).

Scripts ignore the selection for `setPixel`/`setPixels`; only `fillSelection` uses it. On tilemap
layers, pixel writes edit the shared tile images, exactly like drawing does. Editing a cel that is
linked to other frames edits all of them, immediately.

In Indexed and Grayscale sprites every color written is snapped to the palette or to gray, and
cels the script changed are snapped once more when the run ends.

### Layers

- `sprite:newLayer([name])` adds a layer on top of the stack (never inside a group) and returns it.
- `sprite:newGroup([name])` adds an empty group on top.
- `sprite:deleteLayer(i)` deletes a layer and, for a group, everything in it. The last drawable layer
  cannot be deleted.
- `sprite:moveLayer(i, to)` moves layer `i` (with its children) one step at a time, like dragging
  it in the layer list, until it reaches position `to`, and returns the position it ended at.
  It stops early when it cannot move further.
- `sprite:moveLayerIntoGroup(i, group)` moves layer `i` to the top of a group and returns its new
  position. It fails for non-groups, for a group's own subtree and when the group already is the parent.
- `sprite:moveLayerOutOfGroup(i)` moves a layer out of its group to just above it (new position returned).

Layer handle fields:

| Field | |
| --- | --- |
| `name`, `visible`, `opacity` (0-255), `blendMode` (0-18) | read/write |
| `isGroup`, `isTilemap`, `index`, `level` | read-only (`level` is the nesting depth, 0 at the top) |
| `parent` | the containing group, or `nil` |
| `children` | direct children of a group (bottom to top); empty for other layers |
| `isVisible` | read-only: shown on the canvas, i.e. `visible` and every parent group `visible` |
| `tileset` | the tileset of a tilemap layer, or `nil` |
| `userData` | text and color attached to the layer (see User data) |

Tilemap layers also have `layer:getTile(x, y [, frame])` and
`layer:setTile(x, y, index [, flipX, flipY, diagonal [, frame]])`; see Tilemaps.

### Frames

- `sprite:newFrame([copy])` appends a frame after the last one (empty, or a copy of it when `copy` is true).
- `sprite:duplicateFrame(i)` inserts a copy of frame `i` right after it and returns it.
- `sprite:deleteFrame(i)` deletes a frame; tags are adjusted. The last frame cannot be deleted.

Frame handle fields: `duration` (milliseconds, read/write), `index`.

### Tags

- `sprite:newTag(name, from, to)` adds a tag over frames `from..to` (swapped ends are put in order) and returns it.
- `sprite:deleteTag(i)`, or `tag:delete()`.

Tag handle fields: `name`, `from`, `to` (frame numbers, kept in order), `direction` (see
`app.direction`), `repeatCount` (0 = forever), `color` (`0xRRGGBBFF`) are read/write; `index` is
read-only.

### Cels

- `sprite:cel(layer, frame)` returns a cel handle (groups have no cels).
- `sprite:clearCel(layer, frame)` removes the image of a cel; returns whether anything was removed.

Cel handle: `opacity` (0-255, read/write, the cel must not be empty), `isEmpty` (no cel or fully
transparent), `linked` (shares its image with other frames), `linkedFrames` (list of the other
frame numbers that share it), `layer`, `frame`, and `cel:unlink()` which gives the cel its own copy.

Linked frames are one image. A script that processes every frame of a linked cel would process
that image several times; use `linkedFrames` to handle it only from its first frame (see
`flip-all-frames.lua`).

### Tilemaps

A tilemap layer shows a grid of tiles taken from a tileset. **Tile indices are 0-based and tile 0 is
the empty tile.** Pixels written to a tilemap layer with `setPixel`/`setPixels` edit the shared tile
images (all cells showing a tile change), exactly like drawing does.

`sprite.tilesets` lists the tilesets (1-based). Tileset fields: `name` (read/write), `tileWidth`,
`tileHeight`, `count` (including the empty tile), `index`. Methods:

- `ts:getTilePixels(t)` returns the tile as a flat row-major table of `tileWidth * tileHeight` colors.
- `ts:setTilePixels(t, pixels)` replaces a tile image (not tile 0); every cel showing it is redrawn.
- `ts:addTile([pixels])` appends a tile (empty, or with the given pixels) and returns its index.
- `ts:duplicateTile(t)` appends a copy and returns its index.
- `ts:deleteTile(t)` deletes a tile (not tile 0). Cells showing it become empty; higher indices shift down.

`layer:getTile(x, y [, frame])` returns `index, flipX, flipY, diagonal` for the grid cell `(x, y)`
(the cell whose top-left pixel is `(x * tileWidth, y * tileHeight)`; cells outside the canvas read
as empty). `layer:setTile(x, y, index [, flipX, flipY, diagonal [, frame]])` places a tile; index 0
clears the cell. The layer must be a tilemap layer. The diagonal flip only applies to square tiles.

### Slices

- `sprite.slices` lists the slices (1-based); `sprite:newSlice(name, x, y, w, h)` adds one;
  `sprite:deleteSlice(i)` or `slice:delete()` removes one.
- Fields: `name` (read/write), `index`, `hasCenter`, `hasPivot`, `userData`.
- Geometry is per frame: a value set on frame N applies from N until the next one set. `frame`
  defaults to the active frame.
  - `slice:getBounds([frame])` returns `{x=, y=, w=, h=}`; `slice:setBounds(x, y, w, h [, frame])`.
  - `slice:getCenter([frame])` (the 9-patch center, `nil` without one), `slice:setCenter(x, y, w, h [, frame])`,
    `slice:clearCenter()`.
  - `slice:getPivot([frame])` returns `x, y` (`nil, nil` without a pivot), `slice:setPivot(x, y [, frame])`,
    `slice:clearPivot()`.

### User data

`sprite.userData`, `layer.userData`, `cel.userData`, `tag.userData` and `slice.userData` give access
to the text and color Aseprite attaches to those objects: `ud.text` (string or `nil`) and
`ud.color` (`0xRRGGBBAA` or `nil`), both read/write; assign `nil` (or `""` for the text) to clear.
Writing user data to an empty cel creates the cel.

### Selection

`sprite.selection` is a handle to the document's selection. Changes become part of the script's
undo step.

- `isEmpty`, `bounds` (`{x=, y=, w=, h=}` or `nil`) are read-only.
- `selection:contains(x, y)`.
- `selection:selectRect(x, y, w, h [, mode])` where `mode` is `"replace"` (default), `"add"` or
  `"subtract"` (0, 1, 2 also work). The rectangle is clipped to the canvas.
- `selection:selectAll()`, `selection:deselect()`, `selection:invert()`.

### Palette

- `sprite:getPaletteColor(i)`, `sprite:setPaletteColor(i, color)` (index `paletteSize` appends a
  color; in Indexed mode pixels using the old color follow), `sprite:addPaletteColor(color)`
  (returns the index, reusing an existing identical color).

## `app`

- `app.alert(message)` shows `message` when the script ends. `app.alert{ title=, text=, buttons={...} }`
  shows a modal message at once and returns the number of the pressed button (see Dialogs).
- `app.sprite` is the same as the global `sprite` (`nil` in plugin `init`/`exit`).
- `app.isUIAvailable` is `false` for headless runs (command line), `true` inside the app.
- `app.fgColor`, `app.bgColor` (`0xRRGGBBAA`): the drawing colors, read and write.
- `app.events:on(name, fn)` / `app.events:off(listener)` and `sprite.events:on("change", fn)`:
  see Events.
- `app.undoLabel(text)` names the undo step (default "Script").
- `app.version` is Pixmith's version string; `app.apiVersion` is a number that grows with the API
  (this reference describes version 3), for scripts that need a feature check.
- `app.activeLayer` and `app.activeFrame` (1-based) read the current defaults of the pixel calls.
  Assigning one changes those defaults immediately and selects that layer or frame in the editor
  when the script ends (a successful run only). A group cannot be assigned as the active layer.
- `app.blend`: `Normal` 0, `Multiply`, `Screen`, `Overlay`, `Darken`, `Lighten`, `ColorDodge`,
  `ColorBurn`, `HardLight`, `SoftLight`, `Difference`, `Exclusion`, `Hue`, `Saturation`, `Color`,
  `Luminosity`, `Addition`, `Subtract`, `Divide` (18).
- `app.direction`: `Forward` 0, `Reverse` 1, `PingPong` 2, `PingPongReverse` 3.
- `print(...)` works like Lua's, but goes to the dialog shown when the script ends.

## Dialogs, events and plugins

`Dialog` builds windows the app renders (labels, buttons, checks, radios, entries, numbers, sliders,
colors, comboboxes, files, shades, separators), modal with `dlg:show{wait=true}` or non-modal with
`wait=false`; `dlg.data`, `dlg:modify{}` and `onclick` / `onchange` / `onrelease` / `onclose` callbacks
work as in Aseprite. `app.events` (`sitechange`, `fgcolorchange`, `bgcolorchange`, `beforecommand`,
`aftercommand`) and `sprite.events` (`change`, `filenamechange`) call your functions when the editor
reports them. A plugin script defines `init(plugin)` and optionally `exit(plugin)`; `plugin:newCommand{}`
adds a command to File > Scripts and `plugin.preferences` is saved between sessions. The complete
reference, the message format between the core and the app, and the threading rules are in
[LUA_DIALOG.md](LUA_DIALOG.md). Everything stays inside the sandbox above: callbacks run on the script's
own Lua state with the same instruction, time and memory limits per transaction, and can be cancelled.

Not supported (calling them raises an error): `Dialog:canvas`, `image`, `tab`/`endtabs`, `menu`,
`ev:stopPropagation()` and canceling `beforecommand`, `plugin:newMenuGroup`.

## `color`

- `color.rgba(r, g, b [, a])` packs 0-255 channels (alpha defaults to 255).
- `color.split(c)` returns `r, g, b, a`.

## Examples

`scripts/lua-examples/` contains runnable examples: `outline-layer.lua`, `color-ramp.lua`,
`flip-all-frames.lua`, `palette-swap.lua`, `tag-every-4-frames.lua` and `palette-ramp-dialog.lua`
(a dialog with a live ramp preview). `plugin-hello.lua` is a plugin example (a command with a dialog,
preferences and an event); it is loaded as a plugin, not run from File > Scripts.

## Embedding notes

`pixmith_run_script_at` returns the alert and `print` lines joined with `\n`. When a successful script
assigned `app.activeLayer` or `app.activeFrame`, one more line is appended:
`\u{1}PIXMITH-SELECT layer=<n> frame=<n>` (1-based, after a `U+0001` control character, which is
stripped from script output so scripts cannot fake it). The editor selects that layer and frame and
removes the line before showing the messages.
