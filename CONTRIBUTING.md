# Contributing to Pixmith

Thanks for helping. This page covers how to build, test and send a change.

## Setup

- A stable Rust toolchain (`rustup` recommended). This is all the core and the CLI need, on any platform.
- For the macOS app: macOS 13 or newer with Xcode or the Command Line Tools (Swift 5.9+).
- Optional: Godot 4 for `scripts/godot-check.sh`.

```sh
./build.sh            # core + app -> build/Pixmith.app, build/pixmith-cli
open build/Pixmith.app
```

## Where things live

- `core/` is the Rust library. It owns the document and every pixel operation; the app never edits pixels itself. Each feature exposes `#[no_mangle] extern "C" fn pixmith_*` entry points next to its Rust code, and the matching declarations live in `app/Sources/PixmithCore/*.h`.
- `app/` is the SwiftUI/AppKit app. `Sources/Pixmith` (built as `PixmithKit`) is grouped by area: `App`, `Model`, `Canvas`, `Tools`, `Panels`, `Dialogs`, `DesignSystem`. `Sources/PixmithMain` is the `@main` entry point and `Sources/PixmithQA` the end-to-end scenario runner.
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
4. Put it in its own top-level folder (for example `windows/` or `linux/`) with its own README and build script, and add a CI job for it.

Open an issue first if you plan one, so we can move the shared headers somewhere neutral if that helps.

## Reporting bugs

Open an issue with your OS version, the steps to reproduce and, if you can, a small `.aseprite` file that shows the problem.
