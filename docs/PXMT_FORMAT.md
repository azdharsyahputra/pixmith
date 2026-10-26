# The `.pxmt` format

`.pxmt` is Pixmith's own document format and the default for Save. A `.pxmt` file holds everything a Pixmith document holds, plus a small preview image so that project browsers can show thumbnails without decoding documents. `.aseprite` / `.ase` stay supported: Open imports them and *Export As Aseprite* writes them (`pixmith-cli convert` converts either way).

Implementation: `core/src/formats/pxmt.rs`. Version of this specification: **2** (files that do not use isometric tilemaps are still written as version 1).

## Design goals

- **Everything round-trips.** The sprite is stored as an ordinary `.aseprite` file, written and read by the battle-tested `formats::ase` code, so every feature that survives `.aseprite` survives `.pxmt`: layers, groups, blend modes, linked cels, tags, slices, palettes, color modes, tilesets and tilemaps, user data.
- **Cheap to browse.** Size, frame and layer counts (`META`) and a thumbnail (`THMB`) can be read without touching the sprite data.
- **Robust.** Every chunk carries a CRC-32; truncated or corrupted files produce a clear error, never a panic, and damage to the optional chunks never prevents opening the document.
- **Forward compatible.** Unknown chunks are skipped, unknown `META` members are preserved on save, and a file from a future version is refused with an explanation instead of being misread.

## Layout

All integers are little-endian.

```
file   = magic version chunk*
magic  = 50 58 4D 54 0D 0A 1A 0A          "PXMT" CR LF Ctrl-Z LF  (8 bytes)
version = u16                              1 for this specification
chunk  = tag[4] length:u32 payload[length] crc:u32
```

- The magic is modelled on PNG's: the CR LF / LF pair and Ctrl-Z reveal text-mode transfers that would corrupt the file. Readers identify a `.pxmt` by the magic, never by the file name.
- `crc` is the CRC-32 (IEEE 802.3, as in zlib/PNG) of `tag` followed by `payload`.
- The chunk list ends with a `PEND` chunk (empty payload). A file whose chunk list ends without `PEND` is truncated.

A reader that finds a `version` greater than it knows must refuse the file ("newer than this Pixmith reads"); version 0 is invalid. Changes that old readers can safely ignore are made by adding chunks or `META` members; only incompatible changes bump the version.

Version history:

| Version | Written when | Adds |
| --- | --- | --- |
| 1 | the document has no isometric tilemap | everything above |
| 2 | the document has at least one isometric tileset | isometric tilemaps (see below) |

A writer picks the lowest version that can hold the document, so documents that do not use the newer feature stay readable by older Pixmith versions; an older version meeting a version 2 file refuses it with the "newer than this Pixmith reads" message instead of showing isometric layers as square ones. Readers of this specification read both versions.

## Chunks

Writers emit `META`, `THMB`, `SPRT`, `PEND` in this order (so a reader can stop early), but readers accept any order and use the first chunk of each known tag. Tags are four ASCII characters. Unknown tags are skipped without reading their payload; their checksum is not verified.

### `META` (optional) — UTF-8 JSON object

```json
{"v":1,"generator":"pixmith-core 0.1.0","created":1729700000,"modified":1729700500,
 "width":32,"height":32,"frames":4,"layers":3}
```

| Member | Meaning |
| --- | --- |
| `v` | Metadata version (1). |
| `generator` | Program and version that wrote the file. |
| `created`, `modified` | Seconds since the Unix epoch. `created` is carried over when a file is saved again; `modified` is the time of the save. |
| `width`, `height`, `frames`, `layers` | Canvas facts for project browsers. If a member is missing or zero, a reader falls back to decoding `SPRT`. |

Readers ignore members they do not know. A writer that re-saves a file keeps the unknown members it read from the old `META`. At most 1 MiB.

### `THMB` (optional) — PNG image

The first frame, all visible layers composited, as an RGBA8 PNG whose longer side is at most **256 pixels** (smaller canvases are stored 1:1; larger ones are scaled down with nearest-neighbour sampling, keeping the aspect ratio). Readers reject thumbnails larger than 1024 pixels on a side or 16 MiB. If `THMB` is missing or damaged, previews are rendered from `SPRT`.

### `SPRT` (required) — the document

The complete document as an `.aseprite` file (header `0xA5E0`, frames, chunks), exactly what `formats::ase::write` produces, including Pixmith's use of user data and slice chunks. Cel pixel data inside is already zlib-compressed by that format, so the chunk is not compressed again. The payload can be extracted and opened by any tool that reads `.aseprite`.

### `PEND` — end marker

Empty payload. Everything after it is ignored.

## Pixmith-only data

Pixmith keeps some document data that `.aseprite` has no field for. All of it is stored in `.aseprite` user data (sprite-level and layer-level text), which is why it lives inside `SPRT` and needs no chunk of its own:

