//! Best-effort exclusion from supported Windows screen capture APIs.
//!
//! Display affinity is checked at startup, but is not a boundary against malware or cameras.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowDisplayAffinity, SetWindowDisplayAffinity, WINDOW_DISPLAY_AFFINITY,
};

use crate::config::WDA_EXCLUDEFROMCAPTURE;

/// Manages window capture exclusion policies.
pub struct CaptureProtector;

impl CaptureProtector {
    /// Applies capture exclusion to the specified window handle.
    ///
    /// Requires `WDA_EXCLUDEFROMCAPTURE` (Windows 10 2004+ / Windows 11) and checks
    /// the applied value. Unsupported systems fail instead of silently reducing protection.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn apply_protection(hwnd: HWND) -> Result<(), String> {
        if hwnd.0.is_null() {
            return Err("Invalid window handle passed to CaptureProtector".to_string());
        }

        unsafe { SetWindowDisplayAffinity(hwnd, WINDOW_DISPLAY_AFFINITY(WDA_EXCLUDEFROMCAPTURE)) }
            .map_err(|error| format!("Failed to enable capture exclusion: {error}"))?;
        let mut affinity = 0;
        unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) }
            .map_err(|error| format!("Could not verify capture exclusion: {error}"))?;
        if affinity != WDA_EXCLUDEFROMCAPTURE {
            return Err(format!(
                "Capture exclusion was not applied (display affinity: {affinity:#x})"
            ));
        }
        Ok(())
    }
}
