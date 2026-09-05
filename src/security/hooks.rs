//! Security Hotkey & Shortcut Interception
//!
//! Intercepts sensitive keys like PrintScreen (VK_SNAPSHOT) to prevent unauthorized
//! screenshot capture on the secure desktop, and registers desktop toggle shortcuts.

use windows::Win32::Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, HWND};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_D,
    VK_SNAPSHOT,
};

/// Unique identifier for the PrintScreen consumption hotkey.
pub const HOTKEY_PRINTSCREEN_ID: i32 = 1;

/// Unique identifier for the supervisor's session-wide desktop toggle (Ctrl + Alt + D).
pub const HOTKEY_SWITCH_DESKTOP_ID: i32 = 2;

/// Manages system hotkey registration and defensive key interception.
pub struct HotkeyInterceptor {
    hwnd: HWND,
    printscreen_registered: bool,
    switch_registered: bool,
}

impl HotkeyInterceptor {
    /// Creates a new HotkeyInterceptor bound to the application window.
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            printscreen_registered: false,
            switch_registered: false,
        }
    }

    /// Registers the PrintScreen interceptor to consume screenshot keys.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn register_printscreen_blocker(&mut self) -> Result<(), String> {
        if self.printscreen_registered {
            return Ok(());
        }
        if self.hwnd.0.is_null() {
            return Err("Invalid window handle".to_string());
        }

        unsafe {
            RegisterHotKey(
                Some(self.hwnd),
                HOTKEY_PRINTSCREEN_ID,
                MOD_NOREPEAT,
                VK_SNAPSHOT.0 as u32,
            )
        }
        .map_err(|error| hotkey_registration_error("PrintScreen", error))?;
        self.printscreen_registered = true;
        Ok(())
    }

    /// Registers Ctrl+Alt+D once in the supervisor; workers must not claim the same global chord.
    pub fn register_desktop_toggle_hotkey(&mut self) -> Result<(), String> {
        if self.switch_registered {
            return Ok(());
        }
        if self.hwnd.0.is_null() {
            return Err("Invalid window handle".to_string());
        }

        let modifiers = HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_NOREPEAT.0);
        unsafe {
            RegisterHotKey(
                Some(self.hwnd),
                HOTKEY_SWITCH_DESKTOP_ID,
                modifiers,
                VK_D.0 as u32,
            )
        }
        .map_err(|error| hotkey_registration_error("Ctrl+Alt+D", error))?;
        self.switch_registered = true;
        Ok(())
    }

    /// Unregisters all active hotkeys.
    pub fn unregister_all(&mut self) {
        if self.printscreen_registered {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_PRINTSCREEN_ID);
            }
            self.printscreen_registered = false;
        }
        if self.switch_registered {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_SWITCH_DESKTOP_ID);
            }
            self.switch_registered = false;
        }
    }
}

/// Preserves the API's captured error, with an actionable explanation for shortcut conflicts.
fn hotkey_registration_error(shortcut: &str, error: windows::core::Error) -> String {
    if error.code() == windows::core::HRESULT::from_win32(ERROR_HOTKEY_ALREADY_REGISTERED.0) {
        return format!("{shortcut} is already assigned to another application");
    }
    format!("Could not register {shortcut}: {error}")
}

impl Drop for HotkeyInterceptor {
    fn drop(&mut self) {
        self.unregister_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_shortcut_conflicts_have_an_actionable_message() {
        let error = windows::core::Error::from(windows::core::HRESULT::from_win32(
            ERROR_HOTKEY_ALREADY_REGISTERED.0,
        ));
        assert_eq!(
            hotkey_registration_error("Ctrl+Alt+D", error),
            "Ctrl+Alt+D is already assigned to another application"
        );
    }

    #[test]
    fn invalid_window_registration_does_not_claim_hotkey_ownership() {
        let mut interceptor = HotkeyInterceptor::new(HWND::default());
        assert!(interceptor.register_desktop_toggle_hotkey().is_err());
        assert!(interceptor.register_printscreen_blocker().is_err());
        assert!(!interceptor.switch_registered);
        assert!(!interceptor.printscreen_registered);
        interceptor.unregister_all();
    }
}
