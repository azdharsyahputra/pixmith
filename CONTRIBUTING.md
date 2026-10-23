# Contributing to Pixmith

Thanks for helping. This page covers how to build, test and send a change.

## Setup

- macOS 13 or newer with Xcode or the Command Line Tools (Swift 5.9+).
- A stable Rust toolchain (`rustup` recommended).
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
- Tests must never touch your real `~/Library/Application Support/Pixmith`. The QA runner and the Rust tests use a temporary `HOME` and `PIXMITH_SUPPORT_DIR`; new tests should do the same.
- `.aseprite` support follows the public file format specification. Do not copy code from Aseprite or decompile its binaries.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/), for example `fix(undo): ...` or `feat(godot): ...`.

## Reporting bugs

Open an issue with your macOS version, the steps to reproduce and, if you can, a small `.aseprite` file that shows the problem.
