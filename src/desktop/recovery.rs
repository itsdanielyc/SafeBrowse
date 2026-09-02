//! Desktop Recovery Guard & Watchdog Subsystem
//!
//! Enforces the fail-safe recovery invariant: under NO circumstances should a crash,
//! panic, or unexpected termination leave the user trapped on an unresponsive alternate desktop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::StationsAndDesktops::{SwitchDesktop, HDESK};
use windows::Win32::System::Threading::WaitForSingleObject;

/// RAII Guard that unconditionally restores the default desktop upon drop.
pub struct DesktopRecoveryGuard {
    default_desktop: HDESK,
    active: bool,
}

impl DesktopRecoveryGuard {
    /// Creates a recovery guard for the given default desktop handle.
    pub fn new(default_desktop: HDESK) -> Self {
        Self {
            default_desktop,
            active: true,
        }
    }

    /// Disarms the guard if clean manual desktop restoration was already completed.
    pub fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for DesktopRecoveryGuard {
    fn drop(&mut self) {
        if self.active && !self.default_desktop.is_invalid() {
            // Why: Guaranteed execution on thread panic or early return.
            unsafe {
                let _ = SwitchDesktop(self.default_desktop);
            }
        }
    }
}

/// Dedicated supervisor thread monitoring worker process health and emergency keys.
pub struct DesktopWatchdog {
    worker_process_handle: HANDLE,
    _default_desktop: HDESK,
    shutdown_signal: Arc<AtomicBool>,
    worker_exited: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,
}

impl DesktopWatchdog {
    /// Spawns a background supervisor monitoring the worker process.
    ///
    /// # Arguments
    /// - `worker_process_handle`: Win32 `HANDLE` to the worker process.
    /// - `default_desktop`: Win32 `HDESK` to switch back to on termination.
    pub fn spawn(worker_process_handle: HANDLE, default_desktop: HDESK) -> Self {
        let shutdown_signal = Arc::new(AtomicBool::new(false));
        let worker_exited = Arc::new(AtomicBool::new(false));

        let shutdown_clone = Arc::clone(&shutdown_signal);
        let worker_exited_clone = Arc::clone(&worker_exited);

        let handle_val = worker_process_handle.0 as usize;
        let desktop_val = default_desktop.0 as usize;

        let join_handle = thread::spawn(move || {
            let proc_handle = HANDLE(handle_val as _);
            let def_desktop = HDESK(desktop_val as _);

            while !shutdown_clone.load(Ordering::Relaxed) {
                // Poll process liveness with a non-blocking timeout
                let wait_result = unsafe { WaitForSingleObject(proc_handle, 200) };
                if wait_result == WAIT_OBJECT_0 {
                    // Worker process has terminated (cleanly or via crash)
                    worker_exited_clone.store(true, Ordering::SeqCst);
                    // Immediately restore the interactive default desktop
                    unsafe {
                        let _ = SwitchDesktop(def_desktop);
                    }
                    break;
                }
            }
        });

        Self {
            worker_process_handle,
            _default_desktop: default_desktop,
            shutdown_signal,
            worker_exited,
            join_handle: Some(join_handle),
        }
    }

    /// Checks if the monitored worker process has terminated.
    #[inline]
    pub fn has_worker_exited(&self) -> bool {
        self.worker_exited.load(Ordering::SeqCst)
    }

    /// Stops the watchdog supervisor.
    pub fn stop(&mut self) {
        self.shutdown_signal.store(true, Ordering::SeqCst);
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for DesktopWatchdog {
    fn drop(&mut self) {
        self.stop();
        if !self.worker_process_handle.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.worker_process_handle);
            }
        }
    }
}
