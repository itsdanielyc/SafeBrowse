//! Security Hotkey & Shortcut Interception
//!
//! Intercepts sensitive keys like PrintScreen (VK_SNAPSHOT) to prevent unauthorized
//! screenshot capture on the secure desktop, and registers desktop toggle shortcuts.

use windows::Win32::Foundation::{GetLastError, HWND};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    VK_SNAPSHOT,
};

/// Unique identifier for the PrintScreen consumption hotkey.
pub const HOTKEY_PRINTSCREEN_ID: i32 = 0xBEEF;

/// Unique identifier for the emergency switch-back hotkey (Ctrl + Alt + D).
pub const HOTKEY_SWITCH_DESKTOP_ID: i32 = 0xCAFE;

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
        if self.hwnd.0.is_null() {
            return Err("Invalid window handle".to_string());
        }

        // Intercept bare VK_SNAPSHOT (PrintScreen)
        let result = unsafe {
            RegisterHotKey(
                Some(self.hwnd),
                HOTKEY_PRINTSCREEN_ID,
                MOD_NOREPEAT,
                VK_SNAPSHOT.0 as u32,
            )
        };

        if result.is_ok() {
            self.printscreen_registered = true;
            Ok(())
        } else {
            Err(format!(
                "Failed to register PrintScreen hotkey: Win32 Error {:?}",
                unsafe { GetLastError() }
            ))
        }
    }

    /// Registers the desktop toggle hotkey (Ctrl + Alt + D).
    pub fn register_desktop_toggle_hotkey(&mut self) -> Result<(), String> {
        if self.hwnd.0.is_null() {
            return Err("Invalid window handle".to_string());
        }

        let modifiers = HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_NOREPEAT.0);
        // 'D' key is virtual key code 0x44
        let result = unsafe {
            RegisterHotKey(
                Some(self.hwnd),
                HOTKEY_SWITCH_DESKTOP_ID,
                modifiers,
                0x44,
            )
        };

        if result.is_ok() {
            self.switch_registered = true;
            Ok(())
        } else {
            Err(format!(
                "Failed to register desktop switch hotkey: Win32 Error {:?}",
                unsafe { GetLastError() }
            ))
        }
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

impl Drop for HotkeyInterceptor {
    fn drop(&mut self) {
        self.unregister_all();
    }
}
