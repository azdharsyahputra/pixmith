# Pixmith for Windows, Linux and macOS

This is the cross-platform front end of Pixmith, written in Rust with [egui](https://github.com/emilk/egui). It links the Rust core in `../core` as a normal dependency and is designed to look and behave exactly like the native macOS app in `../app`: same layout, same design tokens (`src/theme.rs` mirrors `DesignSystem.swift`), same core calls. Only the platform's native file dialogs and system font differ.

Pixmith is a lightweight pixel-art editor. Its own document format is `.pxmt`; `.aseprite` files can be opened, saved back and exported (File > Export As Aseprite). See the [main README](../README.md) for what it does.

## Download

Prebuilt builds are on the [Releases](https://github.com/azdharsyahputra/pixmith/releases) page:

- Windows 10 or newer (x64): `Pixmith-<version>-windows-x64.zip`. Unzip, run `Pixmith.exe`. If SmartScreen warns about an unknown publisher, choose More info, then Run anyway (the build is not code-signed yet).
- Linux (x64): `Pixmith-<version>-linux-x64.tar.gz`. Unpack, run `./Pixmith`. It is built on a current Ubuntu, so it needs a similarly recent glibc, and a GPU driver with Vulkan or OpenGL.
- macOS: use the native app (`Pixmith-<version>.dmg`), or build this one yourself.

## Build and run

You need a stable Rust toolchain ([rustup.rs](https://rustup.rs)). The core bundles Lua, so a C compiler must be on the path as well (Visual Studio Build Tools on Windows, Xcode Command Line Tools on macOS, `build-essential` on Linux).

```sh
cd desktop
cargo run --release
```

### Windows

Install the "Desktop development with C++" workload of the Visual Studio Build Tools, then run the commands above in PowerShell or `cmd`. The release build embeds the icon (`assets/pixmith.ico`) and product name into `pixmith-desktop.exe`; rename the file to `Pixmith.exe` if you like (the release workflow does).

### Linux

Windowing, graphics and the file dialog are loaded at run time, so the only build-time packages are the usual ones:

```sh
sudo apt-get install -y build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev \
  libxcursor-dev libxi-dev libxrandr-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libgl1-mesa-dev libvulkan-dev
```

On Fedora the equivalents are `libxkbcommon-devel wayland-devel libX11-devel libXcursor-devel libXi-devel libXrandr-devel mesa-libGL-devel vulkan-loader-devel`. At run time you need a Vulkan or OpenGL driver and, for the Open and Save dialogs, `xdg-desktop-portal` with a backend for your desktop (GNOME, KDE and most others ship one).

### macOS

Nothing beyond the Xcode Command Line Tools. This builds the egui app; the SwiftUI app is built with `../build.sh`.

## Tests and screenshots

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                                # file actions, tabs, quit flow, exports (no GPU needed)
cargo test --test screens -- --ignored    # renders the window to target/screens/<name>-<dark|light>.png
```

The `screens` test draws the real UI headlessly at 1280 x 800 points (2x) in both themes, the same size as the macOS screenshots in `../docs/design/screens/`. Compare `target/screens/main-dark.png` with `main-dark@2x.png` and `tabs-dark.png` with `tabs-dark@2x.png`. It needs a GPU (wgpu), so CI only compiles it.

## Source map

Each file mirrors a Swift file, so a change in one app is easy to carry over to the other.

| File | Swift counterpart |
| --- | --- |
| `src/theme.rs` | `DesignSystem/DesignSystem.swift` (tokens, colors, fonts) |
| `src/ui/widgets.rs` | the `DS*` controls: buttons, `DSDialog`, `DSFormRow`, `DSNumberField`, `DSPopup`, `IconButton` |
| `src/app.rs` | `App/ContentView.swift` layout, `Tool`, per-tab state |
| `src/doc.rs`, `src/calls/*.rs` | `Model/SpriteModel.swift`: safe wrappers over the `pixmith_*` entry points (all `unsafe` lives here) |
| `src/ui/title_row.rs` | `Tools/TopBar.swift` `TitleRow`, `App/Documents.swift` `TabBar` |
| `src/ui/dialogs.rs`, `src/calls/files.rs` | `NewSpriteSheet`, `App/FileActions.swift`, `App/Documents.swift` |
| `src/ui/tool_rail.rs` | `Tools/ToolsPanel.swift` |
| `src/ui/tilemap/*.rs`, `src/calls/tilemap.rs` | `Panels/TilemapView.swift`, `Panels/AutotileView.swift`, `Canvas/SlicesView.swift` (tileset, terrain brush, slices, their sheets and canvas overlays) |
| `src/ui/gamedev/`, `src/calls/gamedev.rs` | `Panels/TweenView.swift`, `Dialogs/GodotExport.swift`, `Dialogs/GodotTilesetExport.swift`, `App/Scripts.swift`, `Autosaver` in `App/FileActions.swift`, `ProgressSheet.swift` |
| `src/ui/menus/` (`bar.rs`, `commands.rs`) | the menus of `PixmithApp.swift` and every `CommandMenu` in `app/Sources/Pixmith` |
| `src/ui/menus/filters.rs`, `calls/menus.rs` | `Dialogs/FiltersView.swift`, `Dialogs/CurvesEditor.swift`, `Dialogs/ColorModeView.swift` |
| `src/ui/menus/history.rs`, `prefs.rs`, `shortcuts.rs`, `help.rs` | `Panels/HistoryPanel.swift`, `App/Preferences.swift`, `App/Shortcuts.swift` |
| `src/ui/projects/`, `src/calls/projects.rs` | Home (recent canvases and projects), the project view (canvas grid, trash, reorder), the editor's project switcher; the `.pxmt` and project core calls |
| `src/ui/context_bar.rs`, `canvas.rs`, `inspector.rs`, `timeline.rs` | `Tools/TopBar.swift` `ContextBar`, `Canvas/`, `Panels/Inspector.swift`, `Panels/Timeline.swift` |

## Status: phase 1

Works now:

- Window with the macOS app's layout and both themes, app icon and the Pixmith name on every platform.
- Title row: document tabs with the unsaved-changes dot, close button on hover, middle-click and a context menu to close, the active tab on the lifted panel color, "+" for a new tab; zoom out, a zoom menu (presets, Zoom to Fit, Actual Size) and zoom in, bound to the active tab; the blue Export menu.
- New sprite sheet (width and height, 1 to 4096), Open (`.aseprite`, `.ase` and `.png`, which opens as a new unsaved sprite), Save, Save As (`.aseprite`), and dropping files on the window. An already open file switches to its tab.
- The unsaved-changes question (Save, Don't Save, Cancel) when closing a dirty tab or the window; closing the last tab leaves a fresh untitled sprite.
- Export of PNG (current frame), sprite sheet (strip or grid, padding, scale, JSON), animated GIF, animated PNG and PNG sequence, all through the same core calls as the macOS app.
- Errors from the core show in a dialog.

| Shortcut | Action |
| --- | --- |
| Ctrl/Cmd+N | New sprite |
| Ctrl/Cmd+O | Open |
| Ctrl/Cmd+S | Save |
| Ctrl/Cmd+Shift+S | Save As |
| Ctrl/Cmd+W | Close tab |
| Ctrl/Cmd+= and Ctrl/Cmd+- | Zoom in and out |

Not in phase 1: the macOS app's other import commands (GIF, sprite sheet, PNG as layer), Open Recent, autosave and crash recovery, the Export options and Godot export sheets, tab reordering by drag, and the panels and canvas tools that are still being ported (see the Swift sources for the full target). Exports run on the UI thread, so very large sprites pause the window while they write.

## Projects and `.pxmt`

Pixmith opens on Home (File > Home, Ctrl+Shift+H, brings it back; the open tabs stay). A project is a folder of canvases with a `.pixmith` folder holding its name, canvas order and trash. Opening a file directly (drag and drop, File > Open, or `Pixmith.exe file.pxmt` on the command line, which is also how Windows "Open with" works) skips Home. Save and Save As write `.pxmt`; a document opened from an `.aseprite` file is saved back to that file; File > Export As Aseprite writes a copy.
