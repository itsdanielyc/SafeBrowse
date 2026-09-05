//! Win32 Desktop Isolation & Lifecycle Management
//!
//! Provides separate desktop creation (`CreateDesktopW`) using inherited access controls,
//! desktop switching (`SwitchDesktop`), and process-level assignment via `STARTUPINFO.lpDesktop`.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_FILE_NOT_FOUND, HANDLE};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, GetThreadDesktop, GetUserObjectInformationW, OpenDesktopW,
    SetThreadDesktop, SwitchDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_SWITCHDESKTOP, HDESK, UOI_IO,
};
use windows::Win32::System::Threading::GetCurrentThreadId;

use super::launch_auth::{
    new_session_desktop_name, spawn_authenticated_worker, AuthenticatedWorkerSession,
    SupervisedWorkerProcess,
};
use crate::config::{
    DEFAULT_DESKTOP_NAME, DESKTOP_SWITCH_MAX_RETRIES, DESKTOP_SWITCH_RETRY_DELAY,
    SAFE_DESKTOP_ACCESS_MASK,
};

/// RAII wrapper for a Win32 `HDESK` handle ensuring resource reclamation on drop.
#[derive(Debug)]
pub struct DesktopHandle {
    handle: HDESK,
    owned: bool,
}

impl DesktopHandle {
    /// Creates a new managed desktop handle wrapper.
    pub fn new(handle: HDESK, owned: bool) -> Self {
        Self { handle, owned }
    }

    /// Returns the raw Win32 `HDESK` handle.
    #[inline]
    pub fn raw(&self) -> HDESK {
        self.handle
    }
}

impl Drop for DesktopHandle {
    fn drop(&mut self) {
        if self.owned && !self.handle.is_invalid() {
            // Why: Avoid closing the current thread's active desktop if it is currently assigned.
            let current_thread_desktop = unsafe { GetThreadDesktop(GetCurrentThreadId()) };
            if Ok(self.handle) != current_thread_desktop {
                unsafe {
                    let _ = CloseDesktop(self.handle);
                }
            }
        }
    }
}

unsafe impl Send for DesktopHandle {}
unsafe impl Sync for DesktopHandle {}

/// Holds the fresh session desktop open for its full lifetime, preventing name replacement.
pub struct DesktopManager {
    safe_desktop_name: String,
    safe_desktop: Option<DesktopHandle>,
    default_desktop: Option<DesktopHandle>,
    is_on_safe_desktop: Arc<AtomicBool>,
}

/// The session desktop currently receiving physical keyboard and pointer input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionDesktop {
    SafeBrowse,
    Windows,
}

unsafe impl Send for DesktopManager {}
unsafe impl Sync for DesktopManager {}

impl Default for DesktopManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopManager {
    /// Prepares a fresh UUID desktop identity without creating any Windows object.
    pub fn new() -> Self {
        Self {
            safe_desktop_name: new_session_desktop_name(),
            safe_desktop: None,
            default_desktop: None,
            is_on_safe_desktop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Opens only the desktop authorized by this worker's completed supervisor exchange.
    pub fn from_authenticated_worker(
        authorization: &AuthenticatedWorkerSession,
    ) -> Result<Self, String> {
        let mut manager = Self {
            safe_desktop_name: authorization.desktop_name().to_owned(),
            safe_desktop: None,
            default_desktop: None,
            is_on_safe_desktop: Arc::new(AtomicBool::new(false)),
        };
        manager.acquire_default_desktop()?;
        manager.open_authorized_safe_desktop()?;
        Ok(manager)
    }

    /// Returns this manager's fresh session identity for desktop-scoped native controls.
    pub fn safe_desktop_name(&self) -> &str {
        &self.safe_desktop_name
    }

    /// Returns an atomic flag tracking whether the safe desktop is currently active.
    pub fn safe_desktop_active_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.is_on_safe_desktop)
    }

    /// Queries Windows because a desktop switch in the worker cannot update supervisor memory.
    pub(super) fn input_desktop(&self) -> Result<SessionDesktop, String> {
        let safe = self
            .safe_desktop
            .as_ref()
            .ok_or_else(|| "Safe desktop handle not initialized".to_string())?;
        let default = self
            .default_desktop
            .as_ref()
            .ok_or_else(|| "Default desktop handle not initialized".to_string())?;
        if desktop_receives_input(safe.raw())? {
            return Ok(SessionDesktop::SafeBrowse);
        }
        if desktop_receives_input(default.raw())? {
            return Ok(SessionDesktop::Windows);
        }
        Err("The desktop shortcut is unavailable while another Windows desktop is active".into())
    }

