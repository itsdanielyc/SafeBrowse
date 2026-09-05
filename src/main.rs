//! Windows launcher, isolated worker, and supervised session lifecycle.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod startup_error;

use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, HANDLE,
};
use windows::Win32::System::StationsAndDesktops::GetThreadDesktop;
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentThreadId, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE,
};

use safebrowse::browser::runtime::ensure_webview2_runtime_available;
use safebrowse::cli::LaunchOptions;
use safebrowse::desktop::{
    authenticate_worker_launch, extract_worker_auth_arguments, run_default_desktop_dock,
    DesktopManager, DesktopRecoveryGuard, DesktopWatchdog, WorkerAuthArguments,
};
use safebrowse::security::{refuse_elevated_browser_host, ClipboardBroker};
use safebrowse::ui::run_kiosk_session;

const SESSION_MUTEX_NAME: windows::core::PCWSTR =
    windows::core::w!("Local\\SafeBrowse_Session_Mutex");
const MAINTENANCE_MUTEX_NAME: windows::core::PCWSTR =
    windows::core::w!("Local\\SafeBrowse_Maintenance_Mutex");
const MAINTENANCE_IN_PROGRESS_MESSAGE: &str =
    "SafeBrowse is being installed or removed. Wait for setup to finish, then open it again.";

/// Owns a kernel handle so each successful open has exactly one corresponding close.
struct KernelHandle(HANDLE);

impl Drop for KernelHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Prints supported launch modes and the explicit development capture override.
fn print_usage() {
    let _ = writeln!(
        std::io::stdout().lock(),
        r#"SafeBrowse — community browser for Windows

USAGE:
    safebrowse.exe [OPTIONS]

OPTIONS:
    --help, -h                 Show this help
    --windowed, -w             Use the current Windows desktop for development
    --persistent, -p           Keep browser cookies and site data between sessions
    --url <HTTP(S) URL>         Open a destination at startup (default: DuckDuckGo)
    --allow-screen-recording   Disable capture protection for this launch only

By default, SafeBrowse uses a separate desktop, requests capture exclusion,
and removes its temporary browser profile after a normal shutdown.
Ctrl+Alt+D returns to Windows; the SafeBrowse taskbar entry returns to the session.

--allow-screen-recording displays a blocking red warning and a lasting indicator.
Use it with --windowed for UI debugging. Do not use real credentials in this mode.
Run SafeBrowse normally, without administrator privileges.
--worker is a private, authenticated entry point; invoking it directly is rejected.
"#
    );
}

fn main() -> ExitCode {
    let error_presentation = startup_error::StartupErrorPresentation::detect();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error_presentation.report(&error);
            ExitCode::FAILURE
        }
    }
}

/// Validates all launch arguments before touching the clipboard, desktop, or browser profile.
fn run() -> Result<(), String> {
    let arguments = std::env::args_os()
        .skip(1)
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| "Arguments must contain valid Unicode".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (options, worker_authorization) = parse_launch_request(arguments)?;
    if options.show_help {
        print_usage();
        return Ok(());
    }
    // Web content must never inherit an administrator or high-integrity process token.
    refuse_elevated_browser_host()?;
    if let Some(authorization) = worker_authorization {
        return run_worker(options, authorization);
    }
    ensure_webview2_runtime_available()?;
    run_launcher(options)
}

/// Requires the private transport and worker mode together before any startup side effects.
fn parse_launch_request(
    mut arguments: Vec<String>,
) -> Result<(LaunchOptions, Option<WorkerAuthArguments>), String> {
    let authorization = extract_worker_auth_arguments(&mut arguments)?;
    let options = LaunchOptions::parse(arguments)?;
    if options.worker != authorization.is_some() {
        return Err(
            "The internal worker entry point requires authorization from a live SafeBrowse launcher. Start SafeBrowse without internal worker arguments."
                .into(),
        );
    }
    Ok((options, authorization))
}

/// Starts the browser only after its live supervisor authenticates this worker and desktop.
fn run_worker(options: LaunchOptions, authorization: WorkerAuthArguments) -> Result<(), String> {
    let authenticated_session = authenticate_worker_launch(authorization)?;
    ensure_webview2_runtime_available()?;
    let desktop_manager = DesktopManager::from_authenticated_worker(&authenticated_session)?;
    ClipboardBroker::purge_clipboard(None)?;
    let session_result = run_kiosk_session(
        true,
        options.profile_mode,
        options.target_url,
        Some(desktop_manager),
        options.allow_screen_recording,
    );
    let clipboard_result = ClipboardBroker::purge_clipboard(None);
    match (session_result, clipboard_result) {
        (Err(session), Err(clipboard)) => Err(format!(
            "{session}\n\nClipboard cleanup also failed: {clipboard}"
        )),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// Serializes all user-facing sessions so persistent profiles and bookmarks have one writer.
fn run_launcher(options: LaunchOptions) -> Result<(), String> {
    let session_handle = unsafe { CreateMutexW(None, false, SESSION_MUTEX_NAME) }
        .map_err(|error| format!("Could not acquire session lock: {error}"))?;
    let session_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let _session_lock = KernelHandle(session_handle);
    if session_exists {
        return Err(
            "SafeBrowse is already running. Use its existing taskbar control, or close that session before starting another one."
                .into(),
        );
    }
    ensure_maintenance_is_inactive(MAINTENANCE_MUTEX_NAME)?;
    let cleanup = safebrowse::browser::profile::reclaim_abandoned_ephemeral_profiles()?;
    if !cleanup.failures.is_empty() || cleanup.limit_reached {
        return Err(format!(
            "Earlier temporary browser data could not be fully removed. SafeBrowse has not opened a new session. Close old SafeBrowse processes and try again; see README.md for storage locations.\n\n{}{}",
            cleanup.failures.join("\n"),
            if cleanup.limit_reached { "\nThe bounded cleanup scan reached its limit; another launch can continue cleanup." } else { "" },
        ));
    }
    if cleanup.reclaimed > 0 {
        let _ = writeln!(
            std::io::stderr().lock(),
            "[SafeBrowse] Reclaimed {} abandoned temporary profile(s).",
            cleanup.reclaimed
        );
    }
    if options.windowed {
        return run_kiosk_session(
            false,
            options.profile_mode,
            options.target_url,
            None,
            options.allow_screen_recording,
        );
    }
    run_supervisor(options)
}

/// Keeps a new session out of the installer's cleanup-to-file-removal interval.
fn ensure_maintenance_is_inactive(name: windows::core::PCWSTR) -> Result<(), String> {
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, name) } {
        Ok(handle) => {
            let _maintenance_handle = KernelHandle(handle);
            Err(MAINTENANCE_IN_PROGRESS_MESSAGE.into())
        }
        Err(error)
            if error.code() == windows::core::HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) =>
        {
            Ok(())
        }
        Err(error) => Err(format!(
            "Cannot check whether SafeBrowse setup is running: {error}. Close setup and try again."
        )),
    }
}

