# Contributing to Pixmith

Thanks for helping. This page covers how to build, test and send a change.

## Setup

- A stable Rust toolchain (`rustup` recommended). This is all the core and the CLI need, on any platform.
- For the macOS app: macOS 13 or newer with Xcode or the Command Line Tools (Swift 5.9+).
- For the desktop (egui) app: nothing more on Windows and macOS. On Linux install the packages listed in [desktop/README.md](desktop/README.md#linux).
- Optional: Godot 4 for `scripts/godot-check.sh`.

```sh
./build.sh            # core + app -> build/Pixmith.app, build/pixmith-cli
open build/Pixmith.app
```

## Where things live

- `core/` is the Rust library. It owns the document and every pixel operation; the app never edits pixels itself. Each feature exposes `#[no_mangle] extern "C" fn pixmith_*` entry points next to its Rust code, and the matching declarations live in `app/Sources/PixmithCore/*.h`.
- `app/` is the SwiftUI/AppKit app. `Sources/Pixmith` (built as `PixmithKit`) is grouped by area: `App`, `Model`, `Canvas`, `Tools`, `Panels`, `Dialogs`, `DesignSystem`. `Sources/PixmithMain` is the `@main` entry point and `Sources/PixmithQA` the end-to-end scenario runner.
- `desktop/` is the cross-platform front end for Windows, Linux and macOS, written in Rust with egui. It links `core/` as a normal Rust dependency and calls the same `pixmith_*` functions as the Swift app, through safe `Doc` methods. It is built to look and behave exactly like the macOS app, so its files map one to one onto the Swift files:

  | `desktop/src/` | Swift counterpart (`app/Sources/Pixmith/`) |
  | --- | --- |
  | `theme.rs` | `DesignSystem/DesignSystem.swift` tokens (`DS`, `DSColor`) |
  | `ui/widgets.rs` | the `DS*` controls: buttons, `DSDialog`, `DSFormRow`, `DSNumberField`, `DSPopup` |
  | `app.rs` | `App/ContentView.swift`, the `Tool` enum and tab state of `Model/SpriteModel.swift` |
  | `ui/title_row.rs`, `ui/dialogs.rs`, `calls/files.rs` | `Tools/TopBar.swift` `TitleRow`, `App/Documents.swift`, `App/FileActions.swift`, the sheets |
  | `ui/tool_rail.rs` | `Tools/ToolsPanel.swift` |
  | `ui/context_bar.rs`, `ui/canvas.rs` | `Tools/TopBar.swift` `ContextBar`, `Canvas/` |
  | `ui/inspector.rs`, `ui/timeline.rs` | `Panels/Inspector.swift`, `Panels/Timeline.swift` |
  | `doc.rs`, `calls/*.rs` | `Model/SpriteModel.swift` calls into `PixmithCore` |

  When you change how something looks or behaves in one front end, change the other. Keep every `unsafe` block in `doc.rs` and `calls/`.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) explains the document model, undo history, threading and the C ABI conventions.

## Checks to run before a pull request

```sh
cd core
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cd ..
./scripts/qa.sh       # headless end-to-end scenarios against the real app code
```

Run `./scripts/godot-check.sh` too when you touch Godot export.

For the desktop app:

```sh
cd desktop
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --test screens -- --ignored   # needs a GPU; renders the window to target/screens/*.png
```

The `screens` test renders the real UI headlessly in both themes at the size of the macOS screenshots in `docs/design/screens/`. Open the PNGs next to the matching macOS screenshot (`main-dark@2x.png`, `tabs-dark@2x.png`, ...) and compare; a change to the desktop UI is done when the two match. Add a scenario to `desktop/tests/screens.rs` for any new screen or dialog.

## Guidelines

- Keep pixel logic in `core/`, with a Rust test next to it. Swift turns input into core calls and shows the result.
- Adding a C entry point:
  - Wrap its body in `crate::ffi::guarded(name, doc, || ...)`; a test fails if you forget.
  - Tolerate null pointers and out-of-range indices.
  - Declare it in the matching header under `app/Sources/PixmithCore/`.
- An undoable edit calls `checkpoint()` once before it changes the document. Multi-part operations go in a transaction so they undo as one step.
- Tests must never touch your real app data (`~/Library/Application Support/Pixmith` on macOS). The QA runner and the Rust tests use a temporary `HOME` and `PIXMITH_SUPPORT_DIR`; new tests should do the same.
- `.aseprite` support follows the public file format specification. Do not copy code from Aseprite or decompile its binaries.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/), for example `fix(undo): ...` or `feat(godot): ...`.

## Adding a front end for another platform

The core does all the work, so a Windows or Linux front end is mostly UI:

1. Build `core/` as a static or dynamic library (`crate-type` already includes `staticlib`; add `cdylib` if your toolkit prefers a DLL or `.so`).
2. Use the headers in `app/Sources/PixmithCore/*.h` as the API. They are plain C, so they work from C, C++, C# (P/Invoke), Rust, Zig and others. `docs/ARCHITECTURE.md` describes the conventions (opaque `PixmithDoc*`, `0xRRGGBBAA` colors, `pixmith_last_error`, strings freed with `pixmith_string_free`).
3. Mirror what the macOS app does in `app/Sources/Pixmith/Model/SpriteModel.swift`: call the core, then re-render from `pixmith_render` / `pixmith_render_rect`.
4. Put it in its own top-level folder with its own README and build script, and add a CI job for it. `desktop/` is the model: a Rust front end that calls the core directly.

Windows and Linux already have a front end, `desktop/`; contributions to it are the quickest way to bring those platforms up to the macOS app.

Open an issue first if you plan one, so we can move the shared headers somewhere neutral if that helps.

## Reporting bugs

Open an issue with your OS version, the steps to reproduce and, if you can, a small `.aseprite` file that shows the problem.
