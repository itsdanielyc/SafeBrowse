//! Screen Scraper & Screen Recording Defense
//!
//! Applies Win32 `SetWindowDisplayAffinity` to exclude the window from DWM capture,
//! preventing screen scrapers, Snipping Tool, OBS, and remote desktop viewing from observing content.

use windows::Win32::Foundation::{GetLastError, HWND};
use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WINDOW_DISPLAY_AFFINITY};

use crate::config::{WDA_EXCLUDEFROMCAPTURE, WDA_MONITOR};

/// Manages window capture exclusion policies.
pub struct CaptureProtector;

impl CaptureProtector {
    /// Applies capture exclusion to the specified window handle.
    ///
    /// Tries `WDA_EXCLUDEFROMCAPTURE` first (Windows 10 2004+ / Windows 11).
    /// If that fails (e.g. on older builds), falls back to `WDA_MONITOR`.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn apply_protection(hwnd: HWND) -> Result<(), String> {
        if hwnd.0.is_null() {
            return Err("Invalid window handle passed to CaptureProtector".to_string());
        }

        // Attempt modern WDA_EXCLUDEFROMCAPTURE
        let primary_affinity = WINDOW_DISPLAY_AFFINITY(WDA_EXCLUDEFROMCAPTURE);
        let primary_result = unsafe { SetWindowDisplayAffinity(hwnd, primary_affinity) };
        if primary_result.is_ok() {
            return Ok(());
        }

        // Why: Legacy fallback for pre-2004 Windows 10 releases.
        let fallback_affinity = WINDOW_DISPLAY_AFFINITY(WDA_MONITOR);
        let fallback_result = unsafe { SetWindowDisplayAffinity(hwnd, fallback_affinity) };
        if fallback_result.is_ok() {
            Ok(())
        } else {
            Err(format!(
                "Failed to apply window display affinity: Win32 Error {:?}",
                unsafe { GetLastError() }
            ))
        }
    }
}