    /// Obtains a handle to the interactive default user desktop (`WinSta0\Default`).
    ///
    /// Attempts full desktop access first, falling back to `DESKTOP_SWITCHDESKTOP`
    /// if full permissions are restricted.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn acquire_default_desktop(&mut self) -> Result<(), String> {
        let default_name_wide: Vec<u16> = OsStr::new(DEFAULT_DESKTOP_NAME)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe {
            OpenDesktopW(
                PCWSTR(default_name_wide.as_ptr()),
                DESKTOP_CONTROL_FLAGS(0),
                false,
                SAFE_DESKTOP_ACCESS_MASK,
            )
        };

        let handle = match handle {
            Ok(h) if !h.is_invalid() => Ok(h),
            _ => unsafe {
                // Why: Fallback to least privilege if running under restricted token
                OpenDesktopW(
                    PCWSTR(default_name_wide.as_ptr()),
                    DESKTOP_CONTROL_FLAGS(0),
                    false,
                    DESKTOP_SWITCHDESKTOP.0,
                )
            },
        };

        match handle {
            Ok(h) if !h.is_invalid() => {
                self.default_desktop = Some(DesktopHandle::new(h, true));
                Ok(())
            }
            _ => Err(format!(
                "Failed to open default desktop: Win32 Error {:?}",
                unsafe { GetLastError() }
            )),
        }
    }

    /// Creates this session's randomly named desktop after rejecting a pre-existing object.
    ///
    /// Win32 does not provide atomic exclusive desktop creation. The unpredictable name is
    /// kept inside this process until creation, and the resulting handle remains open. This
    /// prevents predictable-name reuse but does not promise isolation from same-user malware.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn create_safe_desktop(&mut self) -> Result<(), String> {
        if self.safe_desktop.is_some() {
            return Err("The isolated session desktop was already initialized".into());
        }
        let safe_name_wide: Vec<u16> = OsStr::new(&self.safe_desktop_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        match unsafe {
            OpenDesktopW(
                PCWSTR(safe_name_wide.as_ptr()),
                DESKTOP_CONTROL_FLAGS(0),
                false,
                DESKTOP_SWITCHDESKTOP.0,
            )
        } {
            Ok(existing) => {
                drop(DesktopHandle::new(existing, true));
                return Err("Refusing to reuse a pre-existing isolated desktop".into());
            }
            Err(error)
                if error.code() == windows::core::HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) => {}
            Err(error) => {
                return Err(format!(
                    "Could not verify a fresh isolated desktop: {error}"
                ));
            }
        }
        let handle = unsafe {
            CreateDesktopW(
                PCWSTR(safe_name_wide.as_ptr()),
                PCWSTR::null(),
                None,
                DESKTOP_CONTROL_FLAGS(0),
                SAFE_DESKTOP_ACCESS_MASK,
                None,
            )
        };

        match handle {
            Ok(h) if !h.is_invalid() => {
                self.safe_desktop = Some(DesktopHandle::new(h, true));
                Ok(())
            }
            Err(error) => Err(format!(
                "Failed to create the isolated session desktop: {error}"
            )),
            _ => Err("Invalid desktop handle returned".to_string()),
        }
    }

    /// Resolves the already-pinned desktop only after worker launch authentication.
    fn open_authorized_safe_desktop(&mut self) -> Result<(), String> {
        let safe_name_wide: Vec<u16> = OsStr::new(&self.safe_desktop_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe {
            OpenDesktopW(
                PCWSTR(safe_name_wide.as_ptr()),
                DESKTOP_CONTROL_FLAGS(0),
                false,
                SAFE_DESKTOP_ACCESS_MASK,
            )
        };

        match handle {
            Ok(h) if !h.is_invalid() => {
                self.safe_desktop = Some(DesktopHandle::new(h, true));
                Ok(())
            }
            Err(e) => Err(format!(
                "Failed to open the authorized session desktop: {:?}",
                e
            )),
            _ => Err("Invalid desktop handle returned".to_string()),
        }
    }

    /// Switches the physical display and input focus to `SafeBrowseDesktop`.
    ///
    /// Retries multiple times to absorb any transient Windows input queue or
    /// foreground activation latency from Explorer.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn switch_to_safe_desktop(&self) -> Result<(), String> {
        let safe = self
            .safe_desktop
            .as_ref()
            .ok_or_else(|| "Safe desktop handle not initialized".to_string())?;

        // Unlock foreground permissions so SwitchDesktop succeeds immediately without ERROR_ACCESS_DENIED
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP};
            use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
            keybd_event(0x12 /* VK_MENU */, 0, KEYEVENTF_KEYUP, 0);
            let _ = AllowSetForegroundWindow(ASFW_ANY);
        }

        let mut last_error = windows::Win32::Foundation::WIN32_ERROR(0);
        for attempt in 0..DESKTOP_SWITCH_MAX_RETRIES {
            let result = unsafe { SwitchDesktop(safe.raw()) };
            if result.is_ok() {
                self.is_on_safe_desktop.store(true, Ordering::SeqCst);
                return Ok(());
            }
            last_error = unsafe { GetLastError() };
            if attempt < DESKTOP_SWITCH_MAX_RETRIES - 1 {
                std::thread::sleep(DESKTOP_SWITCH_RETRY_DELAY);
            }
        }

        Err(format!(
            "Failed to switch to safe desktop after retries: Win32 Error {:?}",
            last_error
        ))
    }

    /// Restores the physical display and input focus back to the default desktop.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn switch_to_default_desktop(&self) -> Result<(), String> {
        let default = self
            .default_desktop
            .as_ref()
            .ok_or_else(|| "Default desktop handle not initialized".to_string())?;

        // Unlock foreground permissions so SwitchDesktop succeeds immediately without ERROR_ACCESS_DENIED
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP};
            use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
            keybd_event(0x12 /* VK_MENU */, 0, KEYEVENTF_KEYUP, 0);
            let _ = AllowSetForegroundWindow(ASFW_ANY);
        }

        let mut last_error = windows::Win32::Foundation::WIN32_ERROR(0);
        for attempt in 0..DESKTOP_SWITCH_MAX_RETRIES {
            let result = unsafe { SwitchDesktop(default.raw()) };
            if result.is_ok() {
                self.is_on_safe_desktop.store(false, Ordering::SeqCst);
                return Ok(());
            }
            last_error = unsafe { GetLastError() };
            if attempt < DESKTOP_SWITCH_MAX_RETRIES - 1 {
                std::thread::sleep(DESKTOP_SWITCH_RETRY_DELAY);
            }
        }

        Err(format!(
            "Failed to switch to default desktop after retries: Win32 Error {:?}",
            last_error
        ))
    }

    /// Attaches the calling thread to `SafeBrowseDesktop` so windows created by it
    /// automatically reside on the isolated desktop.
    pub fn assign_current_thread_to_safe_desktop(&self) -> Result<(), String> {
        let safe = self
            .safe_desktop
            .as_ref()
            .ok_or_else(|| "Safe desktop handle not initialized".to_string())?;

        let result = unsafe { SetThreadDesktop(safe.raw()) };
        if result.is_ok() {
            Ok(())
        } else {
            Err(format!(
                "Failed to assign thread to safe desktop: Win32 Error {:?}",
                unsafe { GetLastError() }
            ))
        }
    }

    /// Contains and authenticates one worker on this session's pinned desktop.
    pub fn spawn_authenticated_worker(
        &self,
        worker_arguments: &[&str],
    ) -> Result<SupervisedWorkerProcess, String> {
        if self.safe_desktop.is_none() {
            return Err("Create the isolated session desktop before launching its worker".into());
        }
        spawn_authenticated_worker(&self.safe_desktop_name, worker_arguments)
    }
}

