//! SafeBrowse Library Core
//!
//! Exposes all core subsystems for integration tests, tooling, and the main executable:
//! - `desktop`: Win32 Alternate Desktop creation and fail-safe recovery
//! - `security`: Screen capture exclusion, clipboard purging, and defensive hotkey interception
//! - `keyboard`: Hook-immune secure virtual on-screen keyboard
//! - `browser`: Chromium WebView2 engine orchestration and container sandboxing
//! - `bookmarks`: Persistent renderer-isolated bookmark store
//! - `ui`: Full-screen kiosk shell and assets
//! - `config`: Operational parameters and security constants

pub mod bookmarks;
pub mod browser;
pub mod cli;
pub mod config;
pub mod desktop;
pub mod keyboard;
pub mod maintenance;
pub mod security;
pub mod ui;
