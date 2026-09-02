//! UI module root

pub mod assets;
pub mod kiosk;

pub use kiosk::{clamp_window_pos, clamp_window_rect, make_rect, run_kiosk_session};
