//! Hidden bundled-document fixture for the production worker authentication and lifetime job.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use safebrowse::desktop::{DesktopManager, SupervisedWorkerProcess};
use safebrowse::keyboard::ScopedLanguageBarGuard;
use serde::{Deserialize, Serialize};
use tao::dpi::LogicalSize;
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::WindowBuilder;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Environment8, ICoreWebView2Settings8, COREWEBVIEW2_PROCESS_KIND,
    COREWEBVIEW2_PROCESS_KIND_BROWSER, COREWEBVIEW2_PROCESS_KIND_RENDERER,
};
use webview2_core::{Interface, BOOL};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HWND, LPARAM, RECT, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::StationsAndDesktops::{
    EnumDesktopWindows, GetThreadDesktop, GetUserObjectInformationW, HDESK, UOI_NAME,
};
use windows::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetClassNameW, GetWindowRect, IsWindowVisible,
};
use wry::{WebContext, WebView, WebViewBuilder, WebViewExtWindows};

const FIXTURE_DIRECTORY_PREFIX: &str = "safebrowse-auth-webview-probe-";
const READY_FILENAME: &str = "ready.json";
const ERROR_FILENAME: &str = "error.txt";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const HOLD_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const PROCESS_EXIT_TIMEOUT_MS: u32 = 10_000;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
const NATIVE_NAME_CAPACITY: usize = 256;
const MAX_FIXTURE_WINDOWS: usize = 512;
const INDICATOR_OVERLAY_CLASS: &str = "UAC_InputIndicatorOverlayWnd";
const INPUT_INDICATOR_CLASS: &str = "UAC Input Indicator";
const INDICATOR_HIDE_TIMEOUT: Duration = Duration::from_secs(3);
const FIXTURE_HTML: &str = "<!doctype html><html><head><meta charset='utf-8'><title>Hidden worker fixture</title></head><body><p id='probe'>isolated fixture</p></body></html>";

#[derive(Serialize, Deserialize)]
struct RuntimeProcess {
    id: u32,
    kind: i32,
}

/// Diagnostic metadata from the unused fixture desktop; never records captions or content.
#[derive(Serialize, Deserialize)]
struct NativeWindow {
    class: String,
    rectangle: Option<[i32; 4]>,
    visible: bool,
    #[serde(skip)]
    handle: usize,
}

#[derive(Serialize, Deserialize)]
struct IndicatorSuppressionReport {
    before: Vec<NativeWindow>,
    after: Vec<NativeWindow>,
    native_hiding_verified: bool,
}

#[derive(Serialize, Deserialize)]
struct ReadyReport {
    reputation_checking_required: bool,
    document_verified: bool,
    processes: Vec<RuntimeProcess>,
    input_indicators: IndicatorSuppressionReport,
}

/// Verifies exact-job membership while native browser processes are alive, then observes exit.
pub(super) fn run_supervisor(desktop: &DesktopManager) -> Result<(), String> {
    let directory = FixtureDirectory::create()?;
    let directory_argument = directory
        .path
        .to_str()
        .ok_or("Fixture path is not Unicode")?;
    let worker = desktop.spawn_authenticated_worker(&[
        "--worker",
        "--probe-webview",
        "--probe-directory",
        directory_argument,
    ])?;
    let report = wait_for_report(&worker, &directory.path)?;
    println!(
        "Fixture-only native indicator observation: {}",
        serde_json::to_string(&report.input_indicators).map_err(|error| error.to_string())?
    );
    for candidate_class in [INDICATOR_OVERLAY_CLASS, INPUT_INDICATOR_CLASS] {
        let found = report
            .input_indicators
            .before
            .iter()
            .any(|window| window.class == candidate_class);
        println!("Fixture-only native class {candidate_class:?}: observed={found}");
    }
    println!(
        "Fixture-only production indicator suppression: {}",
        if report.input_indicators.native_hiding_verified {
            "native-hide-verified"
        } else {
            "native-hide-unverified (targets absent, already hidden, or destroyed)"
        }
    );
    if !report.reputation_checking_required || !report.document_verified {
        return Err("Hidden WebView did not verify its document and reputation settings".into());
    }
    for required_kind in [
        COREWEBVIEW2_PROCESS_KIND_BROWSER,
        COREWEBVIEW2_PROCESS_KIND_RENDERER,
    ] {
        if !report
            .processes
            .iter()
            .any(|process| process.kind == required_kind.0)
        {
            return Err("Hidden WebView did not report both browser and renderer processes".into());
        }
    }
    let mut observed_processes = Vec::with_capacity(report.processes.len());
    for process in &report.processes {
        let observed = ObservedProcess::open(process.id)?;
        if !worker.contains_process(observed.0)? {
            return Err(format!(
                "WebView2 process {} was outside the supervisor's job",
                process.id
            ));
        }
        observed_processes.push(observed);
    }
    drop(worker);
    for process in &observed_processes {
        if unsafe { WaitForSingleObject(process.0, PROCESS_EXIT_TIMEOUT_MS) } != WAIT_OBJECT_0 {
            return Err("A WebView2 process survived closure of the supervisor's job".into());
        }
    }
    directory.remove()?;
    println!("PASS: hidden bundled HTML executed; SmartScreen setting true; {} WebView2 processes were contained and terminated with the job.", observed_processes.len());
    Ok(())
}

