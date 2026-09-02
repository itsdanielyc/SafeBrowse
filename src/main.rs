//! SafeBrowse: Open-Source High-Assurance Windows Browser
//!
//! Replicating Bitdefender SafePay architecture:
//! - Win32 Alternate Desktop (`WinSta0\SafeBrowseDesktop`)
//! - Display capture exclusion (`WDA_EXCLUDEFROMCAPTURE`)
//! - Hook-immune secure virtual keyboard (direct DOM injection)
//! - Sandboxed Chromium engine with ephemeral auto-purge profiles
//! - Renderer-isolated persistent bookmarks store

use std::env;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::StationsAndDesktops::GetThreadDesktop;
use windows::Win32::System::Threading::WaitForSingleObject;

use safebrowse::browser::ProfileMode;
use safebrowse::desktop::{DesktopManager, DesktopRecoveryGuard, DesktopWatchdog};
use safebrowse::security::ClipboardBroker;
use safebrowse::ui::run_kiosk_session;

/// Prints usage help and operational mode flags.
fn print_usage() {
    println!(
        r#"SafeBrowse - High-Assurance Isolated Windows Browser (Bitdefender SafePay Architecture)

USAGE:
    safebrowse.exe [FLAGS] [OPTIONS]

FLAGS:
    --help, -h          Print this help documentation
    --windowed, -w      Run in windowed mode on current desktop (testing & development)
    --persistent, -p    Use durable persistent profile instead of ephemeral zero-retention
    --worker            Internal flag: signals execution inside the isolated desktop

OPTIONS:
    --url <URL>         Target URL to open immediately upon launch (Default: DuckDuckGo)

ARCHITECTURAL SPECIFICATION:
    - Runs on isolated Win32 Desktop: WinSta0\SafeBrowseDesktop
    - Excluded from DWM screen scrapers via WDA_EXCLUDEFROMCAPTURE
    - Intercepts and consumes PrintScreen keystrokes
    - Secure Virtual Keyboard dispatches directly to DOM (zero OS SendInput hooks)
    - Ephemeral profiles automatically purge all cookies, cache, and history on exit
"#
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    // Check for help flag
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        return Ok(());
    }

    let is_worker = args.iter().any(|a| a == "--worker");
    let is_windowed = args.iter().any(|a| a == "--windowed" || a == "-w");
    let profile_mode = if args.iter().any(|a| a == "--persistent" || a == "-p") {
        ProfileMode::Persistent
    } else {
        ProfileMode::Ephemeral
    };

    let mut target_url = None;
    for i in 0..args.len() {
        if args[i] == "--url" && i + 1 < args.len() {
            target_url = Some(args[i + 1].clone());
        }
    }

    // MODE 1: Windowed Mode (Direct execution on current desktop without switching)
    if is_windowed {
        println!("[SafeBrowse] Starting in Windowed Mode on current desktop...");
        let _ = ClipboardBroker::purge_clipboard(None);
        if let Err(e) = run_kiosk_session(false, profile_mode, target_url, None) {
            eprintln!("[SafeBrowse Error] Session failure: {}", e);
        }
        let _ = ClipboardBroker::purge_clipboard(None);
        return Ok(());
    }

    // MODE 2: Worker Process (Running directly inside SafeBrowseDesktop)
    if is_worker {
        let mut dm = DesktopManager::new();
        let _ = dm.acquire_default_desktop();
        let _ = dm.create_or_open_safe_desktop();
        let _ = dm.assign_current_thread_to_safe_desktop();

        println!("[SafeBrowse Worker] Initialized inside isolated desktop. Launching Kiosk...");
        let _ = ClipboardBroker::purge_clipboard(None);
        if let Err(e) = run_kiosk_session(true, profile_mode, target_url, Some(dm)) {
            eprintln!("[SafeBrowse Worker Error] Kiosk runtime error: {}", e);
        }
        let _ = ClipboardBroker::purge_clipboard(None);
        return Ok(());
    }

    // MODE 3: Launcher / Supervisor Process (Default on launch)
    // Why: Spawns the worker onto the isolated desktop, activates the watchdog failover,
    // and seamlessly switches the active interactive desktop.
    println!("===============================================================");
    println!(" SafeBrowse: Initializing High-Assurance Secure Desktop");
    println!("===============================================================");

    // Sanitize clipboard prior to switching to isolated desktop
    let _ = ClipboardBroker::purge_clipboard(None);

    let mut desktop_manager = DesktopManager::new();

    // 1. Acquire default desktop handle for guaranteed restoration
    desktop_manager
        .acquire_default_desktop()
        .map_err(|e| format!("Failed to acquire default desktop: {}", e))?;

    // 2. Create the isolated desktop
    desktop_manager
        .create_or_open_safe_desktop()
        .map_err(|e| format!("Failed to create isolated desktop: {}", e))?;

    // 3. Prepare child process arguments
    let mut worker_args = vec!["--worker"];
    if profile_mode == ProfileMode::Persistent {
        worker_args.push("--persistent");
    }
    let url_string;
    if let Some(ref url) = target_url {
        worker_args.push("--url");
        url_string = url.clone();
        worker_args.push(&url_string);
    }

    // 4. Spawn the worker process targeted to SafeBrowseDesktop
    let proc_info = desktop_manager
        .spawn_worker_on_safe_desktop(&worker_args)
        .map_err(|e| format!("Failed to launch worker on safe desktop: {}", e))?;

    // 5. Spawn supervisor watchdog thread for emergency failover
    // If the worker crashes, the watchdog will immediately switch back to Default desktop.
    let default_desktop_raw = unsafe {
        GetThreadDesktop(windows::Win32::System::Threading::GetCurrentThreadId())?
    };

    let watchdog = DesktopWatchdog::spawn(proc_info.hProcess, default_desktop_raw);
    let mut recovery_guard = DesktopRecoveryGuard::new(default_desktop_raw);

    // 6. Switch to the isolated safe desktop
    println!("[SafeBrowse] Switching display to SafeBrowseDesktop...");
    desktop_manager
        .switch_to_safe_desktop()
        .map_err(|e| format!("Failed to switch to safe desktop: {}", e))?;

    // 7. Wait for worker process termination
    unsafe {
        WaitForSingleObject(proc_info.hProcess, windows::Win32::System::Threading::INFINITE);
        let _ = CloseHandle(proc_info.hThread);
    }

    // 8. Safely switch back to default desktop
    println!("[SafeBrowse] Worker terminated. Restoring Default interactive desktop...");
    let _ = desktop_manager.switch_to_default_desktop();
    recovery_guard.disarm();
    drop(watchdog);

    // Final clipboard sanitization after session teardown
    let _ = ClipboardBroker::purge_clipboard(None);

    println!("[SafeBrowse] Session completed cleanly.");
    Ok(())
}
