//! Security module root

pub mod capture;
pub mod clipboard;
pub mod hooks;

pub use capture::CaptureProtector;
pub use clipboard::ClipboardBroker;
pub use hooks::{HotkeyInterceptor, HOTKEY_PRINTSCREEN_ID, HOTKEY_SWITCH_DESKTOP_ID};
