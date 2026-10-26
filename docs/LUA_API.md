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

Aseprite scripts run largely unchanged: `Point`, `Size`, `Rectangle`, `Image`, `sprite:newCel`,
`sprite:newSlice(Rectangle)` and the rest of the Aseprite names exist; see [Geometry](#geometry-point-size-rectangle),
[Images](#images) and [Aseprite compatibility](#aseprite-compatibility).

## Sandbox

Available: the base functions, `table`, `string`, `utf8`, `math`, `coroutine`. Not available:
`os`, `io`, `package`, `require`, `debug`, `dofile`, `loadfile`. `load` accepts text chunks only
(no bytecode). A run is stopped with an error after 300 million VM instructions, 10 seconds or
128 MB of Lua memory, whichever comes first (hosts can raise these, see [Limits](#limits-for-embedders)).
One very expensive `string.find`/`gsub` pattern is a single VM instruction and cannot be interrupted,
so avoid catastrophic patterns.

**File access.** Scripts cannot read or write files, with one exception: `Image{ fromFile = "/absolute/path.png" }`
reads a PNG file. The path must be absolute; the file must be a regular file of at most 256 MB that starts
with the PNG signature and decodes to at most 8192 x 8192 pixels. Nothing else can be read, nothing can be
written (`Image:saveAs` raises an error), and the decoded pixels only ever end up in an `Image`.

Scripts cannot grow the document without bound either: at most 1024 layers, 4096 frames, 4096
tags, 4096 slices and 64 MB of tile pixels per tileset. Free-standing `Image`s (kept outside the Lua
heap) are limited to 67,108,864 pixels (the largest canvas) each and 512 MB alive at once.

## `sprite`

### Properties (read-only)

| Name | |
| --- | --- |
| `width`, `height` | canvas size |
| `layerCount`, `frameCount`, `paletteSize` | counts |
| `colorMode` | `"rgba"`, `"grayscale"` or `"indexed"` |
| `activeLayer`, `activeFrame` | 1-based, as chosen in the editor when the script started |
| `bounds`, `size` | `Rectangle(0, 0, width, height)` and `Size(width, height)` |
| `layers`, `frames`, `tags` | lists of handles, snapshots taken at access time |
| `cels` | every cel that has an image (frame by frame, bottom layer first) |
| `selection` | selection handle, see below |

### Pixels

- `sprite:getPixel(x, y [, layer [, frame]])` or `sprite:getPixel(point [, layer [, frame]])` returns a
  color; `0` outside the canvas or for empty cels.
- `sprite:setPixel(x, y, color [, layer [, frame]])` or `sprite:setPixel(point, color [, ...])` replaces a
  pixel (a transparent color erases). Out-of-canvas coordinates are ignored. The color is snapped to the
  sprite's color mode.
- `sprite:getPixels(layer, frame [, x, y, w, h])` (or `getPixels(layer, frame, rectangle)`) returns a flat,
  row-major table of colors, 1-based (`px[(row * w) + col + 1]`). The rectangle defaults to the whole canvas.
- `sprite:setPixels(layer, frame, pixels [, x, y, w, h])` (or `..., pixels, rectangle`) writes pixels back
  (replacing, not blending). `pixels` is
  - a flat table of exactly `w * h` colors (at most 4,194,304 pixels = 2048 x 2048 per call, and this
    form costs one VM step per pixel), or
  - **an `Image`** (the rectangle defaults to `0, 0` and the image's size), or
  - **a string of raw RGBA bytes**, row-major, exactly `w * h * 4` long (the rectangle defaults to the
    whole canvas).
  The Image and string forms run in Rust: a 1024 x 1024 canvas is a few VM instructions and a few
  milliseconds, and they may fill a canvas of any size.
  `layer` and `frame` may be numbers or layer / frame handles in all of these calls.
  An error is raised when the rectangle leaves the canvas or the sizes do not match.
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
| `isReference` | read/write: a reference layer is shown in the editor but never exported; turning it on also locks the layer against drawing. Groups cannot be reference layers |
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

- `sprite:cel(layer, frame)` returns a cel handle (groups have no cels); `layer` and `frame` may be numbers or
  handles. `layer:cel(frame)` is the Aseprite form and returns `nil` when the layer has no image in that frame;
  `layer.cels` lists the layer's cels.
- `sprite:clearCel(layer, frame)` removes the image of a cel; returns whether anything was removed.
  `sprite:deleteCel(cel)` / `sprite:deleteCel(layer, frame)` is the Aseprite spelling.
- `sprite:newCel(layer, frame [, image [, position]])` makes (or replaces) the cel of `layer`/`frame` and returns
  it. With an `Image` the cel shows that image with its top-left at `position` (a `Point`, default `0, 0`),
  clipped to the canvas, in one call; without one the cel is empty. See [Images](#images).

Cel handle: `opacity` (0-255, read/write, the cel must not be empty), `zIndex` (-32768..32767, read/write, the cel must not be empty: the cel draws as if its layer sat that many places higher, or lower when negative; the order is layer index plus z-index and ties go to the lower layer; linked cels share it), `isEmpty` (no cel or fully
transparent), `linked` (shares its image with other frames), `linkedFrames` (list of the other
frame numbers that share it), `layer`, `frame` (numbers; `frameNumber` is the same as `frame`),
and `cel:unlink()` which gives the cel its own copy. Aseprite's cel geometry:

- `cel.bounds` (read-only `Rectangle`) and `cel.position` (`Point`, read/write): Pixmith cels are canvas-sized
  buffers, so these describe the box around the cel's non-transparent pixels (`0,0,0,0` / `Point(0, 0)` for an
  empty cel). Assigning `cel.position` moves the pixels; whatever leaves the canvas is lost.
- `cel.image` (read/write): a **live view** of that box as an `Image` (`nil` for an empty cel): drawing on it
  draws on the cel. Assigning an `Image` replaces the cel's pixels with it, keeping the cel's position
  (`0, 0` for an empty cel).

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

- `sprite.slices` lists the slices (1-based). `sprite:newSlice(rectangle)` is the Aseprite form: the slice is
  named "Slice 1", "Slice 2", ... (assign `slice.name` to rename it); `sprite:newSlice()` covers the whole canvas.
  The Pixmith forms `sprite:newSlice(name, x, y, w, h)`, `sprite:newSlice(name, rectangle)` and
  `sprite:newSlice(x, y, w, h)` work too. `sprite:deleteSlice(i)` or `slice:delete()` removes one.
- Fields: `name` (read/write), `index`, `hasCenter`, `hasPivot`, `userData`, and as in Aseprite
  `bounds` (a `Rectangle`), `center` (a `Rectangle` relative to `bounds`, or `nil` for no 9-patch) and
  `pivot` (a `Point`, or `nil`), all read/write. These three are the values of the first frame; assign `nil`
  to `center` / `pivot` to remove them.
- Geometry is per frame: a value set on frame N applies from N until the next one set. `frame`
  defaults to the active frame, and may be a number or a frame handle.
  - `slice:getBounds([frame])` returns a `Rectangle`; `slice:setBounds(rectangle [, frame])` or
    `slice:setBounds(x, y, w, h [, frame])`.
  - `slice:getCenter([frame])` (the 9-patch center, `nil` without one), `slice:setCenter(rectangle | x, y, w, h [, frame])`,
    `slice:clearCenter()`.
  - `slice:getPivot([frame])` returns `x, y` (`nil, nil` without a pivot), `slice:setPivot(point | x, y [, frame])`,
    `slice:clearPivot()`.

### User data

`sprite.userData`, `layer.userData`, `cel.userData`, `tag.userData` and `slice.userData` give access
to the text and color Aseprite attaches to those objects: `ud.text` (string or `nil`) and
`ud.color` (`0xRRGGBBAA` or `nil`), both read/write; assign `nil` (or `""` for the text) to clear.
Writing user data to an empty cel creates the cel.

### Selection

`sprite.selection` is a handle to the document's selection. Changes become part of the script's
undo step.

- `isEmpty` and `bounds` (a `Rectangle`, or `nil` when empty) are read-only.
- `selection.origin`: the top-left of the selection as a `Point` (`Point(0, 0)` when empty). Assigning a
  `Point` moves the selection (pixels pushed off the canvas are dropped).
- `selection:contains(x, y)` or `selection:contains(point)`.
- `selection:selectRect(x, y, w, h [, mode])` or `selection:selectRect(rectangle [, mode])` where `mode` is
  `"replace"` (default), `"add"` or `"subtract"` (0, 1, 2 also work). The rectangle is clipped to the canvas.
- Aseprite's names for the same thing: `selection:select(rectangle)` (replace), `selection:add(rectangle)`,
  `selection:subtract(rectangle)` and `selection:intersect(rectangle)`.
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
  (this reference describes version 4: `Point` / `Size` / `Rectangle` / `Image` arrived in 4), for scripts that
  need a feature check.
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

### Events

```lua
local id = app.events:on("sitechange", function(ev) print(ev.layer, ev.frame) end)   -- returns a number
app.events:off(id)                  -- by the id, or by the function that was registered
sprite.events:on("change", function(ev) if ev.fromUndo then print("undo/redo") end end)
```

| Object | Event | `ev` fields |
| --- | --- | --- |
| `app.events` | `sitechange` | `layer`, `frame` (1-based) of the new site |
| `app.events` | `fgcolorchange`, `bgcolorchange` | `color` (`0xRRGGBBAA`) |
| `app.events` | `beforecommand`, `aftercommand` | `name`, the menu command (`"Undo"`, `"Flip Horizontal"`, ...) |
| `sprite.events` | `change` | `fromUndo` (true for undo and redo) |
| `sprite.events` | `filenamechange` | |

- Listeners run in the order they were registered, on the session's own Lua state, inside a transaction:
  what a handler does to `sprite` is one undo step, and the usual instruction, time and memory limits
  apply to each event separately. Inside a handler `app.activeLayer` / `app.activeFrame` are the site the
  event reports.
- `sprite.events` listeners belong to the document that was active when they were registered and only fire
  for it. The usual pattern works: register on `sitechange`, `off` the old sprite listener, `on` the new one.
- A handler that raises an error is rolled back and reported (`error: ...` in the messages of the
  transaction); the other handlers still run and the failing one stays registered. A handler that exceeds a
  limit or is cancelled ends that event.
- Edits made by a handler do not produce a `change` event themselves.
- A session with listeners stays loaded until the last one is removed (`off`) or the script is stopped.
  Registering more than 1024 listeners in one session raises an error.
- Without the app (command line) `on` accepts the call, returns 0 and nothing ever fires.
- `ev:stopPropagation()` and canceling `beforecommand` are not available; calling `stopPropagation` raises
  an error.

Differences from Aseprite 1.3: `beforecommand` handlers see `ev.name` only (no `ev.params`) and cannot
cancel; there is no `beforeexit` and no `sprite.events` `remaptileset`; `sitechange` also reports
`ev.layer` / `ev.frame`.

### Plugins

A plugin is loaded once (by the app at launch, or when an extension is enabled), not run from the Scripts
menu:

```lua
function init(plugin)
  plugin.preferences.count = (plugin.preferences.count or 0) + 1     -- saved automatically
  plugin:newCommand{
    id = "mark", title = "Mark Corner...", group = "file_scripts",
    onenabled = function() return sprite ~= nil end,
    onchecked = function() return plugin.preferences.on == true end,
    onclick = function() sprite:setPixel(0, 0, 0xFF0000FF) end,       -- one undo step, named by the title
  }
  app.events:on("sitechange", function() end)
end
function exit(plugin) end                                            -- optional
```

- `plugin.name`, `plugin.path` (the plugin folder, `""` if none), `plugin.preferences` (table).
- `plugin:newCommand{ id, title, group, onclick, onenabled, onchecked }`: `id` and `title` are required;
  a command with an existing id replaces it. At most 256 commands. `plugin:deleteCommand(id)` removes one.
  `onenabled` / `onchecked` are evaluated after every event and command, must not change the sprite (any
  change is discarded) and count as false when they fail.
- `onclick` runs on the active document as one transaction (undo label: the command title, or
  `app.undoLabel`). It may open dialogs. An error rolls it back and is reported; the plugin keeps working.
- In `init`/`exit` `sprite` is `nil`; inside commands and event handlers it is the active document.
- `plugin.preferences` holds booleans, finite numbers, strings and tables of them (depth 16; functions,
  userdata, NaN and deeper values are not stored; integer keys of non-array tables become strings). It is
  loaded before `init` from a JSON file (an unreadable, non-object or over 1 MB file counts as empty),
  written after every transaction that changed it and after `exit`; over 1 MB it is not written and one
  warning is shown.
- `exit(plugin)` runs when the plugin is unloaded or the app quits, even if a command was cancelled; it has
  its own time budget. An error in `init` is reported but the commands registered so far keep working.
- `plugin:newMenuGroup` and `plugin:newMenuSeparator` raise "not supported". Every `group` is shown in the
  File > Scripts submenu.

Rust hosts use `scripting::host::Host::{load_plugin, run_command, stop, poll, send, broadcast, apply}`; the
C ABI is `pixmith_plugin_load` and `pixmith_script_*` ([LUA_DIALOG.md](LUA_DIALOG.md) section 4).

## Geometry: `Point`, `Size`, `Rectangle`

Value types as in Aseprite. They are copies: changing a field of one never changes the object it was built from.

| Constructor | Fields |
| --- | --- |
| `Point(x, y)`, `Point{x=, y=}`, `Point{x, y}`, `Point(otherPoint)`, `Point()` | `x`, `y` (read/write) |
| `Size(w, h)`, `Size{width=, height=}` (or `w=`, `h=`), `Size(otherSize)`, `Size()` | `width`, `height`, `w`, `h` |
| `Rectangle(x, y, w, h)`, `Rectangle{x=, y=, width=, height=}` (or `w=`, `h=`), `Rectangle{x, y, w, h}`, `Rectangle(otherRect)`, `Rectangle(point, size)`, `Rectangle()` | `x`, `y`, `width`, `height`, `w`, `h`, `origin` (Point), `size` (Size), `isEmpty` |

Numbers are integers (fractions are dropped). All three compare with `==` (by value; they are never equal to
another type) and convert with `tostring` (`Point{ x=1, y=2 }`, `Size{ width=3, height=4 }`,
`Rectangle{ x=1, y=2, width=3, height=4 }`). Points add, subtract and negate (`Point(1, 2) + Point(3, 4)`).

Rectangle methods (each argument may be a `Rectangle`, a plain table, or loose numbers):

- `rect:contains(point | rectangle | x, y | x, y, w, h)`: a point inside, or a whole rectangle inside.
- `rect:intersects(rectangle)`, `rect:intersect(rectangle)` (the overlap; an empty `Rectangle(0, 0, 0, 0)`
  when there is none), `rect:union(rectangle)` (the smallest rectangle holding both; empty ones are ignored).
- `Size:union(size)` is the larger width and height.

A rectangle is empty when its width or height is 0 or less.

**Everywhere a point or rectangle is taken, one of these objects works**, and so does a plain table
(`{x=, y=}`, `{x=, y=, w=, h=}`, `{width=, height=}`, or `{x, y}` / `{x, y, w, h}`) and the loose numbers older
scripts use: `getPixel` / `setPixel` (point), `getPixels` / `setPixels` (rectangle), `getTile` / `setTile`
(point), `newSlice`, `slice:setBounds` / `setCenter` / `setPivot`, `selection:selectRect` / `contains` /
`select` / `add` / `subtract` / `intersect`, `newCel` and `drawImage` (point), `cel.position`, `dlg.bounds`.
**Everywhere one is returned, it is now a `Rectangle` or `Point`** (`selection.bounds`, `slice:getBounds()`,
`slice:getCenter()`, `dlg.bounds`, `sprite.bounds`, `cel.bounds`, `image.bounds`); they still have the `x`, `y`, `w`,
`h` fields the earlier plain tables had, but `pairs()` / `#` on them no longer behave like on a table.

## Images

`Image` is Aseprite's image object: a width x height grid of RGBA pixels.

```lua
local img = Image(64, 64)                         -- transparent; Image(w, h, ColorMode.RGB)
local art = Image{ fromFile = "/Users/me/art/hero.png" }   -- a PNG, absolute path only
local copy = Image(art)                           -- Image(image) clones; Image(spec), Image(Size(w, h)) too
sprite:newCel(sprite.layers[1], 1, art, Point(8, 8))      -- the whole image becomes the cel, in Rust
```

Constructors: `Image(width, height [, colorMode])`, `Image{ width=, height=, colorMode= }` / `Image(ImageSpec{...})`,
`Image(size)`, `Image(otherImage)` (a copy) and `Image{ fromFile = path }`. `fromFile` follows the rules in
[Sandbox](#sandbox): absolute path, PNG only (any bit depth or color type; palette PNGs are expanded), at most
8192 x 8192. Images are **always RGBA in memory**; the `colorMode` given to `Image` (`ColorMode.RGB` 0,
`GRAY` / `GRAYSCALE` 1, `INDEXED` 2, `TILEMAP` 3) is only remembered, and pixel values are `0xRRGGBBAA` colors,
exactly as with `sprite:getPixel`. Writing an image into a cel of an Indexed or Grayscale sprite snaps its
colors to the palette or to gray.

Fields: `width`, `height`, `bounds` (Rectangle), `size` (Size), `colorMode`, `rowStride` (`width * 4`),
`bytesPerPixel` (4), `spec`, `cel` (the cel a `cel.image` view belongs to, else `nil`), and
**`bytes`** (read/write): the raw pixels as a Lua string, row-major, 4 bytes (R, G, B, A) per pixel. Assigning
checks the length (`width * height * 4` bytes exactly) and replaces every pixel in one native copy:

```lua
local img = Image(1024, 1024)
img.bytes = string.rep("\xff\x00\x00\xff", 1024 * 1024)   -- opaque red; a couple of VM instructions
```

Reading `img.bytes` creates a Lua string of that size, which counts toward the Lua memory limit.

Methods: `getPixel(x, y)` (`0` outside), `putPixel(x, y, color)` (replace), `drawPixel(x, y, color)` (alpha-blend),
`clear([color | rectangle])`, `clone()`, `isEqual(image)`, `isEmpty()`, `isPlain(color)`,
`drawImage(image, point [, opacity [, blendMode]])` (also `drawImage(image, x, y, ...)`; alpha-composites with the
image's top-left at `point`, clipped; opacity 0-255; `blendMode` as in `app.blend`),
`drawSprite(sprite, frame [, point])` (draws the composited frame), `resize(w, h)` / `resize(size)` (nearest
neighbour; not on `cel.image` views) and `pixels([rectangle])`:

```lua
for it in img:pixels() do      -- `it` is one object advanced in place, as in Aseprite
  if it() == 0xFF0000FF then it(0x00FF00FF) end     -- it() reads, it(color) writes; it.x, it.y
end
```

`img:saveAs(path)` raises an error: scripts cannot write files.

All bulk operations (`bytes`, `drawImage`, `clear`, `clone`, `newCel`, `cel.image = img`,
`sprite:setPixels(layer, frame, img)`) are done in Rust, so a whole 1024 x 1024 image costs a handful of VM
instructions. Only the per-pixel calls (`putPixel`, `pixels()`) cost one call each.

## Limits for embedders

Scripts run with `scripting::Limits { instructions, time, memory }`; the default is 300,000,000 VM
instructions, 10 seconds and 128 MiB of Lua memory. The type is public and can be built with
`Limits::default().with_time(Duration::from_secs(120))`, `.with_instructions(n)`, `.with_memory(bytes)` or
`Limits::new(instructions, time, memory)`. Entry points that take it:

- `scripting::run_with_limits(editor: &mut Editor, source: &str, limits: Limits) -> Outcome` (the top
  drawable layer and frame 1 are the defaults for the pixel calls);
- `scripting::run(editor, source, active: (layer, frame), limits)` and
  `scripting::run_with(editor, source, active, limits, cancel: &'static AtomicBool)`;
- `scripting::host::Host::with_limits(limits)` for sessions (dialogs, events, plugins): the limits apply to each
  transaction;
- the C ABI `pixmith_run_script_limits(doc, source, layer, frame, instructions, time_ms, memory_mb, out_messages)`
  where `0` for a budget means its default.

Work done natively (PNG decoding, `bytes`, `drawImage`, `newCel`) is not interruptible and not counted by the
instruction budget, but it is bounded by the image size caps above.

## Aseprite compatibility

Pixmith opens `.aseprite` files, and the scripting API follows Aseprite's names and semantics where Pixmith has
the concept, so a typical Aseprite script works unchanged.

**Supported (Aseprite spelling and meaning):** `Point`, `Size`, `Rectangle` (all constructors, fields,
`contains` / `intersects` / `intersect` / `union`, `==`, `tostring`); `Image` (`Image(w, h [, colorMode])`,
`Image{ fromFile }`, `bytes`, `getPixel`, `putPixel`, `drawPixel`, `drawImage`, `drawSprite`, `clear`, `clone`,
`isEqual`, `isEmpty`, `isPlain`, `pixels`, `resize`, `width` / `height` / `bounds` / `colorMode` / `rowStride` /
`bytesPerPixel` / `spec`, `ImageSpec`, `ColorMode`); `sprite:newCel(layer, frame, image, position)`,
`sprite:deleteCel`, `layer:cel(frame)`, `layer.cels`, `sprite.cels`, `cel.image` (get and set), `cel.position`,
`cel.bounds`, `cel.frameNumber`; `sprite:newSlice(rectangle)`, `slice.bounds` / `center` / `pivot`;
`sprite.bounds`; `selection:select` / `add` / `subtract` / `intersect` / `contains(point)` / `origin` / `bounds`;
layers, groups, frames (`duration`), tags, tilesets and tilemaps, user data, palette, `Dialog` (see
[LUA_DIALOG.md](LUA_DIALOG.md)), `app.events`, plugins (`init` / `exit`, `newCommand`), `app.alert`, `app.fgColor`.

**Known differences:**

- **Colors** are plain integers `0xRRGGBBAA`. There is no `Color` object and no `app.pixelColor`; Aseprite's
  `app.pixelColor.rgba()` packs `0xAABBGGRR` instead. Use `color.rgba(r, g, b, a)` / `color.split(c)`.
- **Images are always RGBA.** `Image(w, h, ColorMode.INDEXED)` gives an RGBA image that only remembers the
  mode; its pixel values are colors, not palette indices.
- **Cels are canvas-sized.** `cel.position` / `cel.bounds` describe the box around the non-transparent pixels, so a
  cel whose pixels touch the canvas edge or are moved off it loses the clipped part; images larger than the canvas
  are clipped when placed. `cel.image` is a view of that box, sized when you read it.
- `sprite:cel(layer, frame)` always returns a handle (Aseprite returns nothing for an empty cel; `layer:cel(frame)`
  does return `nil`). `cel.layer` and `cel.frame` are numbers (1-based), not objects.
- `slice.bounds`, `.center` and `.pivot` read and write the first frame's key; `newSlice()` without a rectangle
  covers the whole canvas.
- **Layers, frames and tags are indices or lightweight handles**, with the stale-handle rule above; there are no
  `app.activeSprite` / `app.open` / `Sprite{...}` constructors, `sprite:flatten`, `app.command`, `app.fs` or `app.clipboard`;
  the one document is the global `sprite` (also `app.sprite`).
- **No file writing and no other file reading**: `Image:saveAs`, `Sprite:saveAs`, `io`, `os` raise errors or do not
  exist. `Image{ fromFile }` accepts absolute paths to PNG files only (not `.aseprite`, `.gif`, relative paths or
  `app.fs.joinPath(...)`).
- `BlendMode` constants are `app.blend.*` (Pixmith numbers, which differ from Aseprite's enum values); `drawImage`
  takes those.
- `Image:flip`, `Image:shrink`, `Image:setPalette` and `Image:saveAs` do not exist (the last raises an explicit error).
- `Point` supports `+`, `-`, unary `-` and `==`; `Size` and `Rectangle` only `==` and `tostring` (no `Point * n` or `Size + Size`).


## `color`

- `color.rgba(r, g, b [, a])` packs 0-255 channels (alpha defaults to 255).
- `color.split(c)` returns `r, g, b, a`.

## Examples

`scripts/lua-examples/` contains runnable examples: `outline-layer.lua`, `color-ramp.lua`,
`flip-all-frames.lua`, `palette-swap.lua`, `tag-every-4-frames.lua` and `palette-ramp-dialog.lua`
(a dialog with a live ramp preview). `plugin-hello.lua` is a plugin example (a command with a dialog,
preferences and an event); it is loaded as a plugin, not run from File > Scripts. `plugin-events.lua`
tracks site changes, color changes and edits and offers a command that reports them.

## Embedding notes

`pixmith_run_script_at` returns the alert and `print` lines joined with `\n`. When a successful script
assigned `app.activeLayer` or `app.activeFrame`, one more line is appended:
`\u{1}PIXMITH-SELECT layer=<n> frame=<n>` (1-based, after a `U+0001` control character, which is
stripped from script output so scripts cannot fake it). The editor selects that layer and frame and
removes the line before showing the messages.
