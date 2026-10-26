//! Pixmith's cross-platform desktop front end, built with egui.
//!
//! It mirrors the macOS app (`app/`) one-to-one: same layout, same design tokens, same core calls. The
//! library form lets the tests in `tests/` drive the real UI headlessly.

pub mod app;
mod calls;
pub mod doc;
pub mod live;
pub mod startup;
pub mod theme;
pub mod ui;

pub use app::PixmithApp;
pub use calls::projects;
pub use calls::script_access;
pub use calls::script_ui;