/// Runs only after the example has authenticated its inherited supervisor capability.
pub(super) fn run_worker(directory: &Path) -> Result<(), String> {
    let result = run_hidden_document(directory);
    if let Err(error) = &result {
        let _ = fs::write(directory.join(ERROR_FILENAME), error);
    }
    result
}

fn run_hidden_document(directory: &Path) -> Result<(), String> {
    if !directory.is_absolute() || !directory.is_dir() {
        return Err("Hidden WebView requires its parent-created fixture directory".into());
    }
    let profile_directory = directory.join("user-data");
    fs::create_dir(&profile_directory)
        .map_err(|error| format!("Cannot create fixture UDF: {error}"))?;
    let mut event_loop = EventLoopBuilder::<()>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_visible(false)
        .with_inner_size(LogicalSize::new(320.0, 200.0))
        .build(&event_loop)
        .map_err(|error| format!("Cannot create hidden fixture window: {error}"))?;
    let mut context = WebContext::new(Some(profile_directory));
    let (loaded_sender, loaded_receiver) = mpsc::channel();
    let loaded_proxy = event_loop.create_proxy();
    let view = WebViewBuilder::new_with_web_context(&mut context)
        .with_visible(false)
        .with_devtools(false)
        .with_navigation_handler(|url| url == "about:blank" || url.starts_with("data:text/html"))
        .with_permission_handler(|_| wry::PermissionResponse::Deny)
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_on_page_load_handler(move |event, _| {
            if matches!(event, wry::PageLoadEvent::Finished) {
                let _ = loaded_sender.send(());
                let _ = loaded_proxy.send_event(());
            }
        })
        .build_as_child(&window)
        .map_err(|error| format!("Cannot create hidden WebView2 under the worker job: {error}"))?;
    let reputation_checking_required = require_fixture_reputation_checking(&view)?;
    view.load_html(FIXTURE_HTML)
        .map_err(|error| format!("Cannot load fixture HTML: {error}"))?;
    receive(&mut event_loop, loaded_receiver)?;
    let (document_sender, document_receiver) = mpsc::channel();
    let document_proxy = event_loop.create_proxy();
    view.evaluate_script_with_callback(
        "document.getElementById('probe')?.textContent === 'isolated fixture' && document.title === 'Hidden worker fixture'",
        move |value| {
            let _ = document_sender.send(value);
            let _ = document_proxy.send_event(());
        },
    ).map_err(|error| format!("Cannot inspect hidden renderer document: {error}"))?;
    let document_verified =
        serde_json::from_str::<bool>(&receive(&mut event_loop, document_receiver)?).map_err(
            |error| format!("Hidden renderer returned invalid inspection data: {error}"),
        )?;
    let (_language_bar_guard, input_indicators) =
        verify_native_indicator_suppression(&mut event_loop)?;
    let report = ReadyReport {
        reputation_checking_required,
        document_verified,
        processes: runtime_processes(&view)?,
        input_indicators,
    };
    let serialized = serde_json::to_vec(&report).map_err(|error| error.to_string())?;
    let pending_report = directory.join("ready.tmp");
    fs::write(&pending_report, serialized).map_err(|error| error.to_string())?;
    fs::rename(pending_report, directory.join(READY_FILENAME))
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + HOLD_TIMEOUT;
    event_loop.run_return(|_, _, flow| {
        *flow = if Instant::now() >= deadline {
            ControlFlow::Exit
        } else {
            ControlFlow::WaitUntil(deadline)
        };
    });
    Err("Supervisor did not terminate the hidden fixture before its deadline".into())
}

