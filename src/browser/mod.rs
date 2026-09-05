//! Browser module root

pub mod controller;
pub mod downloads;
pub(crate) mod health;
pub mod navigation;
pub mod permissions;
pub(crate) mod printing;
pub mod profile;
pub mod requests;
pub mod runtime;
pub(crate) mod security;
pub mod tabs;

pub use controller::BrowserController;
pub use profile::{ProfileManager, ProfileMode};
