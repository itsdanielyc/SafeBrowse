//! Win32 Desktop Isolation & Lifecycle Management
//!
//! Provides isolated desktop creation (`CreateDesktopW`), secure DACL setup,
//! desktop switching (`SwitchDesktop`), and process-level assignment via `STARTUPINFO.lpDesktop`.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::GetLastError;
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, GetThreadDesktop, OpenDesktopW, SetThreadDesktop, SwitchDesktop,
    DESKTOP_CONTROL_FLAGS, HDESK,
};
use windows::Win32::System::Threading::{
    CreateProcessW, GetCurrentThreadId, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION,
    STARTUPINFOW,
};

use crate::config::{DEFAULT_DESKTOP_NAME, SAFE_DESKTOP_ACCESS_MASK, SAFE_DESKTOP_NAME};

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

/// Coordinates isolation between the standard user desktop and `SafeBrowseDesktop`.
pub struct DesktopManager {
    safe_desktop_name: String,
    safe_desktop: Option<DesktopHandle>,
    default_desktop: Option<DesktopHandle>,
    is_on_safe_desktop: Arc<AtomicBool>,
}

impl DesktopManager {
    /// Initializes a new DesktopManager instance.
    pub fn new() -> Self {
        Self {
            safe_desktop_name: SAFE_DESKTOP_NAME.to_string(),
            safe_desktop: None,
            default_desktop: None,
            is_on_safe_desktop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns an atomic flag tracking whether the safe desktop is currently active.
    pub fn safe_desktop_active_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.is_on_safe_desktop)
    }

    /// Obtains a handle to the interactive default user desktop (`WinSta0\Default`).
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

    /// Creates or opens the isolated Win32 desktop (`SafeBrowseDesktop`) in `WinSta0`.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn create_or_open_safe_desktop(&mut self) -> Result<(), String> {
        let safe_name_wide: Vec<u16> = OsStr::new(&self.safe_desktop_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // Attempt to create the desktop. If it already exists, open it.
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
            _ => {
                // Why: If desktop already exists from a previous ungraceful termination, open it directly.
                let open_handle = unsafe {
                    OpenDesktopW(
                        PCWSTR(safe_name_wide.as_ptr()),
                        DESKTOP_CONTROL_FLAGS(0),
                        false,
                        SAFE_DESKTOP_ACCESS_MASK,
                    )
                };
                match open_handle {
                    Ok(h) if !h.is_invalid() => {
                        self.safe_desktop = Some(DesktopHandle::new(h, true));
                        Ok(())
                    }
                    Err(e) => Err(format!("Failed to create or open safe desktop: {:?}", e)),
                    _ => Err("Invalid desktop handle returned".to_string()),
                }
            }
        }
    }

    /// Switches the physical display and input focus to `SafeBrowseDesktop`.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn switch_to_safe_desktop(&self) -> Result<(), String> {
        let safe = self
            .safe_desktop
            .as_ref()
            .ok_or_else(|| "Safe desktop handle not initialized".to_string())?;

        let result = unsafe { SwitchDesktop(safe.raw()) };
        if result.is_ok() {
            self.is_on_safe_desktop.store(true, Ordering::SeqCst);
            Ok(())
        } else {
            Err(format!(
                "Failed to switch to safe desktop: Win32 Error {:?}",
                unsafe { GetLastError() }
            ))
        }
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

        let result = unsafe { SwitchDesktop(default.raw()) };
        if result.is_ok() {
            self.is_on_safe_desktop.store(false, Ordering::SeqCst);
            Ok(())
        } else {
            Err(format!(
                "Failed to switch to default desktop: Win32 Error {:?}",
                unsafe { GetLastError() }
            ))
        }
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

    /// Spawns the worker browser process assigned to `SafeBrowseDesktop` via `STARTUPINFOW.lpDesktop`.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn spawn_worker_on_safe_desktop(
        &self,
        worker_args: &[&str],
    ) -> Result<PROCESS_INFORMATION, String> {
        let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut command_line = format!("\"{}\"", current_exe.display());
        for arg in worker_args {
            command_line.push(' ');
            command_line.push_str(arg);
        }

        let mut cmd_wide: Vec<u16> = OsStr::new(&command_line)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut desktop_name_wide: Vec<u16> = OsStr::new(&self.safe_desktop_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut startup_info = STARTUPINFOW::default();
        startup_info.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        // Why: Direct the Win32 subsystem to instantiate the child process window station & desktop strictly to SafeBrowseDesktop.
        startup_info.lpDesktop = PWSTR(desktop_name_wide.as_mut_ptr());

        let mut process_info = PROCESS_INFORMATION::default();

        let success = unsafe {
            CreateProcessW(
                PCWSTR::null(),
                Some(PWSTR(cmd_wide.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_UNICODE_ENVIRONMENT,
                None,
                PCWSTR::null(),
                &startup_info,
                &mut process_info,
            )
        };

        if success.is_ok() {
            Ok(process_info)
        } else {
            Err(format!(
                "Failed to spawn worker on safe desktop: Win32 Error {:?}",
                unsafe { GetLastError() }
            ))
        }
    }
}
