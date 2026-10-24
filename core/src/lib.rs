//! Pixmith core: the document model and every pixel operation behind the Pixmith pixel-art editor.
//!
//! The crate is linked into the Swift app as a static library through the C ABI in [`ffi`], and the
//! same code backs the `pixmith-cli` binary. A document is an [`Editor`] wrapping a [`Sprite`]
//! (layers, frames, [`Cel`] buffers, palette) with undo history.
//!
//! # Module map
//!
//! - [`document`]: the sprite model and editor, plus layers, slices, tilemaps and color modes.
//! - [`history`]: document state ids ("modified" tracking), the on-disk undo store and its buffer pool.
//! - [`render`]: compositing views and dirty-rectangle tracking for the canvas.
//! - [`paint`]: drawing and editing tools: shapes, ink, curves, text, filters, transforms, selections.
//! - [`palette`]: palette editing, built-in presets and palette file formats.
//! - [`formats`]: `.pxmt` (Pixmith's own format), `.aseprite`, PNG, GIF, APNG, sprite sheets and JSON, plus
//!   crash-safe file writes.
//! - [`project`]: project folders of canvases (`.pxmt` files plus a `.pixmith` folder) and their C ABI.
//! - [`gamedev`]: autotiling, tweening and the Godot 4 exporter.
//! - [`scripting`]: the Lua scripting host.
//! - [`jobs`]: document snapshots for background work.
//! - [`ffi`]: the C ABI plumbing (errors, panic guard) and the document-level entry points.

// Every `extern "C"` entry point shares the contract documented on the `ffi` module (null and out-of-range
// arguments are tolerated, panics are caught), so a `# Safety` section on each of the ~290 would only repeat it.
#![allow(clippy::missing_safety_doc)]
// Pixel code indexes several parallel buffers with one coordinate; rewriting those loops to iterators would
// obscure the arithmetic for no gain.
#![allow(clippy::needless_range_loop)]
// Tuple-shaped Lua callback arguments, fixtures and cache maps; named aliases would add indirection, not clarity.
#![allow(clippy::type_complexity)]
// `chunks_exact(4)` over RGBA bytes is the idiom throughout; `as_chunks` yields `&[u8; 4]` items and a remainder
// slice, which would ripple through every call site.
#![allow(clippy::chunks_exact_to_as_chunks)]
// Boxing the history store's large variants and errors would change its layout and allocation behavior.
#![allow(clippy::large_enum_variant, clippy::result_large_err)]
// Tests spell out `(row * width + column)` even when row is 0 or 1, to keep the geometry readable.
#![cfg_attr(test, allow(clippy::identity_op, clippy::erasing_op))]

pub mod document;
pub mod ffi;
pub mod formats;
pub mod gamedev;
pub mod history;
pub mod jobs;
pub mod paint;
pub mod palette;
pub mod project;
pub mod render;
pub mod scripting;

#[cfg(test)]
mod regressions;

pub use document::{Cel, Editor, Layer, Sprite};
