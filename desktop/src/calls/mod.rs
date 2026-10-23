//! Safe `Doc` methods, grouped by the region that needs them so parallel work does not collide.

mod canvas;
mod files;
pub(crate) mod gamedev;
mod imports;
mod inspector;
mod menus;
mod selection;
mod tilemap;
mod timeline;
mod tools;

pub use menus::FilterRegion;
pub use tilemap::SliceInfo;