/// Borrows only the current authenticated fixture desktop; never opens or switches a desktop.
fn fixture_desktop() -> Result<(HDESK, String), String> {
    let desktop = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
        .map_err(|error| format!("Cannot inspect the fixture desktop: {error}"))?;
    let mut desktop_name = [0u16; NATIVE_NAME_CAPACITY];
    unsafe {
        GetUserObjectInformationW(
            HANDLE(desktop.0),
            UOI_NAME,
            Some(desktop_name.as_mut_ptr().cast()),
            std::mem::size_of_val(&desktop_name) as u32,
            None,
        )
    }
    .map_err(|error| format!("Cannot verify the fixture desktop: {error}"))?;
    let length = desktop_name
        .iter()
        .position(|unit| *unit == 0)
        .ok_or("Fixture desktop name is unterminated")?;
    let desktop_name = String::from_utf16(&desktop_name[..length])
        .map_err(|_| "Fixture desktop name is invalid Unicode")?;
    let prefix = format!("{}_", safebrowse::config::SAFE_DESKTOP_NAME);
    let identifier = desktop_name
        .strip_prefix(&prefix)
        .ok_or("Refusing to enumerate a non-fixture desktop")?;
    if identifier.len() != 32
        || !uuid::Uuid::parse_str(identifier).is_ok_and(|name| name.get_version_num() == 4)
    {
        return Err("Refusing to enumerate a non-fixture desktop".into());
    }
    Ok((desktop, desktop_name))
}

/// Inventories only the current authenticated fixture desktop, including hosted child windows.
/// Time: O(n log n); space: O(n), for at most MAX_FIXTURE_WINDOWS recorded windows.
fn fixture_native_windows() -> Result<Vec<NativeWindow>, String> {
    let (desktop, _) = fixture_desktop()?;
    let mut windows = Vec::<NativeWindow>::new();
    unsafe {
        EnumDesktopWindows(
            Some(desktop),
            Some(inspect_fixture_root_window),
            LPARAM((&mut windows as *mut Vec<NativeWindow>) as isize),
        )
    }
    .map_err(|error| format!("Cannot enumerate fixture native windows: {error}"))?;
    windows.sort_by(|left, right| {
        left.class
            .cmp(&right.class)
            .then(left.rectangle.cmp(&right.rectangle))
    });
    Ok(windows)
}

/// Observes production suppression without creating, showing, or focusing a native indicator.
/// The returned guard remains alive until the supervisor terminates the fixture's lifetime job.
fn verify_native_indicator_suppression(
    event_loop: &mut EventLoop<()>,
) -> Result<(ScopedLanguageBarGuard, IndicatorSuppressionReport), String> {
    let (_, desktop_name) = fixture_desktop()?;
    let before = fixture_native_windows()?;
    let guard = ScopedLanguageBarGuard::install_for_current_thread(&desktop_name)?;
    let deadline = Instant::now() + INDICATOR_HIDE_TIMEOUT;
    let mut next_observation = Instant::now();
    let mut after = Vec::new();
    let mut observation_error = None;
    event_loop.run_return(|_, _, flow| {
        let now = Instant::now();
        if now >= next_observation {
            match fixture_native_windows() {
                Ok(windows) => after = windows,
                Err(error) => observation_error = Some(error),
            }
            next_observation = now + POLL_INTERVAL;
        }
        *flow = if observation_error.is_some()
            || !after.iter().any(visible_input_indicator)
            || now >= deadline
        {
            ControlFlow::Exit
        } else {
            ControlFlow::WaitUntil(next_observation.min(deadline))
        };
    });
    if let Some(error) = observation_error {
        return Err(error);
    }
    if after.iter().any(visible_input_indicator) {
        return Err("Production guard left a native fixture input indicator visible".into());
    }
    let native_hiding_verified = before
        .iter()
        .filter(|window| visible_input_indicator(window))
        .any(|initial| {
            after.iter().any(|current| {
                current.handle == initial.handle
                    && current.class == initial.class
                    && !current.visible
            })
        });
    Ok((
        guard,
        IndicatorSuppressionReport {
            before,
            after,
            native_hiding_verified,
        },
    ))
}

fn visible_input_indicator(window: &NativeWindow) -> bool {
    window.visible
        && matches!(
            window.class.as_str(),
            INDICATOR_OVERLAY_CLASS | INPUT_INDICATOR_CLASS
        )
}

/// Enumerating descendants is read-only and captures indicators embedded in native hosts.
unsafe extern "system" fn inspect_fixture_root_window(
    window: HWND,
    context: LPARAM,
) -> windows::core::BOOL {
    let _ = inspect_fixture_window(window, context);
    let _ = EnumChildWindows(Some(window), Some(inspect_fixture_window), context);
    true.into()
}

unsafe extern "system" fn inspect_fixture_window(
    window: HWND,
    context: LPARAM,
) -> windows::core::BOOL {
    let windows = &mut *(context.0 as *mut Vec<NativeWindow>);
    if windows.len() >= MAX_FIXTURE_WINDOWS {
        return false.into();
    }
    let mut class_name = [0u16; NATIVE_NAME_CAPACITY];
    let class_length = GetClassNameW(window, &mut class_name);
    if class_length <= 0 {
        return true.into();
    }
    let mut rectangle = RECT::default();
    let rectangle = GetWindowRect(window, &mut rectangle).ok().map(|()| {
        [
            rectangle.left,
            rectangle.top,
            rectangle.right,
            rectangle.bottom,
        ]
    });
    windows.push(NativeWindow {
        class: String::from_utf16_lossy(&class_name[..class_length as usize]),
        rectangle,
        visible: IsWindowVisible(window).as_bool(),
        handle: window.0 as usize,
    });
    true.into()
}

