//! Unit and Integration Tests for Desktop Subsystem

use safebrowse::desktop::{DesktopManager, DesktopRecoveryGuard};

#[test]
fn test_desktop_manager_creation() {
    let dm = DesktopManager::new();
    let flag = dm.safe_desktop_active_flag();
    assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn test_desktop_manager_default() {
    let dm = DesktopManager::default();
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
    let res = dm.create_safe_desktop();
    assert!(
        res.is_ok(),
        "Failed to create a fresh safe desktop: {:?}",
        res
    );
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

#[test]
fn sessions_create_distinct_desktops_without_reusing_another_session() {
    let mut first_session = DesktopManager::new();
    let mut second_session = DesktopManager::new();
    assert_ne!(
        first_session.safe_desktop_name(),
        second_session.safe_desktop_name()
    );
    first_session.create_safe_desktop().unwrap();
    second_session.create_safe_desktop().unwrap();
    assert!(first_session.create_safe_desktop().is_err());
}

#[test]
fn a_preexisting_desktop_is_rejected_before_session_creation() {
    use safebrowse::config::SAFE_DESKTOP_ACCESS_MASK;
    use safebrowse::desktop::manager::DesktopHandle;
    use windows::core::PCWSTR;
    use windows::Win32::System::StationsAndDesktops::{CreateDesktopW, DESKTOP_CONTROL_FLAGS};

    let mut session = DesktopManager::new();
    let desktop_name = session
        .safe_desktop_name()
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let _preexisting = DesktopHandle::new(
        unsafe {
            CreateDesktopW(
                PCWSTR(desktop_name.as_ptr()),
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

    let error = session.create_safe_desktop().unwrap_err();
    assert!(
        error.contains("pre-existing"),
        "unexpected rejection: {error}"
    );
}

#[test]
fn test_switch_desktop_constants_and_least_privilege() {
    use safebrowse::config::{
        DESKTOP_SWITCHDESKTOP_ACCESS, DESKTOP_SWITCH_MAX_RETRIES, DESKTOP_SWITCH_RETRY_DELAY,
        SAFE_DESKTOP_ACCESS_MASK,
    };
    use windows::Win32::System::StationsAndDesktops::DESKTOP_SWITCHDESKTOP;

    // Verify constant values
    assert_eq!(DESKTOP_SWITCHDESKTOP_ACCESS, DESKTOP_SWITCHDESKTOP.0);
    assert_eq!(DESKTOP_SWITCHDESKTOP_ACCESS, 0x0100);
    assert_eq!(SAFE_DESKTOP_ACCESS_MASK, 0x01FF);
    assert_eq!(DESKTOP_SWITCH_MAX_RETRIES, 10);
    assert_eq!(
        DESKTOP_SWITCH_RETRY_DELAY,
        std::time::Duration::from_millis(15)
    );
    // Least privilege: DESKTOP_SWITCHDESKTOP rights must be strictly narrower than SAFE_DESKTOP_ACCESS_MASK
    assert_eq!(
        SAFE_DESKTOP_ACCESS_MASK & DESKTOP_SWITCHDESKTOP_ACCESS,
        DESKTOP_SWITCHDESKTOP_ACCESS
    );
    const { assert!(DESKTOP_SWITCHDESKTOP_ACCESS < SAFE_DESKTOP_ACCESS_MASK) };
}

#[test]
fn failed_desktop_switch_is_not_reported_as_success_on_retry() {
    use safebrowse::desktop::trigger_safe_desktop_switch;
    use windows::Win32::Foundation::HWND;

    let dm = DesktopManager::new();
    let dummy_hwnd = HWND(std::ptr::null_mut());

    assert!(trigger_safe_desktop_switch(dummy_hwnd, &dm).is_err());
    assert!(trigger_safe_desktop_switch(dummy_hwnd, &dm).is_err());
}

#[test]
#[ignore = "Interactive Windows desktop switch; run explicitly in a disposable test session"]
fn test_switch_to_default_desktop_lifecycle() {
    let uninit_dm = DesktopManager::new();
    let err_res = uninit_dm.switch_to_default_desktop();
    assert!(
        err_res.is_err(),
        "Uninitialized DesktopManager must fail cleanly when switching to default desktop"
    );

    let mut dm = DesktopManager::new();
    assert!(dm.acquire_default_desktop().is_ok());
    // Calling switch_to_default_desktop when on the current interactive desktop
    let switch_res = dm.switch_to_default_desktop();
    assert!(
        switch_res.is_ok(),
        "Switch to acquired default desktop must succeed: {:?}",
        switch_res
    );
    assert!(
        !dm.safe_desktop_active_flag()
            .load(std::sync::atomic::Ordering::SeqCst),
        "Active flag must be false after switching to default desktop"
    );
}

#[test]
fn test_taskbar_reentry_message_constants() {
    use windows::Win32::UI::WindowsAndMessaging::{
        SC_MAXIMIZE, SC_RESTORE, WA_INACTIVE, WM_ACTIVATE, WM_ACTIVATEAPP, WM_HOTKEY, WM_SYSCOMMAND,
    };

    assert_eq!(WM_ACTIVATE, 0x0006);
    assert_eq!(WM_ACTIVATEAPP, 0x001C);
    assert_eq!(WM_SYSCOMMAND, 0x0112);
    assert_eq!(WM_HOTKEY, 0x0312);
    assert_eq!(SC_RESTORE as usize, 0xF120);
    assert_eq!(SC_MAXIMIZE as usize, 0xF030);
    assert_eq!(WA_INACTIVE as u16, 0);

    // Verify bitwise mask for SC_RESTORE and SC_MAXIMIZE (0xFFF0)
    assert_eq!(0xF122 & 0xFFF0, SC_RESTORE as usize);
    assert_eq!(0xF030 & 0xFFF0, SC_MAXIMIZE as usize);
}

#[test]
fn test_single_instance_session_mutex() {
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    let mutex_name = windows::core::w!("Local\\SafeBrowse_Test_Session_Mutex");
    let h1 = unsafe { CreateMutexW(None, false, mutex_name) };
    assert!(h1.is_ok(), "First mutex creation must succeed");

    let h2 = unsafe { CreateMutexW(None, false, mutex_name) };
    assert!(h2.is_ok(), "Second mutex creation handle should succeed");
    let last_err = unsafe { GetLastError() };
    assert_eq!(
        last_err, ERROR_ALREADY_EXISTS,
        "Second mutex creation must report ERROR_ALREADY_EXISTS"
    );

    unsafe {
        if let Ok(h) = h1 {
            let _ = CloseHandle(h);
        }
        if let Ok(h) = h2 {
            let _ = CloseHandle(h);
        }
    }
}

#[test]
fn watchdog_releases_only_its_own_process_handle() {
    use safebrowse::desktop::DesktopWatchdog;
    use windows::Win32::Foundation::{CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
    use windows::Win32::System::StationsAndDesktops::GetThreadDesktop;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentThreadId, GetExitCodeProcess,
    };

    let mut caller_handle = HANDLE::default();
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            GetCurrentProcess(),
            GetCurrentProcess(),
            &mut caller_handle,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
        .unwrap();
    }
    let current_desktop = unsafe { GetThreadDesktop(GetCurrentThreadId()).unwrap() };
    // The test process remains alive, so the watchdog never switches the active desktop.
    let watchdog = DesktopWatchdog::spawn(caller_handle, current_desktop).unwrap();
    drop(watchdog);
    let mut exit_code = 0;
    let caller_handle_is_valid = unsafe { GetExitCodeProcess(caller_handle, &mut exit_code) };
    unsafe {
        let _ = CloseHandle(caller_handle);
    }
    assert!(
        caller_handle_is_valid.is_ok(),
        "watchdog closed the caller's handle"
    );
}
