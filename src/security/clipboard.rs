//! Clearing the current Windows clipboard at isolated-session boundaries.
//!
//! Does not erase clipboard history, cloud synchronization, or copies held by other processes.

use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard};

/// Provides operations for secure clipboard management.
pub struct ClipboardBroker;

const CLIPBOARD_OPEN_ATTEMPTS: u32 = 5;
const CLIPBOARD_RETRY_INTERVAL: Duration = Duration::from_millis(25);

impl ClipboardBroker {
    /// Clears the Windows clipboard for the current window station.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn purge_clipboard(hwnd: Option<HWND>) -> Result<(), String> {
        // Clipboard managers briefly hold the lock; bounded retries avoid spurious launch failures.
        for attempt in 1..=CLIPBOARD_OPEN_ATTEMPTS {
            match unsafe { OpenClipboard(hwnd) } {
                Ok(()) => break,
                Err(error) if attempt == CLIPBOARD_OPEN_ATTEMPTS => {
                    return Err(format!("Could not open the Windows clipboard: {error}"));
                }
                Err(_) => std::thread::sleep(CLIPBOARD_RETRY_INTERVAL),
            }
        }

        let empty_res = unsafe { EmptyClipboard() };
        let close_res = unsafe { CloseClipboard() };

        empty_res.map_err(|error| format!("Could not empty the Windows clipboard: {error}"))?;
        close_res.map_err(|error| format!("Could not release the Windows clipboard: {error}"))
    }
}