fn require_fixture_reputation_checking(view: &WebView) -> Result<bool, String> {
    let configure = || unsafe {
        let settings = view
            .controller()
            .CoreWebView2()?
            .Settings()?
            .cast::<ICoreWebView2Settings8>()?;
        settings.SetIsWebMessageEnabled(false)?;
        settings.SetAreHostObjectsAllowed(false)?;
        settings.SetIsReputationCheckingRequired(true)?;
        let mut required = BOOL::default();
        settings.IsReputationCheckingRequired(&mut required)?;
        Ok::<bool, webview2_core::Error>(required.as_bool())
    };
    configure().map_err(|error| format!("Cannot set fixture reputation checking: {error}"))
}

fn runtime_processes(view: &WebView) -> Result<Vec<RuntimeProcess>, String> {
    let inspect = || unsafe {
        let collection = view
            .environment()
            .cast::<ICoreWebView2Environment8>()?
            .GetProcessInfos()?;
        let mut count = 0;
        collection.Count(&mut count)?;
        let mut processes = Vec::with_capacity(count as usize);
        for index in 0..count {
            let info = collection.GetValueAtIndex(index)?;
            let mut process_id = 0;
            let mut kind = COREWEBVIEW2_PROCESS_KIND::default();
            info.ProcessId(&mut process_id)?;
            info.Kind(&mut kind)?;
            if process_id > 0 {
                processes.push(RuntimeProcess {
                    id: process_id as u32,
                    kind: kind.0,
                });
            }
        }
        Ok::<_, webview2_core::Error>(processes)
    };
    inspect().map_err(|error| format!("Cannot enumerate fixture WebView2 processes: {error}"))
}

fn receive<T>(event_loop: &mut EventLoop<()>, receiver: Receiver<T>) -> Result<T, String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut result = None;
    event_loop.run_return(|_, _, flow| {
        if result.is_none() {
            result = receiver.try_recv().ok();
        }
        *flow = if result.is_some() || Instant::now() >= deadline {
            ControlFlow::Exit
        } else {
            ControlFlow::WaitUntil(deadline)
        };
    });
    result.ok_or_else(|| "Hidden renderer did not respond before the fixture deadline".into())
}

fn wait_for_report(
    worker: &SupervisedWorkerProcess,
    directory: &Path,
) -> Result<ReadyReport, String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Ok(error) = fs::read_to_string(directory.join(ERROR_FILENAME)) {
            return Err(error);
        }
        match fs::read(directory.join(READY_FILENAME)) {
            Ok(bytes) => return serde_json::from_slice(&bytes).map_err(|error| error.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Cannot read hidden WebView report: {error}")),
        }
        if unsafe { WaitForSingleObject(worker.handle(), 0) } != WAIT_TIMEOUT {
            return Err(format!(
                "Hidden WebView worker exited before reporting (code {})",
                worker.exit_code()?
            ));
        }
        if Instant::now() >= deadline {
            return Err("Hidden WebView worker timed out before reporting".into());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

struct ObservedProcess(HANDLE);

impl ObservedProcess {
    fn open(id: u32) -> Result<Self, String> {
        unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                id,
            )
        }
        .map(Self)
        .map_err(|error| format!("Cannot observe fixture WebView2 process {id}: {error}"))
    }
}

impl Drop for ObservedProcess {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Owns only a freshly created, canonicalized fixture directory under the system temporary root.
struct FixtureDirectory {
    path: PathBuf,
}

impl FixtureDirectory {
    fn create() -> Result<Self, String> {
        let temporary_root = std::env::temp_dir()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let path = temporary_root.join(format!(
            "{FIXTURE_DIRECTORY_PREFIX}{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&path).map_err(|error| error.to_string())?;
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        if path.parent() != Some(temporary_root.as_path()) {
            return Err("Fixture directory resolved outside its temporary root".into());
        }
        Ok(Self { path })
    }

    fn remove(&self) -> Result<(), String> {
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        loop {
            match fs::remove_dir_all(&self.path) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) if Instant::now() >= deadline => {
                    return Err(format!(
                        "Cannot remove fixture directory {}: {error}",
                        self.path.display()
                    ))
                }
                Err(_) => std::thread::sleep(POLL_INTERVAL),
            }
        }
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        if let Err(error) = self.remove() {
            eprintln!("{error}");
        }
    }
}
