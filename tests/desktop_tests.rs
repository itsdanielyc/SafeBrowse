//! Unit and Integration Tests for Desktop Subsystem

use safebrowse::desktop::{DesktopManager, DesktopRecoveryGuard};

#[test]
fn test_desktop_manager_creation() {
    let dm = DesktopManager::new();
    let flag = dm.safe_desktop_active_flag();
    assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn test_acquire_default_desktop() {
    let mut dm = DesktopManager::new();
    let res = dm.acquire_default_desktop();
    assert!(res.is_ok(), "Failed to open default desktop: {:?}", res);
}

#[test]
fn test_create_safe_desktop() {
    let mut dm = DesktopManager::new();
    let res = dm.create_or_open_safe_desktop();
    assert!(res.is_ok(), "Failed to create or open safe desktop: {:?}", res);
}

#[test]
fn test_recovery_guard_disarm() {
    use windows::Win32::System::StationsAndDesktops::GetThreadDesktop;
    let desktop_raw = unsafe {
        GetThreadDesktop(windows::Win32::System::Threading::GetCurrentThreadId()).unwrap()
    };
    let mut guard = DesktopRecoveryGuard::new(desktop_raw);
    guard.disarm();
}
