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
| Colors | `Color` objects, or integers `0xRRGGBBAA` (`0xFF0000FF` is opaque red, `0` is transparent); see [Colors](#colors) |
| Blend modes | Aseprite's `BlendMode` numbers 0-18 (`app.blend.*` is the same table) |
| Color modes, directions | Aseprite's `ColorMode`, `AniDir` numbers |

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

Aseprite scripts run largely unchanged: `Color`, `app.pixelColor`, `Point`, `Size`, `Rectangle`, `Image`,
`Palette`, `Sprite(...)`, `app.open`, `app.fs`, `io`, `os`, `app.command`, `sprite:newCel`,
`sprite:newSlice(Rectangle)` and the rest of the Aseprite names exist; see [Colors](#colors),
[Files and permissions](#files-and-permissions), [Documents](#documents), [`app.command`](#appcommand),
[Geometry](#geometry-point-size-rectangle), [Images](#images) and [Aseprite compatibility](#aseprite-compatibility).

## Sandbox

Available: the base functions, `table`, `string`, `utf8`, `math`, `coroutine`, and `io`, `os`, `dofile`, `loadfile`
and `app.fs` behind permission questions (next section). Not available: `package`, `require`, `debug`,
`os.exit`, `os.setlocale`, `io.stdin`. `load` accepts text chunks only (no bytecode). `os.getenv` answers for a short
list of harmless variables (`HOME`, `USER`, `TMPDIR`, ...) and `nil` for the rest. A run is stopped with an error
after 300 million VM instructions, 10 seconds or 128 MB of Lua memory, whichever comes first (hosts can raise
these, see [Limits](#limits-for-embedders)); time spent waiting for a permission dialog or a `Dialog` does not count.
One very expensive `string.find`/`gsub` pattern is a single VM instruction and cannot be interrupted,
so avoid catastrophic patterns.

Scripts cannot grow the document without bound either: at most 1024 layers, 4096 frames, 4096
tags, 4096 slices and 64 MB of tile pixels per tileset. Free-standing `Image`s (kept outside the Lua
heap) are limited to 67,108,864 pixels (the largest canvas) each and 512 MB alive at once, and one run may
create or open at most 32 sprites (512 MB of pixels together).

## Files and permissions

Like Aseprite, a script **asks first** before it touches anything outside its document. Each of these calls
needs permission for the path (or, for programs, the command) it names:

| Call | Needs |
| --- | --- |
| `io.open` (read modes), `io.lines`, `io.input(path)`, `dofile`, `loadfile`, `Image{ fromFile }`, `Palette{ fromFile }`, `app.open`, `Sprite{ fromFile }`, `sprite:loadPalette`, `app.fs.isFile` / `isDirectory` / `fileSize` / `listFiles` | read |
| `io.open` (modes `w`, `a`, `r+`, ...), `io.output(path)`, `os.remove`, `os.rename` (both paths), `app.fs.makeDirectory` / `makeAllDirectories` / `removeDirectory`, `sprite:saveAs` / `saveCopyAs`, `image:saveAs`, `palette:saveAs`, `app.command.ExportSpriteSheet` / `SaveFile*` | write |
| `os.execute(command)`, `io.popen(command)` | execute |

A question is answered, in this order, by: a path the host pre-allowed (`allowed_paths`, e.g. the MCP server's
`allow_paths`); an earlier **Allow once** for the same path or a parent directory in the same run (a write grant
also covers reads); a path returned by `os.tmpname()` in this run; an earlier **Always allow this script** for that
kind of access (until the app quits); otherwise the front end's dialog. With no front end handler (command line,
tests) the answer is *no*, and the call raises `permission denied: script "name" may not read /path`. The answer
is a Lua error, so `pcall` catches it. See [LUA_HOST_API.md](LUA_HOST_API.md) for what a front end implements.

`io` and `os` are Lua's own libraries once permission is given: `io.open` returns the usual file objects
(`read`, `write`, `lines`, `seek`, `close`, ...), `os.time`, `os.date`, `os.clock`, `os.difftime`, `os.tmpname`,
`os.remove`, `os.rename`, `os.execute` work as in Lua. Standard input does not exist (`io.read()` returns `nil`).

`app.fs` (strings use `/`; `\` is also accepted as a separator in the path functions):

| Function | |
| --- | --- |
| `pathSeparator` | `"/"` |
| `filePath(p)`, `fileName(p)`, `fileExtension(p)`, `fileTitle(p)`, `filePathAndTitle(p)` | `"/a/b/c.png"` gives `"/a/b"`, `"c.png"`, `"png"`, `"c"`, `"/a/b/c"` |
| `normalizePath(p)`, `joinPath(a, b, ...)` | lexical (`.` / `..` removed, separators `/`); never asks |
| `currentPath()`, `appPath()`, `tempPath()`, `userDocsPath()`, `userConfigPath()` | directories as strings; never asks |
| `isFile(p)`, `isDirectory(p)`, `fileSize(p)` (0 when missing), `listFiles(dir)` (sorted names) | read |
| `makeDirectory(p)`, `makeAllDirectories(p)`, `removeDirectory(p)` (empty directories only) | write; return `true`/`false` |

## `sprite`

### Properties

| Name | |
| --- | --- |
| `width`, `height` | canvas size |
| `layerCount`, `frameCount`, `paletteSize` | counts |
| `colorMode` | Aseprite's `ColorMode`: `ColorMode.RGB` 0, `GRAY` / `GRAYSCALE` 1, `INDEXED` 2 |
| `filename` | the document's file (`""` when it has none; read/write: assigning only renames, `saveAs` writes) |
| `isModified` | whether the sprite differs from the file it was last saved to or opened from |
| `spec` | an `ImageSpec` of the canvas: `Image(sprite.spec)` |
| `palettes` | a list with the sprite's palette (`sprite.palettes[1]`), see [Palette](#palette) |
| `rootLayers`, `backgroundLayer` | top-level layers; the background layer or `nil` (`layers` is the flat list, children included) |
| `data`, `color` | the sprite's user data text and `Color` (shortcuts for `userData.text` / `.color`) |
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
- `sprite:deleteLayer(layer)` (a layer handle, its number or its name) deletes a layer and, for a group, everything in it. The last drawable layer
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
| `name`, `visible`, `opacity` (0-255), `blendMode` (a `BlendMode` value) | read/write |
| `isGroup`, `isTilemap`, `isImage`, `isBackground`, `isEditable`, `index` (= `stackIndex`), `level`, `sprite` | read-only (`level` is the nesting depth, 0 at the top) |
| `isReference` | read/write: a reference layer is shown in the editor but never exported; turning it on also locks the layer against drawing. Groups cannot be reference layers |
| `parent` | the containing group, or `nil` |
| `children` | direct children of a group (bottom to top); empty for other layers |
| `isVisible` | shown on the canvas, i.e. `visible` and every parent group `visible`; assigning sets `visible` |
| `tileset` | the tileset of a tilemap layer, or `nil` |
| `isIsometric` | read-only: an isometric tilemap layer (see Isometric tilemaps) |
| `gridOffset` | read/write `Point` on isometric tilemap layers, `nil` elsewhere: the canvas pixel of cell (0, 0)'s box |
| `userData` | text and color attached to the layer (see User data); `layer.data` / `layer.color` are shortcuts |

Tilemap layers also have `layer:getTile(x, y [, frame])` and
`layer:setTile(x, y, index [, flipX, flipY, diagonal [, frame]])`; see Tilemaps.

### Tween rig (parented layers)

Pixmith's per-layer tween (keys for x, y, rotation, scale and opacity, a pivot) can be chained: a layer may have a **tween parent**, and its transform at a frame is the parent's transform composed with its own. Rotating a thigh moves the knee, and the shin, whose pivot is the knee, rotates about the moved knee. Opacity is not inherited. A child without keys of its own simply follows its parent; frames outside the key range of the layer and all its ancestors show the layer's own cels. Cels are resampled by nearest neighbor (RotSprite for turns that are not multiples of 90 degrees), never blended.

- `layer.tweenParent` (read/write): the parent layer handle or `nil`. Assigning a layer (handle, number or name) links, `nil` unlinks. It raises an error for a cycle (including a layer being its own ancestor), for tilemap layers, and for a group and a layer inside it. The link is stored in the layer's user data, so it survives layer moves and file saves.
- `layer:setTweenKey(prop, frame, value [, ease])` adds or replaces a key. `prop` is `"x"`, `"y"`, `"rotation"` (degrees, clockwise), `"scaleX"`, `"scaleY"` or `"opacity"` (0..1); `ease` is `"linear"` (default), `"hold"`, `"inQuad"`, `"outQuad"`, `"inOutQuad"`, `"inOutCubic"`, `"outBack"` or `"outBounce"`. The layer's cel at its first key frame is the source image.
- `layer:setTweenPivot(x, y)` sets the pivot in canvas coordinates (pixel corners; in the rest pose, i.e. the child's own space); `layer:setTweenPivot(nil)` goes back to the center of the content.
- `layer:tweenTransform([frame])` returns `nil` when neither the layer nor an ancestor has keys, else a table: `matrix` and `local` (six numbers `a, b, c, d, e, f` with `x' = a*x + c*y + e`, `y' = b*x + d*y + f`; `matrix` includes the parents, `local` is the layer's own transform), `rotation` (degrees, of the composed matrix), `scaleX`, `scaleY` and `pivotX`, `pivotY` (where the layer's own pivot is on the canvas).
- `layer:tweenMap([frame,] x, y)` returns the canvas position of a rest-pose point at that frame (an error if the layer is not tweened).

```lua
local thigh, shin = sprite.layers[1], sprite.layers[2]
thigh:setTweenPivot(48, 30)   shin:setTweenPivot(48, 42)   -- hip and knee
shin.tweenParent = thigh
for f, deg in ipairs({ 0, 20, 0, -20 }) do thigh:setTweenKey("rotation", f, deg) end
for f, deg in ipairs({ 0, -30, 0, 10 }) do shin:setTweenKey("rotation", f, deg) end
local t = shin:tweenTransform(2)       -- t.pivotX, t.pivotY: where the knee is at frame 2
```

Baking (turning the animation into real cels) is a UI command (Bake / Bake All): it renders every layer from the unchanged document before writing anything, so parents' motion reaches their children whatever the order, and writes parents before children. Baking a child alone uses its composed transform and detaches it; a layer that still has children cannot be baked in place without `keep source` (use Bake All).

### Frames

- `sprite:newFrame([copy])` appends a frame after the last one (empty, or a copy of it when `copy` is true);
  `sprite:newFrame(frame)` inserts a copy of that frame after it (Aseprite's form).
- `sprite:newEmptyFrame([frameNumber])` inserts an empty frame so that it becomes that frame number (default: the end).
- `sprite:duplicateFrame(i)` inserts a copy of frame `i` right after it and returns it.
- `sprite:deleteFrame(frame)` (a handle or a number) deletes a frame; tags are adjusted. The last frame cannot be deleted.

Frame handle fields: `duration` (milliseconds, a whole number of 1-65535: `0.1` is an error, Aseprite's seconds need `* 1000`; read/write), `index` (= `frameNumber`), `sprite`. Frame and layer
handles compare with `==` by position.

### Tags

- `sprite:newTag(name, from, to)` adds a tag over frames `from..to` (swapped ends are put in order) and returns it.
- `sprite:deleteTag(tag)` (a handle, number or name), or `tag:delete()`.

Tag handle fields: `name`, `from`, `to` (frame numbers, kept in order), `direction` (see
`app.direction` / `AniDir`; `aniDir` is the same value), `repeatCount` (0 = forever), `color` (a `Color`;
integers and `Color`s are accepted when assigning) and `data` (the user data text) are read/write; `index`,
`frames` (how many), `fromFrame` / `toFrame` (frame handles) are read-only.

### Cels

- `sprite:cel(layer, frame)` returns a cel handle (groups have no cels); `layer` and `frame` may be numbers or
  handles. `layer:cel(frame)` is the Aseprite form and returns `nil` when the layer has no image in that frame;
  `layer.cels` lists the layer's cels.
- `sprite:clearCel(layer, frame)` removes the image of a cel; returns whether anything was removed.
  `sprite:deleteCel(cel)` / `sprite:deleteCel(layer, frame)` is the Aseprite spelling.
- `sprite:newCel(layer, frame [, image [, position]])` makes (or replaces) the cel of `layer`/`frame` and returns
  it. With an `Image` the cel shows that image with its top-left at `position` (a `Point`, default `0, 0`),
  in one call; the position may lie beyond the canvas edges and the whole image is kept (only what is inside the
  canvas is drawn); without an image the cel is empty. See [Images](#images).

Cel handle: `opacity` (0-255, read/write, the cel must not be empty), `zIndex` (-32768..32767, read/write, the cel must not be empty: the cel draws as if its layer sat that many places higher, or lower when negative; the order is layer index plus z-index and ties go to the lower layer; linked cels share it), `isEmpty` (no cel or fully
transparent), `linked` (shares its image with other frames), `linkedFrames` (list of the other
frame numbers that share it), `layer`, `frame` (a layer handle and a frame handle, as in Aseprite:
`cel.frame.frameNumber`; `cel.frameNumber` is the plain number), `sprite`, `data` / `color` (user data shortcuts),
and `cel:unlink()` which gives the cel its own copy. Aseprite's cel geometry:

- `cel.bounds` (read-only `Rectangle`) and `cel.position` (`Point`, read/write): as in Aseprite a cel is an image
  with a position that may extend beyond the canvas. These describe the box around all of the cel's
  non-transparent pixels, inside the canvas or not (`0,0,0,0` / `Point(0, 0)` for an empty cel); coordinates can be
  negative. Assigning `cel.position` moves the whole cel and loses nothing: moving it off the canvas and back
  restores every pixel. Positions must stay within -32768..32767 (the `.aseprite` file format's range).
- `cel.image` (read/write): a **live view** of that box as an `Image` (`nil` for an empty cel): drawing on it
  draws on the cel, also on the parts beyond the canvas. Assigning an `Image` replaces the cel's content with it,
  keeping the cel's position (`0, 0` for an empty cel).

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

#### Isometric tilemaps

An isometric tileset puts its tile images on 2:1 diamond cells (Godot's `TILE_SHAPE_ISOMETRIC` with
`TILE_LAYOUT_DIAMOND_DOWN`). Cell `(i, j)` has its bounding box at
`(gridOffset.x + (i - j) * cellWidth / 2, gridOffset.y + (i + j) * cellHeight / 2)`: `i` runs down-right,
`j` down-left. Cells are drawn back to front (by `i + j`, then `i`), so a tile image taller than the diamond
(a cliff, a tuft of grass) overlaps the cells behind it. Tile images are `tileWidth` x `tileHeight`; the diamond
is `cellWidth` x `cellHeight`. Square tilesets and layers are unaffected by everything in this section.

Tileset fields (`nil` on square tilesets, `false` for `isIsometric`): `isIsometric`, `cellWidth`, `cellHeight`,
`layout` (`"diamond_down"`). Methods:

- `ts:getPivot(t)` returns a `Point`: the point of tile `t`'s image (pixels from its top-left corner) that sits on
  the center of the diamond. Unless set, tiles are centered across and, when taller than the diamond,
  bottom-aligned (pivot `(tileWidth / 2, tileHeight - cellHeight / 2)`), so the diamond is the bottom
  `cellHeight` rows of the image.
- `ts:setPivot(t [, point])` sets the pivot (no point restores the default); layers showing the tile are redrawn.
- `ts:getTextureOrigin(t)` returns the pivot as Godot's `TileData.texture_origin` (offset from the image center);
  that is what the Godot export writes.
- `ts:setIsometric(cellWidth, cellHeight)` / `ts:setSquare()` change the shape of a tileset that no layer shows
  tiles of (an error otherwise).

Layer methods (also work on square tilemap layers, where a cell is `pixel / tile size`):

- `layer:cellAt(x, y)` (or a `Point`) returns `i, j`, the cell that contains the canvas pixel.
- `layer:cellOrigin(i, j)` returns a `Point`: the canvas pixel of the cell's box (top-left).
- `layer:pickTile(x, y [, frame])` returns `i, j, index, flipX, flipY, diagonal` for the tile you *see* at a canvas
  pixel of an isometric layer (the front-most tile whose image has a visible pixel there, else the tile of the cell
  whose diamond contains it), or nothing when that cell is empty.

On an isometric layer `getTile` / `setTile` take the cell `(i, j)` itself (any integers; the layer's grid grows to
hold the cell; cells may be negative and may lie outside the canvas, where they are simply not visible).
Assigning `layer.gridOffset` moves the whole grid, which is how layers at other elevations are placed: a ledge half
a cell higher is a layer whose offset is `cellHeight / 2` pixels above the floor's. A game map with 96 x 48 cells
whose floor starts at pixel (592, 360) is `gridOffset = Point(592, 360)`.

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
  - `slice:getPivot([frame])` returns `x, y`, or `nil, nil` without a pivot and for frames before the first pivot
    key. `slice:setPivot(point | x, y [, frame])`: without a frame, a slice that has no pivot yet gets it on all its
    existing keys (every frame has it; keys created later inherit it), and one that already has a pivot changes
    only its first key. With a frame (not the active frame) the pivot starts there and earlier frames have none.
    `slice:clearPivot()` removes it. The .aseprite file stores one pivot flag per slice, so after saving and
    reopening a pivot applies from the slice's first frame.

### User data

`sprite.userData`, `layer.userData`, `cel.userData`, `tag.userData` and `slice.userData` give access
to the text and color Aseprite attaches to those objects: `ud.text` (string or `nil`) and
`ud.color` (a `Color` or `nil`; integers `0xRRGGBBAA` and `Color`s are accepted when assigning), both read/write; assign `nil` (or `""` for the text) to clear.
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

- `sprite:getPaletteColor(i)` (a `0xRRGGBBAA` integer), `sprite:setPaletteColor(i, color)` (index `paletteSize` appends a
  color; in Indexed mode pixels using the old color follow), `sprite:addPaletteColor(color)`
  (returns the index, reusing an existing identical color).
- Aseprite's palette object: `sprite.palettes[1]` is live (edits change the sprite); `Palette(n)`, `Palette{ fromFile = path }`
  (`.gpl`, `.hex`, an image, or a `.aseprite` / `.pxmt` file) and `palette:clone()` make free-standing ones.
  `#palette`, `palette:getColor(i)` (a `Color`), `palette:setColor(i, color)`, `palette:resize(n)` (1..256; new entries are
  opaque black), `palette:saveAs(path)` (`.gpl`, `.hex` or `.png`). Indices are 0-based and checked.
  `sprite:setPalette(palette)` and `sprite:loadPalette(path)` replace the sprite's palette (in Indexed mode pixels
  follow their entries).

## `app`

- `app.alert(message)` shows `message` when the script ends. `app.alert{ title=, text=, buttons={...} }`
  shows a modal message at once and returns the number of the pressed button (see Dialogs).
- `app.activeSprite` (also `app.sprite`) is the active document: the one the script runs on until it creates,
  opens or activates another (`nil` in plugin `init`/`exit` without a document). Assigning a sprite activates it.
  The global `sprite` always stays the document the script was started on. `app.sprites` lists that document
  and the ones the script created or opened. See [Documents](#documents).
- `app.activeLayer`, `app.activeFrame`, `app.activeCel`, `app.activeImage`, `app.activeTag` (also `app.tag`) and
  `app.site` (`{ sprite, layer, frame, cel, image, frameNumber }`) describe the active sprite, as objects: layer and
  frame handles, a cel handle (`nil` for an empty cel), the cel's `Image`. `app.range` is the active cel as a one-item
  range (`type`, `isEmpty`, `layers`, `frames`, `cels`, `images`). Assigning `app.activeLayer` or `app.activeFrame` (a
  handle or a 1-based number) changes the defaults of the pixel calls immediately and selects that layer or frame in
  the editor when the script ends (a successful run only). A group cannot be assigned as the active layer.
- `app.isUIAvailable` is `false` for headless runs (command line), `true` inside the app.
- `app.fgColor`, `app.bgColor`: the drawing colors as `Color` objects; assign a `Color` or a `0xRRGGBBAA` integer.
- `app.events:on(name, fn)` / `app.events:off(listener)` and `sprite.events:on("change", fn)`:
  see Events.
- `app.undoLabel(text)` names the undo step (default "Script"). `app.transaction([label,] fn)` runs `fn` and undoes
  its changes if it raises; `app.undo()` / `app.redo()` step through what the script did so far (see `app.command`).
- `app.frameDiff{ frame=, layer=, wrap=, mask= }` tells which pixels a frame changed compared with the frame before
  it (the "frame-difference view" of the editor), to find parts that did not move with the rest of a character.
  `frame` (1-based number or frame handle) defaults to the active frame; `layer` (number, name or layer handle)
  restricts the comparison to one layer's cel (hidden layers count; a missing cel is transparent), otherwise the
  composite of the visible layers is compared (reference layers left out). The frame before frame 1 is the last
  frame when `wrap` is true (the default; right for looping animations) and none when `wrap=false`; a one-frame
  sprite never has one. Returns `{ frame, previous, layer, changed, appeared, disappeared, total, bounds, mask }`:
  `previous` is the 1-based frame compared against (`nil` when there is none, and then every count is 0);
  `appeared` counts pixels that were transparent and are not, `disappeared` the other way round, `changed` pixels
  that are not transparent in either frame but differ in color or alpha (the color of a fully transparent pixel never
  counts); `total` is their sum; `bounds` is a `Rectangle` around all of them (`nil` when none differ).
  With `mask=true`, `mask` is an `Image` the size of the sprite: opaque magenta (`changed`), green (`appeared`) or
  red (`disappeared`) where a pixel differs, transparent elsewhere. It is read-only and changes nothing.
  ```lua
  for f = 1, #sprite.frames do
    local d = app.frameDiff{ frame = f, layer = "Body" }
    if d.total > 0 then print(f, d.changed, d.appeared, d.disappeared, d.bounds) end
  end
  ```
- `app.refresh()` does nothing (there is nothing to redraw while a script runs).
- `app.version` is a `Version` object (`app.version.major`, `tostring(app.version)`, comparable with `<`, `<=`, `==`);
  `Version("1.3-rc1")`, `Version(1, 3, 0)` and `Version{ major=, minor=, patch= }` create them. `app.apiVersion` is the
  level of the Aseprite scripting API this reference follows (21).
- `app.blend` = `BlendMode` under the older names: `Normal` 0, `Multiply`, `Screen`, ... (the values are Aseprite's).
- `app.direction`: `Forward` 0, `Reverse` 1, `PingPong` 2, `PingPongReverse` 3 (same as `AniDir`).
- `app.pixelColor`, `app.fs`, `app.open`, `app.command`: see their sections.
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

## Colors

Three number formats meet in the API; each function speaks exactly one, so know which:

| Format | Where | Example: opaque red |
| --- | --- | --- |
| Pixmith color integer `0xRRGGBBAA` | everything that existed before `Color` did: `sprite:getPixel` / `setPixel` / `getPixels` / `setPixels`, `getPaletteColor` / `setPaletteColor`, `color.rgba`, `color.split`, `fillSelection`, tile pixels, `app.fgColor = n`, dialog colors, `tag.color = n` | `0xFF0000FF` |
| Aseprite pixel value | `Color(n)`, `app.pixelColor.*`, and the `Image` pixel functions (`getPixel`, `putPixel`, `drawPixel`, `clear`, `isPlain`, `pixels()`) | `0xFF0000FF` (RGB images store it as `0xAABBGGRR`, so only red and white look alike in both formats) |
| `Color` objects | Aseprite's color class | `Color(255, 0, 0)` |

**Every function that takes a color accepts a `Color` object as well as a Pixmith integer**, so
`sprite:setPixel(0, 0, Color(255, 0, 0))` and `sprite:setPixel(0, 0, 0xFF0000FF)` are the same call. Functions that
return a color return a Pixmith integer when they always did (`getPixel`, `getPaletteColor`, `getPixels`) and a
`Color` where Aseprite returns one (`app.fgColor`, `app.bgColor`, `palette:getColor`, `tag.color`, `layer.color`,
`userData.color`, `dlg.data.<color widget>`). Images follow Aseprite: `image:getPixel` returns a pixel value
(RGB `0xAABBGGRR`, grayscale `gray | alpha << 8`, indexed images the palette index), and `putPixel` takes the same
kind of value or a `Color`.

Bridges: `color.toPixel(c)` (Pixmith integer or `Color` to Aseprite pixel value), `color.fromPixel(px)` (the other
way), `color.rgba(r, g, b [, a])` / `color.split(c)` (Pixmith integers). A `Color` used in a bit operation
(`c >> 24`, `c & 0xFF`) acts as its Pixmith integer, so scripts that did arithmetic on `app.fgColor` keep working.
Comparing a `Color` with a plain number using `==` is always false: convert one side.

`Color` constructors (all channel numbers 0-255 unless noted):

- `Color(r, g, b [, a])`, `Color{ r=, g=, b=, a= }` (or `red=`, `green=`, `blue=`, `alpha=`; `a` defaults to 255),
  `Color(pixelValue)` (Aseprite's `0xAABBGGRR`), `Color("#rrggbb")` / `"#rrggbbaa"` / `"#rgb"` / `"#rgba"`.
- `Color{ h=, s=, v=, a= }` HSV (hue 0-360, saturation and value 0-1; also `hue=`, `saturation=`, `value=`),
  `Color{ h=, s=, l=, a= }` HSL (`lightness=`).
- `Color{ gray=, alpha= }` and `Color{ index= }` (a palette index of the active sprite).

Properties (all readable; assigning `red`, `green`, `blue`, `alpha`, `hue`, `saturation`, `value`, `lightness`,
`hslHue`, `hslSaturation`, `hsvHue`, `hsvSaturation`, `hsvValue`, `gray` or `index` changes the color; colors are
mutable): `red`, `green`, `blue`, `alpha` (also `r`, `g`, `b`, `a`), `hue` / `saturation` / `value` (HSV),
`lightness` (HSL), `hsvHue`, `hsvSaturation`, `hsvValue`, `hslHue`, `hslSaturation`, `hslLightness`, `gray`
(luma for RGB colors), `index` (the nearest palette entry for RGB colors), `rgbaPixel` (`0xAABBGGRR`), `grayPixel`
(`gray | alpha << 8`). `==` compares the resolved RGBA (two index colors compare by index); `tostring` gives
`Color{ r=1, g=2, b=3, a=4 }` (or the `h=`/`s=`/`v=`, `gray=`, `index=` form of the model it was built with).

`app.pixelColor`: `rgba(r, g, b [, a])` packs `r | g << 8 | b << 16 | a << 24`; `rgbaR(px)`, `rgbaG`, `rgbaB`,
`rgbaA` unpack; `graya(v [, a])` packs `v | a << 8`; `grayaV(px)`, `grayaA(px)` unpack.

## Documents

A script runs on one document (the global `sprite`), but can create more, as in Aseprite:

- `Sprite(width, height [, colorMode])`, `Sprite(ImageSpec{ width=, height=, colorMode= })`, `Sprite(otherSprite)` (a
  copy), `Sprite{ fromFile = path }` (any file Pixmith opens: `.aseprite`, `.pxmt`, PNG, GIF, ...) create a sprite
  with one layer, one frame and an (empty) first cel. It becomes the active sprite.
- `app.open(path)` opens a file and makes it active (`nil` when it cannot be read; if the path is already open in the run
  it returns that sprite). `app.sprites` lists the open sprites; `app.activeSprite = s` switches.
- `sprite:saveAs(path)`: saves in the format of the extension (`.aseprite` / `.ase`, `.pxmt`, `.png`, `.gif`, `.webp`,
  `.jpg`, `.bmp`, `.tga`, `.ico`, `.pcx`, `.flc`); for `.aseprite` / `.ase` / `.pxmt` the sprite's `filename`
  becomes `path`, other extensions export and leave `filename` alone. `sprite:saveCopyAs(path)` never changes
  `filename`. A still format with several frames writes one file per frame, `<name>-001.png`, `<name>-002.png`, ...
- `sprite:close()` closes it (the previous sprite becomes active again). Using a closed sprite raises an error.
- `sprite.filename` of the document the script runs on is its file as the editor knows it.

What the editor shows afterwards: sprites still open when the script ends open as new tabs (images such as PNGs
untitled), the active one is selected, the original document's edits are one undo step, and a closed original is
closed without a save prompt. A script that fails opens nothing (files it already wrote stay). See
[LUA_HOST_API.md](LUA_HOST_API.md).

## `app.command`

`app.command.Name{ ... }` runs a command on the active sprite (the parameters are Aseprite's). Commands that exist:

- Frames and layers: `NewFrame{ content = "empty" }` (a copy of the active frame by default), `RemoveFrame`, `NewLayer{ name, group, reference }`,
  `RemoveLayer`, `DuplicateLayer`, `MergeDownLayer`, `FlattenLayers`, `GotoFirstFrame`, `GotoPreviousFrame`,
  `GotoNextFrame`, `GotoLastFrame`, `GotoFrame{ frame }`, `GotoPreviousLayer`, `GotoNextLayer`.
- History: `Undo`, `Redo` step back and forward through the changes the script made itself: each command, each
  `app.transaction` and the plain API edits between them are one step each (nothing from before the script
  can be undone).
- Transformations: `Flip{ orientation = "horizontal" | "vertical", target = "canvas" }` (the whole sprite; without `target` it
  flips the selection or the active cel), `FlipHorizontal`, `FlipVertical` (the whole sprite),
  `Rotate{ angle = 90 | 180 | 270 }` (the whole sprite; `target = "mask"` rotates the selection or cel),
  `CanvasSize{ left, top, right, bottom }` (negative values crop), `SpriteSize{ width, height }` (or one of them,
  or `scale`; nearest neighbour), `CropSprite` (to the selection), `AutocropSprite` (to the content),
  `ChangePixelFormat{ format = "rgb" | "gray" | "indexed", dithering = "none" | "ordered" }`.
- Filters on the active cel, limited to the selection: `ReplaceColor{ from, to, tolerance }`, `Outline{ color, place = "outside" | "inside", matrix }`,
  `InvertColor`, `Desaturate`, `BrightnessContrast{ brightness, contrast }` (-100..100), `HueSaturation{ hue, saturation, lightness }`,
  `Despeckle`, `RemoveBackground{ tolerance, shadow, shadowDepth, global, color }` (see below), and the Pixmith-specific `CleanEdges{ ... }` (see [Clean Edges and lint](#clean-edges-and-lint)).
- Warp (a pixel-art puppet / mesh warp, Pixmith's own command) of the active cel, limited to the selection:
  `Warp{ grid = {cols, rows}, offsets = {{dx, dy}, ...}, samples = 4, smooth = false, pins = {{ x, y, dx, dy, radius }, ...} }`.
  A `cols` x `rows` grid of handles (each 2 to 8, default `{2, 2}`) is spread over the selection bounds (the canvas
  when nothing is selected); `offsets` has one `{dx, dy}` (or `{dx = , dy = }`) per handle, row by row, and says where the
  content at that handle goes, in pixels (a missing `offsets` is all zeros). Between handles the displacement is
  interpolated bilinearly, or with a Catmull-Rom spline when `smooth = true`. Each pin moves the content within `radius`
  of canvas point (`x`, `y`) by up to (`dx`, `dy`), fading out smoothly. The result is resampled by majority vote: each
  output pixel is cut into `samples` x `samples` points (1 to 8), each is mapped back to the source, and the most frequent
  source color wins (transparent counts as a color; a tie goes to a visible color, then to the sample nearest the pixel
  center), so nothing is blended and no new color appears. Pixels outside the selection are neither read nor changed,
  and where nothing is mapped the result is transparent. Bad grids, a wrong number of offsets or malformed pins raise an
  error. Very thin diagonal lines can break under a bend, as a 1 px diagonal covers under half of each output pixel.
- Pixel-art cleaning: `RemoveBackground{ ... }` and `TraceGrid{ ... }` (see below). Both return a report table.
- Selection and cels: `SelectAll`, `DeselectMask`, `InvertMask`, `MaskContent`, `Clear`, `ClearCel`.
- Files: `NewFile{ width, height, colorMode }`, `OpenFile{ filename }`, `SaveFile`, `SaveFileAs{ filename }`,
  `SaveFileCopyAs{ filename }`, `CloseFile`, `DuplicateSprite`, `ExportSpriteSheet{ type = SpriteSheetType.HORIZONTAL | VERTICAL | ROWS | COLUMNS | PACKED,
  textureFilename, dataFilename, columns, rows, shapePadding, borderPadding, extrude }` (a PNG sheet and Aseprite-style JSON; layers
  as shown, all frames; `PACKED` is a square-ish grid; `trim`, `splitLayers`, `tag`, `openGenerated`, ... are ignored).
- `Refresh` does nothing.

### `RemoveBackground` and `TraceGrid`

For art from AI image tools: "pseudo-pixel" art upscaled with soft edges, a slightly drifting grid, JPEG-like noise, a
paper-white background and a soft drop shadow.

`app.command.RemoveBackground{ tolerance = 28, shadow = false, shadowDepth = 55, global = false, color = nil }` works on the
active cel, inside the selection. The background colour is the dominant colour of the image border (or `color`). It is
removed by a flood fill from the edges through pixels within `tolerance` (0..255 per channel) of it, so equal colours
enclosed by the figure (eyes, highlights) stay. `global = true` also removes every such pixel inside the figure.
`shadow = true` also removes a soft shadow: pixels that are the background colour darkened by a common factor (down to
`shadowDepth` percent darker, in the hue of the background) that connect to the removed background through other such
pixels. A shadow step that would take more than 60 % of what remains (a grey figure, not a shadow) is skipped and
reported as `shadowSkipped`. Returns `{ color, background, global, shadow, shadowSkipped, removed }` (pixel counts;
`color` is the background colour used, absent when the border has no dominant colour).

`app.command.TraceGrid{ pitch = "auto", colors = 16, removeBackground = true, closeOutline = true }` traces the sprite
onto a real pixel grid. The active cel defines the grid, the palette and the crop; every image cel of the sprite is
resampled with them, so an animation stays on one grid. The canvas becomes the traced size and the sprite palette the
traced palette. An RGB sprite is required (no tilemap layers). One undo step.

| Parameter | Meaning |
| --- | --- |
| `pitch` | `"auto"` (default), a number, or `{ x, y }`: cell size in source pixels. Fractions are fine (6.5, 7.3). |
| `minPitch`, `maxPitch` | Range for `"auto"` (default 3 and 32). |
| `phase` | `{ x, y }` offset of the first cell edge; default: detected. |
| `separateAxes` | Find x and y pitch independently (default: one pitch for both, which is steadier). |
| `colors` | A number (k-means palette of that size), `"auto"` (default; the smallest count in `minColors`..`maxColors`, 2..32, that fits), `"keep"` (no quantising), `"palette"` (the sprite's current palette) or a list of colors (snap to those). |
| `removeBackground` | Remove the background from the traced cells; takes `tolerance`, `shadow`, `shadowDepth`, `global` and `backgroundColor` as above. |
| `closeOutline` | Close one-pixel breaks in the dark outer outline (a transparent pixel with outline pixels on both sides and figure fill on one side only). |
| `crop` | Crop to the opaque cells (default true). |
| `seed` | Seed of the k-means start (default 1; the result is deterministic). |

The grid is found by folding the per-column and per-row edge energy modulo each candidate pitch and keeping the sharpest
(the scan step keeps the drift over the whole image under half a pixel; of near-ties the largest pitch wins, because
half the true pitch folds as sharply). Soft AI sheets drift and jitter, so no single fold of the whole image lines up
(it is flat). When the whole-image fold is weak, the pitch (up to 16) is searched in overlapping 96 px tiles instead:
every tile has its own phase, and the candidate whose Fourier power stands out most against its neighbouring pitches
wins. A grid that is present but weak (soft or drifting edges) is returned anyway, with a `warning` in the result (check
the quality, or pass `pitch`). Only an image without edges ("no edges to line up") or one whose best pitch is
indistinguishable from noise ("grid too weak (best pitch ≈ N, concentration X); pass pitch") raises an error. Each cell is the per-channel median of its central 60 % (transparent pixels are
ignored), then the palette and the snap are applied. Returns `{ width, height, pitch = { x, y }, phase = { x, y },
palette, quality, closedOutline, background, warning }`; `warning` is only present for a weak grid; `palette` is a list of `Color`, `quality` is 0..1 (how well cells and
palette explain the source pixels) and `background` is the `RemoveBackground` report. A sprite without edges (flat) has
no grid and raises an error.

```lua
local r = app.command.TraceGrid{ pitch = "auto", colors = 16, removeBackground = true, shadow = true, closeOutline = true }
print(r.pitch[1], r.width, r.height, r.quality)
```

The same operations exist in the core as `pixmith_trace_grid` / `pixmith_remove_background` (pixmith_trace.h), as the
filter kind `PIXMITH_FILTER_REMOVE_BACKGROUND`, and as the import mode `{"mode": "trace_grid"}` of
`pixmith_import_image_ex`.

Every other Aseprite command (`Zoom`, `Cut`, `Copy`, `Paste`, `ShowGrid`, `Options`, ...) raises
`app.command.X is not available in Pixmith`. File commands ask for permission like the API calls do.

## Clean Edges and lint

Pixmith-specific tools for tidying sprites traced from reference art (they are not part of Aseprite's API). They are
also in the app: **FX > Clean Edges...** and **FX > Lint Sprite...**. Preserving small interior features (a 1-2 px
eye, a 1 px highlight, a 1 px finger touching the outline) is the main requirement: nothing is ever removed unless it
is reachable from the outside of the sprite, and every removed pixel must be a "simple point" (taking it away
cannot split the sprite, open a hole or merge holes).

`app.command.CleanEdges{ ... }` works on the active cel, limited to the selection, and is one undo step. Without any
pass option every pass runs with its defaults; otherwise only the passes you name run.

| Option | Meaning |
| --- | --- |
| `halo = true` or `{ rings = 2 }` | Remove light, grayish "paper halo" pixels from the outer rings (default 2). A pixel qualifies when it is light (luma >= 110) and grayish (saturation <= 30%), has pixels behind it that are at least 40 luma darker and not less colourful, and belongs to a connected group of 3 or more. `haloGain` and `haloSaturation` change the thresholds. |
| `despeckle = 4` | Remove 8-connected pieces smaller than 4 pixels that touch the outside of the sprite, as long as a bigger piece exists. A small piece sealed inside the body (a pupil in a socket) is kept. |
| `holes = 2` | Fill enclosed transparent areas smaller than 2 pixels with the surrounding colour. Off unless given: a transparent eye or window is a hole too. |
| `lightSpecks = true` | Also replace a lone light, grayish pixel whose 8 neighbours are all much darker. Off unless given: a 1 px highlight inside a dark eye looks exactly like this. |
| `thinOutline = true` or `{ thickness = 3 }` | Thin a dark outer outline that is 2 to `thickness` pixels thick to 1 pixel, keeping the pixels next to the body. Outlines thicker than `thickness` (and dark masses such as hair) are left alone. `outlineColor` (with `outlineTolerance`) names the outline colour; otherwise pixels with luma <= `outlineLuma` (default 70) are outline. The outline stays 8-connected and closed, nothing detaches, and notches that a 2x 3x3 closing would fill are restored. |
| `frames = "all"` / `"layers"` | Apply to every frame of the layer / every layer of the frame instead of the active cel. |

```lua
app.command.CleanEdges{ halo = true, despeckle = 4, holes = 2, thinOutline = true, frames = "all" }
```

`app.lint{ ... }` reads the sprite and returns the findings without changing anything. Options: `frames`
(`"all"` / `"layers"`), `speck` (size of a detached speck, default 4), `list` (positions listed per category, default
64; the counts are always complete), `palette = true` (also report colours missing from the palette), `strayTolerance` (default 48, see `strays`),
`categories` (a list of the category names below; only those are reported, the others come back empty), and
`outlineColor` / `outlineTolerance` / `outlineLuma` as above. The result is an array with one entry per cel plus
`total`; each entry has `layer` and `frame` (1-based), `layerName`, `total`, and these categories, each
`{ count, items = { { x, y, size }, ... } }` with 0-based pixel coordinates (for an area, its first pixel in scan order):

- `light_edge`: paper halo, exactly the pixels the default `CleanEdges` halo pass would remove (it reuses that pass: light, grayish, lighter than what is behind, in a group, on the outer rings). Light detail just inside an outline (a cream cuff) is not listed, so lint and `CleanEdges` never disagree. `halo` thresholds are the ones of `CleanEdges`.
- `holes`: enclosed transparent areas.
- `specks`: small pieces detached from the body.
- `no_outline`: edge pixels that are not outline colour and have no outline pixel next to them.
- `strays`: a lone off pixel in a flat area. It differs from every neighbour by more than `strayTolerance` (largest channel difference, default 48; 0 means any difference) while its neighbours (at least three solid) are alike: three quarters of them lie within half the tolerance of one another. Texture, dither and noise are therefore not strays (a textured 40-colour walk sheet used to list 52,639). It stays on by default because a flat-area stray is a real defect; use `categories` to leave it out. Isolated pixels are `specks`.
- `semi_transparent`: pixels with 0 < alpha < 255.
- `off_palette`: colours not in the palette (only with `palette = true`).

```lua
local r = app.lint{ frames = "all" }
for _, cel in ipairs(r) do
  print(cel.frame, cel.holes.count, cel.specks.count, cel.light_edge.count)
end
```

## Enumerations

`BlendMode` (`NORMAL` 0, `DARKEN`, `MULTIPLY`, `COLOR_BURN`, `LIGHTEN`, `SCREEN`, `COLOR_DODGE`, `ADDITION`, `OVERLAY`,
`SOFT_LIGHT`, `HARD_LIGHT`, `DIFFERENCE`, `EXCLUSION`, `SUBTRACT`, `DIVIDE`, `HSL_HUE`, `HSL_SATURATION`, `HSL_COLOR`,
`HSL_LUMINOSITY` 18 -- Aseprite's numbering; `layer.blendMode`, `drawImage` and `app.blend` use it), `ColorMode`
(`RGB` 0, `GRAY` = `GRAYSCALE` 1, `INDEXED` 2, `BITMAP` 3, `TILEMAP` 4), `FlipType` (`HORIZONTAL` 0, `VERTICAL` 1),
`AniDir` (`FORWARD` 0, `REVERSE`, `PING_PONG`, `PING_PONG_REVERSE` 3), `SpriteSheetType` (`HORIZONTAL` 1, `VERTICAL`,
`ROWS`, `COLUMNS`, `PACKED` 5), `SpriteSheetDataFormat` (`JSON_HASH` 0, `JSON_ARRAY`), `RangeType` (`EMPTY` 0,
`LAYERS`, `FRAMES`, `CELS` 3).

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

Constructors: `Image(width, height [, colorMode])`, `Image{ width=, height=, colorMode= }` / `Image(ImageSpec{...})` /
`Image(sprite.spec)`, `Image(size)`, `Image(otherImage)` (a copy) and `Image{ fromFile = path }`. `fromFile` needs read
permission and an absolute path; a PNG (any bit depth or color type; palette PNGs are expanded) or, for other
extensions, any image Pixmith opens (the first frame); at most 8192 x 8192 for PNG. Images are **always RGBA in
memory**; the `colorMode` given to `Image` (`ColorMode.RGB` 0, `GRAY` / `GRAYSCALE` 1, `INDEXED` 2) is remembered and
decides what the pixel functions return and take (see [Colors](#colors)): RGB `0xAABBGGRR`, grayscale
`gray | alpha << 8`, indexed a palette index of the active sprite. A `cel.image` of an Indexed or Grayscale sprite
speaks that mode. Writing an image into a cel of an Indexed or Grayscale sprite snaps its colors to the palette
or to gray.

Fields: `width`, `height`, `bounds` (Rectangle), `size` (Size), `colorMode`, `rowStride` (`width * 4`),
`bytesPerPixel` (4), `spec`, `cel` (the cel a `cel.image` view belongs to, else `nil`), and
**`bytes`** (read/write): the raw pixels as a Lua string, row-major, 4 bytes (R, G, B, A) per pixel. Assigning
checks the length (`width * height * 4` bytes exactly) and replaces every pixel in one native copy:

```lua
local img = Image(1024, 1024)
img.bytes = string.rep("\xff\x00\x00\xff", 1024 * 1024)   -- opaque red; a couple of VM instructions
```

Reading `img.bytes` creates a Lua string of that size, which counts toward the Lua memory limit.

Methods: `getPixel(x, y)` (`0` outside, a pixel value), `putPixel(x, y, value | Color)` (replace),
`drawPixel(x, y, value | Color)` (alpha-blend), `clear([value | Color | rectangle [, value | Color]])`, `clone()`,
`isEqual(image)`, `isEmpty()`, `isPlain(value | Color)`,
`drawImage(image, point [, opacity [, blendMode]])` (also `drawImage(image, x, y, ...)`; alpha-composites with the
image's top-left at `point`, clipped; opacity 0-255; `blendMode` as in `app.blend`),
`drawSprite(sprite, frame [, point])` (draws the composited frame), `resize(w, h)` / `resize(size)` (nearest
neighbour; not on `cel.image` views) and `pixels([rectangle])`:

```lua
for it in img:pixels() do      -- `it` is one object advanced in place, as in Aseprite
  if it() == 0xFF0000FF then it(0xFF00FF00) end     -- it() reads, it(value) writes; it.x, it.y (pixel values)
end
```

`img:flip([FlipType])` flips in place (horizontal by default); `img:shrinkBounds([color])` returns the `Rectangle`
around everything that is not transparent (or not that color); `img:saveAs(path)` writes any format Pixmith exports
(write permission).

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
- `scripting::run_in(editor, source, active, limits, cancel, &RunConfig { script_name, doc_path, allowed_paths })`
  names the script for permission questions, gives `sprite.filename`, and pre-allows path prefixes for that run;
  `Outcome` then also carries `new_documents`, `active`, `close_original`, `original_path`, `original_saved`
  (docs/LUA_HOST_API.md);
- `scripting::access::{set_permission_handler, set_allowed_paths}` and the C ABI
  `pixmith_script_set_permission_handler` / `pixmith_script_set_allowed_paths` for the permission questions;
  `pixmith_run_script_named` for a script name and document path;
- `scripting::host::Host::with_limits(limits)` for sessions (dialogs, events, plugins): the limits apply to each
  transaction;
- the C ABI `pixmith_run_script_limits(doc, source, layer, frame, instructions, time_ms, memory_mb, out_messages)`
  where `0` for a budget means its default.

Work done natively (PNG decoding, `bytes`, `drawImage`, `newCel`) is not interruptible and not counted by the
instruction budget, but it is bounded by the image size caps above.

## Aseprite compatibility

Pixmith opens `.aseprite` files, and the scripting API follows Aseprite's names and semantics where Pixmith has
the concept (reference: https://www.aseprite.org/api/, `app.apiVersion` 21), so a typical Aseprite script works unchanged.

**Supported (Aseprite spelling and meaning):**

- `Color` (every constructor, HSV / HSL / gray / index models, mutable properties, `==`, `tostring`), `app.pixelColor`;
  every color argument takes a `Color` or a Pixmith integer.
- `Point`, `Size`, `Rectangle`; `Image` (constructors, `bytes`, pixel functions in Aseprite's pixel format, `drawImage`,
  `drawSprite`, `clear`, `clone`, `isEqual`, `isEmpty`, `isPlain`, `pixels`, `resize`, `flip`, `shrinkBounds`, `saveAs`,
  `ImageSpec`); `Palette` (see above).
- Documents: `Sprite(w, h [, colorMode])`, `Sprite(ImageSpec)`, `Sprite(sprite)`, `Sprite{ fromFile }`, `app.open`, `app.sprites`,
  `app.activeSprite` (read and assign), `sprite:saveAs` / `saveCopyAs` / `close`, `sprite.filename`, `sprite.isModified`,
  `sprite.spec`, `sprite.colorMode` (a `ColorMode`), `sprite:flatten()`, `sprite:resize`, `sprite:crop`, `sprite:newEmptyFrame`.
- Files: `io`, `os` (`remove`, `rename`, `execute`, `tmpname`, `time`, `date`, `clock`, `getenv` for harmless variables),
  `dofile`, `loadfile`, all of `app.fs`, with Aseprite-style permission questions.
- `app.command.*` (the list above), `app.transaction`, `app.undo` / `redo`, `app.refresh`, `app.activeLayer` / `activeFrame` /
  `activeCel` / `activeImage` / `activeTag` / `site` / `range` (objects), `app.version` (a `Version`), `app.apiVersion`,
  `app.fgColor` / `bgColor` (`Color`), `app.alert`, `app.events`, `app.isUIAvailable`.
- `BlendMode`, `ColorMode`, `FlipType`, `AniDir`, `SpriteSheetType`, `SpriteSheetDataFormat`, `RangeType`; `layer.blendMode`
  uses `BlendMode` values; `layer.isVisible` can be assigned; `layer.stackIndex`, `isImage`, `isBackground`, `isEditable`,
  `layer.sprite`, `layer.data` / `color`; cels with `layer` / `frame` / `sprite` objects, `cel.image`, `cel.position`,
  `cel.bounds`, `cel.frameNumber`, `cel.data` / `color`; `frame.frameNumber`; `tag.aniDir`, `fromFrame`, `toFrame`, `frames`,
  `data`, `color` (a `Color`); `sprite.rootLayers`, `backgroundLayer`, `data`, `color`; `sprite:deleteLayer` / `deleteFrame` /
  `deleteTag` / `deleteSlice` with handles or names; `sprite:newFrame(frame)`; `sprite:newCel`, `sprite:newSlice(rectangle)`,
  `slice.bounds` / `center` / `pivot`; selection `select` / `add` / `subtract` / `intersect` / `contains(point)` / `origin` /
  `bounds`; layers, groups, frames, tags, tilesets and tilemaps, user data, `Dialog` (see [LUA_DIALOG.md](LUA_DIALOG.md)),
  plugins (`init` / `exit`, `newCommand`).

**Known differences** (everything else in Aseprite's API is not implemented):

- **Colors** have three number formats (see [Colors](#colors)): older Pixmith functions keep `0xRRGGBBAA`, while `Color(n)`,
  `app.pixelColor` and `Image` pixels use Aseprite's `0xAABBGGRR`. `app.fgColor`, `bgColor`, `tag.color`, `userData.color`
  and dialog color values are `Color` objects now: `app.fgColor == 0xFF0000FF` is false, use `app.fgColor == Color(255, 0, 0)`.
  `sprite:getPixel`, `getPixels`, `getPaletteColor` still return integers. `app.blend.*`, `layer.blendMode` and
  `drawImage` blend modes now use Aseprite's numbers (symbolic names are unchanged; raw numbers 0-18 mean something else).
  `sprite.colorMode` is a `ColorMode` number, not a string. `app.activeLayer` / `app.activeFrame` are handles, not numbers
  (`sprite.activeLayer` / `activeFrame` are the numbers); `cel.layer` / `cel.frame` are handles too (`cel.frameNumber` is the number).
- **Images are always RGBA in memory.** An indexed image has no palette of its own: its index values refer to the palette
  of the active sprite. `Image(w, h, ColorMode.INDEXED)` stores RGBA colors (index values are converted on the way in
  and out).
- **Only the canvas is drawn on.** A cel's content may extend beyond the canvas (and files keep it), but drawing
  tools, filters, `sprite:getPixel` / `setPixel` / `setPixels` and exports work on the canvas; use `cel.image` to
  reach the parts beyond the edges. Tilemap cels cannot extend beyond the canvas. `cel.image` is `nil` for a cel that is
  fully transparent, so a fresh `Sprite(w, h)` has a cel but `cel.image` is `nil` until something is drawn (use
  `Image(sprite.spec)` and `sprite:newCel`).
- `sprite:cel(layer, frame)` always returns a handle (Aseprite returns nothing for an empty cel; `layer:cel(frame)`
  and `app.activeCel` do return `nil`).
- `slice.bounds`, `.center` and `.pivot` read and write the first frame's key; `newSlice()` without a rectangle
  covers the whole canvas.
- **Layers, frames and tags are indices or lightweight handles**, with the stale-handle rule above (creating a layer or
  frame with `app.command` in the middle of the stack invalidates handles above it). `sprite.layers` is the flat list
  with children (Aseprite's lists only top-level layers; use `rootLayers` and `layer.children`).
- **One palette per sprite**: `sprite.palettes` has one entry; there are no palette frames or palette resources.
- **Documents**: `app.sprites` only knows the document the script runs on and the ones it created or opened (the editor's
  other tabs are not visible); `Sprite{ fromFile }` loads a new sprite even if the file is open elsewhere in the run.
  `saveAs` to an export format does not change `filename`; a multi-frame sprite saved as a still format writes
  `name-001.png`, `name-002.png`, ... (Aseprite has its own numbering scheme for sequences). Only the formats Pixmith reads and writes work. `app.open` does not show a dialog and
  returns `nil` for unreadable files.
- **`io` / `os`**: standard input does not exist; `os.exit`, `os.setlocale`, `package`, `require`, `debug` are gone; `os.getenv`
  answers for a short whitelist only; no `io.stdout` capture (use `print`).
- **`app.command`** has the commands listed above; all other commands (clipboard, view, UI) raise an error. `Undo` / `Redo` and
  `app.transaction` only cover what the script itself did. Filters act on the active cel inside the selection's mask
  (Aseprite acts on the selected range of cels).
- **Not implemented at all**: `app.useTool`, `app.clipboard`, `app.window`, `app.preferences`, `app.theme`, `app.tool`,
  `app.brush`, `app.defaultPalette`, `app.websocket`, `Brush`, creating tilesets, `sprite:assignColorSpace` / `colorSpace`,
  `sprite.gridBounds` / `pixelRatio`, `Image:setPalette`, the `id` property of cels, layers, sprites and images. `app.range`
  always describes the active cel (the editor's timeline selection is not visible to scripts).
- `Point` supports `+`, `-`, unary `-` and `==`; `Size` and `Rectangle` only `==` and `tostring` (no `Point * n` or `Size + Size`).

## `color`

The Pixmith helper table for `0xRRGGBBAA` integers (its functions also take `Color` objects):

- `color.rgba(r, g, b [, a])` packs 0-255 channels (alpha defaults to 255).
- `color.split(c)` returns `r, g, b, a`.
- `color.toPixel(c)` converts a Pixmith integer or `Color` to an Aseprite pixel value (`0xAABBGGRR`);
  `color.fromPixel(px)` converts back.

## Examples

`scripts/lua-examples/` contains runnable examples: `outline-layer.lua`, `color-ramp.lua`,
`flip-all-frames.lua`, `palette-swap.lua`, `tag-every-4-frames.lua` and `palette-ramp-dialog.lua`
(a dialog with a live ramp preview). `plugin-hello.lua` is a plugin example (a command with a dialog,
preferences and an event); it is loaded as a plugin, not run from File > Scripts. `plugin-events.lua`
tracks site changes, color changes and edits and offers a command that reports them.

## Embedding notes

Documents the script created and permission questions are covered in [LUA_HOST_API.md](LUA_HOST_API.md): for the
synchronous entry points `Outcome` carries the new documents (`pixmith_script_outcome(0, 0)` and
`pixmith_script_take_doc(0, 0, i)` through the C ABI), and sessions report them in the `txn` message.

`pixmith_run_script_at` returns the alert and `print` lines joined with `\n`. When a successful script
assigned `app.activeLayer` or `app.activeFrame`, one more line is appended:
`\u{1}PIXMITH-SELECT layer=<n> frame=<n>` (1-based, after a `U+0001` control character, which is
stripped from script output so scripts cannot fake it). The editor selects that layer and frame and
removes the line before showing the messages.
