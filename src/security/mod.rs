//! Security module root

pub mod capture;
pub mod clipboard;
pub mod hooks;
pub mod integrity;

pub use capture::CaptureProtector;
pub use clipboard::ClipboardBroker;
pub use hooks::{HotkeyInterceptor, HOTKEY_PRINTSCREEN_ID, HOTKEY_SWITCH_DESKTOP_ID};
pub use integrity::{
    current_process_integrity, refuse_elevated_browser_host, IntegrityLevel, ProcessIntegrity,
};