/// Supervises the isolated worker and restores Windows on normal or failed exit.
fn run_supervisor(options: LaunchOptions) -> Result<(), String> {
    let mut desktop_manager = DesktopManager::new();
    desktop_manager.acquire_default_desktop()?;
    desktop_manager.create_safe_desktop()?;
    let arguments = options.worker_arguments();
    let argument_refs = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    let worker = desktop_manager.spawn_authenticated_worker(&argument_refs)?;
    let original_desktop = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
        .map_err(|error| format!("Could not acquire recovery desktop: {error}"))?;
    let watchdog = DesktopWatchdog::spawn(worker.handle(), original_desktop)?;
    let mut recovery_guard = DesktopRecoveryGuard::new(original_desktop);

    desktop_manager.switch_to_safe_desktop()?;
    let desktop_manager = Arc::new(desktop_manager);
    let dock_result = run_default_desktop_dock(
        Arc::clone(&desktop_manager),
        worker.handle(),
        worker.thread_id(),
    );
    desktop_manager.switch_to_default_desktop()?;
    recovery_guard.disarm();
    drop(watchdog);
    dock_result?;
    let exit_code = worker.exit_code()?;
    if exit_code != 0 {
        return Err(format!(
            "Browser session ended with exit code {exit_code}. Windows desktop restored. SafeBrowse did not reload pages or repeat submissions. Check the website's transaction status before trying again. Temporary session data may remain; the next launch retries cleanup."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_maintenance_is_inactive, parse_launch_request, KernelHandle,
        MAINTENANCE_IN_PROGRESS_MESSAGE,
    };
    use windows::core::PCWSTR;
    use windows::Win32::System::Threading::{CreateEventW, CreateMutexW};

    #[test]
    fn maintenance_gate_detects_only_the_injected_mutex_lifetime() {
        let name = format!(
            "Local\\SafeBrowse_Maintenance_Test_{}",
            uuid::Uuid::new_v4()
        );
        let wide_name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let name = PCWSTR(wide_name.as_ptr());
        assert!(ensure_maintenance_is_inactive(name).is_ok());
        let maintenance = KernelHandle(unsafe { CreateMutexW(None, false, name) }.unwrap());
        assert_eq!(
            ensure_maintenance_is_inactive(name).unwrap_err(),
            MAINTENANCE_IN_PROGRESS_MESSAGE
        );
        drop(maintenance);
        assert!(ensure_maintenance_is_inactive(name).is_ok());
    }

    #[test]
    fn unexpected_maintenance_object_errors_fail_closed() {
        let name = format!(
            "Local\\SafeBrowse_Maintenance_Test_{}",
            uuid::Uuid::new_v4()
        );
        let wide_name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let name = PCWSTR(wide_name.as_ptr());
        let _event = KernelHandle(unsafe { CreateEventW(None, true, false, name) }.unwrap());
        assert!(ensure_maintenance_is_inactive(name)
            .unwrap_err()
            .starts_with("Cannot check whether SafeBrowse setup is running:"));
    }

    #[test]
    fn worker_mode_requires_exactly_one_complete_authorization_transport() {
        for arguments in [
            vec!["--worker"],
            vec!["--worker", "--worker"],
            vec!["--worker", "--worker-auth-read", "4"],
            vec![
                "--worker-auth-read",
                "4",
                "--worker-auth-write",
                "8",
                "--worker-auth-parent",
                "12",
            ],
            vec![
                "--worker",
                "--windowed",
                "--worker-auth-read",
                "4",
                "--worker-auth-write",
                "8",
                "--worker-auth-parent",
                "12",
            ],
        ] {
            assert!(
                parse_launch_request(arguments.iter().map(|value| (*value).to_owned()).collect())
                    .is_err(),
                "accepted invalid launch request: {arguments:?}"
            );
        }
    }

    #[test]
    fn public_launch_modes_do_not_require_or_receive_worker_authorization() {
        for arguments in [
            vec![],
            vec!["--help"],
            vec!["--windowed", "--allow-screen-recording"],
        ] {
            let (options, authorization) =
                parse_launch_request(arguments.iter().map(|value| (*value).to_owned()).collect())
                    .unwrap();
            assert!(!options.worker);
            assert!(authorization.is_none());
        }
    }
}