| Data | Where it lives |
| --- | --- |
| Tween tracks (keyframed layer transforms) | the layer's user data text, a versioned line after the user's own text (`gamedev/tween.rs`) |
| Tween rig links (a layer's tween parent) | the same tween line, as `id=` and `pa=` members (below) |
| Terrains and auto-tile intent | the tilemap layer's user data text, a `pixmith.autotile.v1:` line (`gamedev/autotile.rs`) |
| Godot export settings | the sprite's user data text, a `pixmith-godot-export ` line (`gamedev/godot/settings.rs`) |
| Isometric tilemaps: cell size, tile pivots, per-layer grid offset | the sprite's user data text, a `pixmith.iso.v1:` line (`document/isoext.rs`); see below |

### Isometric tilemaps (`pixmith.iso.v1:`)

Aseprite has no isometric tilemaps, so Pixmith stores what it cannot express as one line of the sprite's user data text, after the user's own text. Pixmith removes the line when it reads the file (the user never sees it) and writes it again on save. A document without isometric tilesets has no such line and is written as `.pxmt` version 1.

```
pixmith.iso.v1:{"v":1,"tilesets":[{"i":0,"tw":96,"th":96,"cw":96,"ch":48,"layout":"diamond_down","pivots":[[3,48,72]]}],"layers":[{"i":2,"x":592,"y":360}]}
```

| Member | Meaning |
| --- | --- |
| `v` | Version of this line's payload (1). A line with a higher `v` is ignored. |
| `tilesets[].i` | Index of the isometric tileset (its Tileset chunk ID). |
| `tilesets[].tw`, `th` | The tileset's tile image size; an entry whose size does not match the tileset is ignored. |
| `tilesets[].cw`, `ch` | The diamond cell size in pixels (1 to 4096; Pixmith rounds odd values up to even). 2:1 is the usual shape. |
| `tilesets[].layout` | Cell layout. Only `diamond_down` (Godot `TILE_LAYOUT_DIAMOND_DOWN`): cell `(i, j)` has its box at `(x + (i - j) * cw / 2, y + (i + j) * ch / 2)`. |
| `tilesets[].pivots` | Tiles whose pivot is not the default, as `[tile, x, y]`: the point of the tile image (pixels from its top-left corner) that sits on the diamond's center. The default is `(tw / 2, th - ch / 2)` for images taller than the diamond and `(tw / 2, th / 2)` otherwise. |
| `layers[].i`, `x`, `y` | Tilemap layer `i` (an isometric one) has the grid offset `(x, y)`: the canvas pixel of cell (0, 0)'s box. Layers at offset (0, 0) are left out. |

Entries that do not fit the file (unknown index, wrong size, out-of-range numbers, a layer that is not an isometric tilemap) are skipped; unknown members are ignored.

In the file the layers are ordinary Aseprite tilemap layers: the Tileset chunk holds the tile images (all `tw` x `th`), and each tilemap cel stores the grid of tile values as usual, except that the cel's position is the cell coordinate `(i, j)` of the grid's first cell instead of a pixel position (Aseprite, which ignores the line, shows such a layer as a square grid). Pixel positions of the tile images come only from the line above. The cels' pixels are not stored for tilemaps; they are rendered from the grid when the file is read: cells are drawn back to front by `i + j`, then by `i`, with source-over blending, each tile image placed so that its pivot is on the center of the cell's diamond.

### Tween payload and the rig link

The tween line is `\u001e` (record separator) `pixmith.tween.v1:` followed by `;`-separated `name=value` members: `rs`, `ss`, `oo` (flags), `ex=hold|loop` (what the layer shows after its last key: the last pose, or the key range repeated; **absent means `rest`**, the layer's own cels, which is how every file written before this member existed behaves, so those files render unchanged; a new tween writes `ex=hold`; older Pixmith versions ignore the member and show the rest pose), `pv=x,y` (pivot), the tracks `px`, `py`, `rot`, `sx`, `sy`, `op` (`frame,value,ease` keys joined by `~`), and, since the rig was added, **`id=N`** and **`pa=N`**. A layer that has children gets `id` (a positive integer, unique in the sprite, assigned the first time a child is linked); a child stores its parent's number as `pa`. Layers are never referenced by index, so reordering, inserting and deleting layers keeps links intact, and everything stays inside the layer's own user data text (it is plain `.aseprite` user data: Aseprite shows it and keeps it, and ignores its meaning).

Compatibility: this is an additive change inside payload version 1, so no version number changed. Files without `id`/`pa` read and write byte for byte as before. An older Pixmith skips the two members it does not know (the layers then move on their own, unparented) and drops them when it saves the file again. A `pa` that names no layer is ignored; if two layers carry the same `id` (hand editing, copied layers) the lowest layer is the parent; a cycle in a hand edited file is read with the walk stopping where it would revisit a layer; `Duplicate Layer` gives the copy no `id`, so the children keep following the original.

Future Pixmith-only data that does not fit user data would get its own chunk (or a `META` member) and be ignored by older readers.

## Reading efficiently

| Need | Read |
| --- | --- |
| Name, size, frame and layer count, timestamps | `META` only: `pxmt::read_meta_file(path)` |
| A preview | `THMB` only: `pxmt::read_thumbnail_file(path)` or `pxmt::file_thumbnail(path, max_w, max_h)` |
| The document | `SPRT`: `pxmt::open_file(path)` / `pxmt::read(bytes)` |

The first two seek over the sprite data: reading them costs a few small reads however large the document is.

## Writing

`pxmt::save_file(path, &sprite)` (and `pixmith_save`) writes `.pxmt` when the path ends in `.pxmt` (any case) and `.aseprite` otherwise, through `formats::durable::write_file`: a temporary file next to the target, flushed, then renamed over it, so an interrupted save leaves the old file intact.

## Errors a reader reports

| Situation | Message (abridged) |
| --- | --- |
| Not a `.pxmt` (wrong magic, too short) | "not a Pixmith document (.pxmt)" |
| Version newer than supported | "this file is .pxmt version N, newer than this Pixmith reads (version 1); update Pixmith" |
| Chunk longer than the rest of the file, or list ends without `PEND` | "the file is truncated (...)" |
| CRC mismatch | "the SPRT chunk is corrupt (checksum mismatch)" |
| No `SPRT` | "the file has no sprite data (SPRT chunk)" |
| `SPRT` is not a valid sprite | "sprite data: ..." with the `.aseprite` reader's reason |

A file that is complete up to and including a valid `SPRT` chunk opens even if the `PEND` marker is cut off.
