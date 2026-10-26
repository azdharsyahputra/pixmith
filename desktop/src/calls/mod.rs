//! Safe `Doc` methods, grouped by the region that needs them so parallel work does not collide.

mod canvas;
mod files;
pub(crate) mod gamedev;
pub mod images;
mod imports;
mod inspector;
mod menus;
pub mod projects;
mod retouch;
pub mod script_access;
pub mod script_ui;
mod selection;
mod spritedata;
mod tilemap;
mod timeline;
mod tools;
mod warp;

pub(crate) use files::same_file;
pub use menus::FilterRegion;
pub use tilemap::SliceInfo;
