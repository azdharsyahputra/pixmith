# Extensions

An extension is a `.pixmith-extension` file, like Aseprite's `.aseprite-extension`: a zip with a `package.json` at its
root that adds **scripts** (Lua plugins), **palettes** and **color themes**. Both front ends manage them in
**Preferences > Extensions**; a package can also be opened (File > Open, Finder or Explorer) or dropped on the window.
`scripts/extension-template/` is a working sample with a packing script.

## Package

```json
{
  "name": "sample-extension",
  "displayName": "Sample Extension",
  "version": "1.0.0",
  "author": "Pixmith contributors",
  "description": "A palette, a color theme and a plugin command.",
  "contributes": {
    "scripts":  [ { "name": "Sample commands", "path": "scripts/sample.lua" } ],
    "palettes": [ { "name": "Sunset",          "path": "palettes/sunset.gpl" } ],
    "themes":   [ { "name": "Midnight",        "path": "themes/midnight.json" } ]
  }
}
```

| Field | Rules |
| --- | --- |
| `name` | required; the identity and folder name: lower case letters, digits, `.`, `-`, `_` (starts with a letter or digit, at most 64) |
| `version` | required text, at most 32 characters |
| `displayName`, `author`, `description` | optional text (the display name defaults to `name`) |
| `contributes.scripts` | Lua files (`.lua`); each is loaded as a **plugin** (see `docs/LUA_DIALOG.md`, "Plugins") |
| `contributes.palettes` | `.gpl` (GIMP) or `.hex` files; they join the palette presets |
| `contributes.themes` | theme JSON files (below) |

A contribution is a path (`"scripts/a.lua"`, the label is the file name) or `{ "name": "Label", "path": "..." }`; at most
64 per kind. Paths are relative to the package root and use `/`.

## Install, enable, uninstall

Packages unpack into `<app data>/Extensions/<name>/` (the same `<app data>` as scripts and recovery files; see
`docs/LIVE_PROTOCOL.md`, "Discovery", and `PIXMITH_SUPPORT_DIR`). Installing an extension that is already installed
updates it in place and keeps its enabled state. **Enabled** is stored in `Extensions/state.json`; a disabled
extension contributes nothing. **Uninstall** deletes the folder.

Installing validates everything first and writes nothing if anything is wrong:

- file names: no absolute paths, drive letters, `..`, backslashes, control characters, Windows device names, trailing
  dots or spaces, duplicates (case-insensitive), or a name that is both a file and a folder; at most 200 bytes and 8
  folders deep;
- no encryption, zip64 or multi-part archives, no symbolic links or special files, only stored and deflated entries;
- limits: package 32 MB, 512 entries, 8 MB per file, 48 MB unpacked, deflate ratio checked against the header and the
  inflated size, CRC of every file;
- `package.json` valid, every contributed file present, and each one parses (palettes, themes; scripts are UTF-8 text).

Files are written with `create_new` into a staging folder that is renamed into place in one step; an installed folder
that a user edits by hand is re-read with the same checks (links are never followed), and a broken one is listed with
its error and can still be uninstalled.

## Themes

```json
{
  "name": "Midnight",
  "accent": "#FF6AC1",
  "dark":  { "panel": "#171724", "text": "#EDEDF7" },
  "light": { "panel": "#F4F1FA" }
}
```

`dark` and `light` override the semantic colors of `DSColor` (Swift) and `Colors` (egui `theme.rs`). Colors are `#RGB`,
`#RRGGBB` or `#RRGGBBAA`. Tokens: `workspace`, `panel`, `panel2`, `recessed`, `elevated`, `separator`, `separatorSoft`,
`field`, `hover`, `pressed`, `accent`, `destructive`, `guide`, `overlay`, `canvasShadow`, `text`, `textSecondary`,
`textTertiary`. A token that is not mentioned keeps its default; the selected fills and accent outline follow `accent`.
A top-level `accent` applies to both appearances. A theme may style only one appearance; picking it then switches
Light/Dark to the one it styles. Unknown tokens and malformed colors reject the theme.

**Preferences > Appearance > Color theme** lists Default and the themes of the enabled extensions and switches live. The
choice is saved by theme id (`<extension>/<path>`); if its extension is disabled the app shows the default look and
brings the theme back when the extension is enabled again.

## Palettes

Contributed palettes are appended to the built-in presets (`Palette > Presets`, the palette actions menu) as
"Name (Extension)". Loading one is a normal preset load: one undo step.

## Programmatic access

- Rust: `pixmith_core::extensions::{install, installed, set_enabled, uninstall, scripts, themes, palettes, refresh,
  generation}` and `Store` for any folder; palette presets through `palette::presets::{names, colors}`.
- C: `app/Sources/PixmithCore/pixmith_ext.h` (`pixmith_ext_list`, `_install`, `_set_enabled`, `_uninstall`, `_themes`,
  `_scripts`, `_dir`, `_refresh`); results are JSON.
