//! UI module root

pub mod assets;
pub(crate) mod branding;
mod floating_keyboard;
pub mod kiosk;
mod native;
mod permission_ui;
mod shell_windows;
pub(crate) mod trusted;

// Tao registers one process-wide event window class. Native unit tests use KioskEvent
// consistently and serialize their Windows/WebView2 lifecycles.
#[cfg(test)]
pub(crate) static NATIVE_WEBVIEW_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub use kiosk::{
    clamp_window_pos, clamp_window_rect, make_rect, run_kiosk_session, MIN_VISIBLE_SIDE_WIDTH,
    MIN_VISIBLE_TOP_HEIGHT,
};
