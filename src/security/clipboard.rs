//! Clipboard Sanitization & Memory Clearing
//!
//! Enforces clipboard hygiene by providing explicit clearing operations on session
//! startup and termination to prevent sensitive banking credentials, credit card numbers,
//! and OTP codes from leaking across logon desktops.

use windows::Win32::Foundation::{GetLastError, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard};

/// Provides operations for secure clipboard management.
pub struct ClipboardBroker;

impl ClipboardBroker {
    /// Clears the Windows clipboard for the current window station.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn purge_clipboard(hwnd: Option<HWND>) -> Result<(), String> {
        let window_handle = hwnd.unwrap_or(HWND(std::ptr::null_mut()));

        // Why: OpenClipboard locks clipboard for exclusive access by the calling thread.
        let open_res = unsafe { OpenClipboard(Some(window_handle)) };
        if open_res.is_err() {
            return Err(format!(
                "Failed to open clipboard for purging: Win32 Error {:?}",
                unsafe { GetLastError() }
            ));
        }

        let empty_res = unsafe { EmptyClipboard() };
        let close_res = unsafe { CloseClipboard() };

        if empty_res.is_err() || close_res.is_err() {
            return Err("Failed to empty or close clipboard".to_string());
        }

        Ok(())
    }
}