/// Reads the input flag without changing the calling thread's desktop or allocating a name buffer.
fn desktop_receives_input(desktop: HDESK) -> Result<bool, String> {
    let mut receives_input = BOOL::default();
    unsafe {
        GetUserObjectInformationW(
            HANDLE(desktop.0),
            UOI_IO,
            Some((&mut receives_input as *mut BOOL).cast()),
            std::mem::size_of::<BOOL>() as u32,
            None,
        )
    }
    .map_err(|error| format!("Could not determine the active desktop: {error}"))?;
    Ok(receives_input.as_bool())
}

#[cfg(test)]
mod input_desktop_tests {
    use super::*;

    #[test]
    fn a_desktop_that_was_never_shown_does_not_receive_input() {
        let desktop_name = format!("SafeBrowse_Input_Test_{}", uuid::Uuid::new_v4());
        let desktop_name_wide = desktop_name
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let desktop = DesktopHandle::new(
            unsafe {
                CreateDesktopW(
                    PCWSTR(desktop_name_wide.as_ptr()),
                    PCWSTR::null(),
                    None,
                    DESKTOP_CONTROL_FLAGS(0),
                    SAFE_DESKTOP_ACCESS_MASK,
                    None,
                )
            }
            .unwrap(),
            true,
        );
        assert!(!desktop_receives_input(desktop.raw()).unwrap());
    }

    #[test]
    fn unavailable_desktop_handles_fail_instead_of_assuming_windows_is_active() {
        assert!(desktop_receives_input(HDESK::default()).is_err());
        assert!(DesktopManager::new().input_desktop().is_err());
    }
}
