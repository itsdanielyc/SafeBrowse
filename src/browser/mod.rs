//! Browser module root

pub mod controller;
pub mod profile;
pub mod tabs;

pub use controller::BrowserController;
pub use profile::{ProfileManager, ProfileMode};
