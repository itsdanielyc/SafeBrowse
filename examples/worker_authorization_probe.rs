//! Native launch-admission fixture. It creates an unused desktop and never switches to it.
//!
//! Run with `cargo run --example worker_authorization_probe`. Child modes are private to
//! this fixture and exercise the same production authentication and job-containment code.

use std::os::windows::process::CommandExt;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use safebrowse::desktop::{
    authenticate_worker_launch, extract_worker_auth_arguments, DesktopManager,
};
use windows::Win32::Foundation::{
    CloseHandle, DuplicateHandle, DUPLICATE_HANDLE_OPTIONS, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, WaitForSingleObject, CREATE_NO_WINDOW, PROCESS_SYNCHRONIZE,
};

#[path = "worker_authorization_probe/webview.rs"]
mod webview;

const PROBE_TIMEOUT_MS: u32 = 10_000;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Worker authorization probe: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let raw_arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let mut arguments = raw_arguments.clone();
    let authorization = extract_worker_auth_arguments(&mut arguments)?;
    if arguments.iter().any(|argument| argument == "--worker") {
        if arguments
            .iter()
            .any(|argument| argument == "--probe-exit-before-auth")
        {
            return Ok(());
        }
        if arguments
            .iter()
            .any(|argument| argument == "--probe-stall-before-auth")
        {
            std::thread::sleep(Duration::from_secs(30));
            return Ok(());
        }
        let authorization = authorization.ok_or("Direct worker was rejected before startup")?;
        let _session = authenticate_worker_launch(authorization)?;
        if arguments
            .iter()
            .any(|argument| argument == "--probe-webview")
        {
            let directory = arguments
                .windows(2)
                .find(|pair| pair[0] == "--probe-directory")
                .map(|pair| std::path::Path::new(&pair[1]))
                .ok_or("WebView fixture directory was not supplied")?;
            return webview::run_worker(directory);
        }
        if arguments
            .iter()
            .any(|argument| argument == "--probe-duplicate")
        {
            let duplicate = child_command(&raw_arguments)
                .status()
                .map_err(|error| format!("Could not launch duplicate fixture: {error}"))?;
            if duplicate.success() {
                return Err("Replayed worker arguments unexpectedly authenticated".into());
            }
        }
        if arguments.iter().any(|argument| argument == "--probe-idle") {
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        return Ok(());
    }
    if authorization.is_some() {
        return Err("Authorization handles were supplied without worker mode".into());
    }
    run_supervisor_probe()
}

fn run_supervisor_probe() -> Result<(), String> {
    let direct = child_command(&["--worker".into()])
        .status()
        .map_err(|error| format!("Could not launch direct-worker fixture: {error}"))?;
    if direct.success() {
        return Err("Direct worker launch was not rejected".into());
    }

    let mut desktop = DesktopManager::new();
    desktop.create_safe_desktop()?;
    let worker = desktop.spawn_authenticated_worker(&["--worker", "--probe-duplicate"])?;
    if !worker.contains_process(worker.handle())? {
        return Err("Authenticated worker was not in its supervisor's job".into());
    }
    if unsafe { WaitForSingleObject(worker.handle(), PROBE_TIMEOUT_MS) } != WAIT_OBJECT_0 {
        return Err("Authenticated worker did not finish its duplicate-launch check".into());
    }
    if worker.exit_code()? != 0 {
        return Err("Authenticated worker failed its duplicate-launch check".into());
    }
    drop(worker);

    for mode in ["--probe-exit-before-auth", "--probe-stall-before-auth"] {
        if desktop
            .spawn_authenticated_worker(&["--worker", mode])
            .is_ok()
        {
            return Err("An incomplete launch exchange was accepted".into());
        }
    }

    let worker = desktop.spawn_authenticated_worker(&["--worker", "--probe-idle"])?;
    let mut observed_process = HANDLE::default();
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            worker.handle(),
            GetCurrentProcess(),
            &mut observed_process,
            PROCESS_SYNCHRONIZE.0,
            false,
            DUPLICATE_HANDLE_OPTIONS(0),
        )
    }
    .map_err(|error| format!("Could not observe fixture worker lifetime: {error}"))?;
    drop(worker);
    let stopped =
        unsafe { WaitForSingleObject(observed_process, PROBE_TIMEOUT_MS) } == WAIT_OBJECT_0;
    unsafe {
        let _ = CloseHandle(observed_process);
    }
    if !stopped {
        return Err("Worker outlived its supervisor-owned job".into());
    }
    webview::run_supervisor(&desktop)?;
    println!("PASS: inherited authentication, same-image parent, duplicate rejection, early exit, timeout, hidden WebView2 rendering, and worker/runtime job-close termination; no desktop switch.");
    Ok(())
}

fn child_command(arguments: &[String]) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("fixture executable path"));
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0);
    command
}
