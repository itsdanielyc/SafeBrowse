//! Native regression fixtures use disposable desktops that never receive user input.

use super::*;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, OpenDesktopW, SetThreadDesktop, DESKTOP_CONTROL_FLAGS,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThreadId, CREATE_NO_WINDOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW, RegisterClassW,
    ShowWindow, TranslateMessage, UnregisterClassW, MSG, PM_REMOVE, SW_SHOWNOACTIVATE,
    WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPED,
};

const FIXTURE_DESKTOP_VARIABLE: &str = "SAFEBROWSE_LANGUAGE_BAR_FIXTURE_DESKTOP";
const FIXTURE_MESSAGE_PREFIX: &str = "SAFEBROWSE_LANGUAGE_BAR_FIXTURE:";
const FIXTURE_TEST_NAME: &str =
    "keyboard::language_bar::tests::native_indicator_subprocess_fixture";
const WINDOW_CLASS: &str = "SafeBrowse.LanguageBar.TestWindow";
const CANDIDATE_CLASS: &str = "SafeBrowse.LanguageBar.TestImeCandidate";
const CORE_WINDOW_CLASS: &str = "Windows.UI.Core.CoreWindow";
const OVERLAY_SUFFIX_LOOKALIKE_CLASS: &str = "UAC_InputIndicatorOverlayWndSuffix";
const OVERLAY_PREFIX_LOOKALIKE_CLASS: &str = "PrefixUAC_InputIndicatorOverlayWnd";
const INDICATOR_SUFFIX_LOOKALIKE_CLASS: &str = "UAC Input Indicator Suffix";
const INDICATOR_PREFIX_LOOKALIKE_CLASS: &str = "Prefix UAC Input Indicator";
const WINDOW_WIDTH: i32 = 96;
const WINDOW_HEIGHT: i32 = 40;
const NATIVE_TIMEOUT: Duration = Duration::from_secs(8);
const NONBLOCKING_REFRESH_LIMIT: Duration = Duration::from_secs(2);
const PUMP_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FixtureHandles {
    process_id: u32,
    thread_id: u32,
    floating: usize,
    tray: usize,
    alternate_overlay: usize,
    alternate_indicator: usize,
    late_floating: usize,
    late_tray: usize,
    late_alternate_overlay: usize,
    late_alternate_indicator: usize,
    never_shown: usize,
    plain_eng: usize,
    plain_en: usize,
    longer_title_prefix: usize,
    ime_candidate: usize,
    generic_core_window: usize,
    app_picker: usize,
    overlay_suffix_lookalike: usize,
    overlay_prefix_lookalike: usize,
    indicator_suffix_lookalike: usize,
    indicator_prefix_lookalike: usize,
}

impl FixtureHandles {
    fn initial_indicators(&self) -> [usize; 4] {
        [
            self.floating,
            self.tray,
            self.alternate_overlay,
            self.alternate_indicator,
        ]
    }

    fn indicators(&self) -> [usize; 8] {
        [
            self.floating,
            self.tray,
            self.alternate_overlay,
            self.alternate_indicator,
            self.late_floating,
            self.late_tray,
            self.late_alternate_overlay,
            self.late_alternate_indicator,
        ]
    }

    fn retained_windows(&self) -> [usize; 10] {
        [
            self.plain_eng,
            self.plain_en,
            self.longer_title_prefix,
            self.ime_candidate,
            self.generic_core_window,
            self.app_picker,
            self.overlay_suffix_lookalike,
            self.overlay_prefix_lookalike,
            self.indicator_suffix_lookalike,
            self.indicator_prefix_lookalike,
        ]
    }

    fn assert_controls_unchanged(&self) {
        assert!(self.retained_windows().into_iter().all(is_visible));
        assert!(!is_visible(self.never_shown));
    }

    fn assert_native_owner(&self) {
        let mut process_id = 0;
        let thread_id = unsafe {
            GetWindowThreadProcessId(native_window(self.floating), Some(&mut process_id))
        };
        assert_eq!(process_id, self.process_id);
        assert_eq!(thread_id, self.thread_id);
    }
}

struct DisposableDesktop {
    name: String,
    handle: HDESK,
}

impl DisposableDesktop {
    fn new() -> Self {
        let name = format!("SafeBrowseLanguageBarTest{}", uuid::Uuid::new_v4().simple());
        let wide_name = wide(&name);
        let handle = unsafe {
            CreateDesktopW(
                PCWSTR(wide_name.as_ptr()),
                PCWSTR::null(),
                None,
                DESKTOP_CONTROL_FLAGS(0),
                crate::config::SAFE_DESKTOP_ACCESS_MASK,
                None,
            )
        }
        .expect("create disposable language-bar test desktop");
        Self { name, handle }
    }
}

impl Drop for DisposableDesktop {
    fn drop(&mut self) {
        unsafe { CloseDesktop(self.handle) }
            .expect("close disposable desktop after fixture teardown");
    }
}

/// Attaches a fresh fixture thread without switching the desktop that receives input.
struct ThreadDesktop {
    previous: HDESK,
    fixture: HDESK,
}

impl ThreadDesktop {
    fn attach(name: &str) -> Self {
        let previous = unsafe { GetThreadDesktop(GetCurrentThreadId()) }.unwrap();
        let wide_name = wide(name);
        let fixture = unsafe {
            OpenDesktopW(
                PCWSTR(wide_name.as_ptr()),
                DESKTOP_CONTROL_FLAGS(0),
                false,
                crate::config::SAFE_DESKTOP_ACCESS_MASK,
            )
        }
        .expect("open fixture desktop on a fresh thread");
        if let Err(error) = unsafe { SetThreadDesktop(fixture) } {
            let _ = unsafe { CloseDesktop(fixture) };
            panic!("attach fixture thread to disposable desktop: {error}");
        }
        Self { previous, fixture }
    }
}

impl Drop for ThreadDesktop {
    fn drop(&mut self) {
        unsafe { SetThreadDesktop(self.previous) }
            .expect("detach fixture thread after window teardown");
        unsafe { CloseDesktop(self.fixture) }.expect("close fixture thread desktop handle");
    }
}

struct RegisteredClasses {
    instance: HINSTANCE,
    names: Vec<Vec<u16>>,
}

impl RegisteredClasses {
    fn new() -> Self {
        let instance = HINSTANCE(unsafe { GetModuleHandleW(None) }.unwrap().0);
        let mut classes = Self {
            instance,
            names: Vec::new(),
        };
        for name in [
            WINDOW_CLASS,
            TRAY_INPUT_INDICATOR_CLASS,
            ALTERNATE_DESKTOP_INPUT_OVERLAY_CLASS,
            ALTERNATE_DESKTOP_INPUT_INDICATOR_CLASS,
            CANDIDATE_CLASS,
            CORE_WINDOW_CLASS,
            OVERLAY_SUFFIX_LOOKALIKE_CLASS,
            OVERLAY_PREFIX_LOOKALIKE_CLASS,
            INDICATOR_SUFFIX_LOOKALIKE_CLASS,
            INDICATOR_PREFIX_LOOKALIKE_CLASS,
        ] {
            let name = wide(name);
            let class = WNDCLASSW {
                lpfnWndProc: Some(fixture_window_procedure),
                hInstance: instance,
                lpszClassName: PCWSTR(name.as_ptr()),
                ..Default::default()
            };
            assert_ne!(
                unsafe { RegisterClassW(&class) },
                0,
                "register fixture-only window class"
            );
            classes.names.push(name);
        }
        classes
    }
}

impl Drop for RegisteredClasses {
    fn drop(&mut self) {
        for name in &self.names {
            unsafe { UnregisterClassW(PCWSTR(name.as_ptr()), Some(self.instance)) }
                .expect("unregister class after destroying all fixture windows");
        }
    }
}

struct FixtureWindow(HWND);

/// Creates a fixture window without activating its desktop or taking user focus.
fn create_fixture_window(
    classes: &RegisteredClasses,
    class: &str,
    title: &str,
    visible: bool,
) -> FixtureWindow {
    let class = wide(class);
    let title = wide(title);
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPED,
            0,
            0,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            None,
            None,
            Some(classes.instance),
            None,
        )
    }
    .expect("create a window on the unswitched fixture desktop");
    if visible {
        let _ = unsafe { ShowWindow(window, SW_SHOWNOACTIVATE) };
    }
    FixtureWindow(window)
}

impl Drop for FixtureWindow {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.0) }.expect("destroy fixture window on its owner thread");
    }
}

struct FixtureWindows {
    handles: FixtureHandles,
    created_indicators: Option<[usize; 2]>,
    // Windows must be destroyed before their registered classes.
    _windows: Vec<FixtureWindow>,
    _classes: RegisteredClasses,
}

impl FixtureWindows {
    fn new() -> Self {
        let classes = RegisteredClasses::new();
        let mut windows = Vec::new();
        let mut create = |class: &str, title: &str, visible: bool| {
            let window = create_fixture_window(&classes, class, title, visible);
            let address = window.0 .0 as usize;
            windows.push(window);
            address
        };
        let handles = FixtureHandles {
            process_id: unsafe { GetCurrentProcessId() },
            thread_id: unsafe { GetCurrentThreadId() },
            floating: create(WINDOW_CLASS, FLOATING_LANGUAGE_BAR_TITLE, true),
            tray: create(TRAY_INPUT_INDICATOR_CLASS, "ENG", true),
            alternate_overlay: create(ALTERNATE_DESKTOP_INPUT_OVERLAY_CLASS, "ENG", true),
            alternate_indicator: create(ALTERNATE_DESKTOP_INPUT_INDICATOR_CLASS, "", true),
            late_floating: create(WINDOW_CLASS, FLOATING_LANGUAGE_BAR_TITLE, false),
            late_tray: create(TRAY_INPUT_INDICATOR_CLASS, "", false),
            late_alternate_overlay: create(ALTERNATE_DESKTOP_INPUT_OVERLAY_CLASS, "", false),
            late_alternate_indicator: create(ALTERNATE_DESKTOP_INPUT_INDICATOR_CLASS, "ENG", false),
            never_shown: create(WINDOW_CLASS, FLOATING_LANGUAGE_BAR_TITLE, false),
            plain_eng: create(WINDOW_CLASS, "ENG", true),
            plain_en: create(WINDOW_CLASS, "EN", true),
            longer_title_prefix: create(
                WINDOW_CLASS,
                &format!("{FLOATING_LANGUAGE_BAR_TITLE} suffix"),
                true,
            ),
            ime_candidate: create(CANDIDATE_CLASS, "IME candidate list", true),
            generic_core_window: create(CORE_WINDOW_CLASS, "Input candidates", true),
            app_picker: create(WINDOW_CLASS, "SafeBrowse input language", true),
            overlay_suffix_lookalike: create(OVERLAY_SUFFIX_LOOKALIKE_CLASS, "ENG", true),
            overlay_prefix_lookalike: create(OVERLAY_PREFIX_LOOKALIKE_CLASS, "ENG", true),
            indicator_suffix_lookalike: create(INDICATOR_SUFFIX_LOOKALIKE_CLASS, "ENG", true),
            indicator_prefix_lookalike: create(INDICATOR_PREFIX_LOOKALIKE_CLASS, "ENG", true),
        };
        Self {
            handles,
            created_indicators: None,
            _windows: windows,
            _classes: classes,
        }
    }

    fn apply(&self, command: &str) {
        let windows = match command {
            "late" => vec![
                self.handles.late_floating,
                self.handles.late_tray,
                self.handles.late_alternate_overlay,
                self.handles.late_alternate_indicator,
            ],
            "reshow" => self
                .handles
                .indicators()
                .into_iter()
                .chain(self.created_indicators.into_iter().flatten())
                .collect(),
            other => panic!("unknown fixture command: {other}"),
        };
        for window in windows {
            let _ = unsafe { ShowWindow(native_window(window), SW_SHOWNOACTIVATE) };
        }
    }

    /// Exercises windows first created after guard installation, independently of late visibility.
    fn create_late_indicators(&mut self) -> [usize; 2] {
        assert!(
            self.created_indicators.is_none(),
            "late-created fixture may be issued once"
        );
        let windows = [
            create_fixture_window(
                &self._classes,
                ALTERNATE_DESKTOP_INPUT_OVERLAY_CLASS,
                "ENG",
                true,
            ),
            create_fixture_window(
                &self._classes,
                ALTERNATE_DESKTOP_INPUT_INDICATOR_CLASS,
                "",
                true,
            ),
        ];
        let handles = windows.each_ref().map(|window| window.0 .0 as usize);
        self._windows.extend(windows);
        self.created_indicators = Some(handles);
        handles
    }
}

unsafe extern "system" fn fixture_window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(window, message, wparam, lparam)
}

struct ThreadFixture {
    commands: Sender<String>,
    replies: Receiver<String>,
    thread: Option<JoinHandle<()>>,
}

impl ThreadFixture {
    fn start(desktop: String) -> (Self, FixtureHandles) {
        let (commands, command_receiver) = mpsc::channel::<String>();
        let (reply_sender, replies) = mpsc::channel();
        let thread = thread::spawn(move || {
            let _desktop = ThreadDesktop::attach(&desktop);
            let mut windows = FixtureWindows::new();
            reply_sender
                .send(serde_json::to_string(&windows.handles).unwrap())
                .unwrap();
            run_fixture_loop(&mut windows, command_receiver, |message| {
                reply_sender.send(message).unwrap()
            });
        });
        let fixture = Self {
            commands,
            replies,
            thread: Some(thread),
        };
        let handles =
            serde_json::from_str(&receive_with_pump(&fixture.replies, "thread fixture ready"))
                .unwrap();
        (fixture, handles)
    }

    fn command(&self, command: &str) {
        self.commands.send(command.into()).unwrap();
        assert_eq!(
            receive_with_pump(&self.replies, "thread command completion"),
            "done"
        );
    }

    fn create_late_indicators(&self) -> [usize; 2] {
        self.commands.send("create".into()).unwrap();
        serde_json::from_str(&receive_with_pump(
            &self.replies,
            "late thread indicators created",
        ))
        .unwrap()
    }
}

impl Drop for ThreadFixture {
    fn drop(&mut self) {
        let _ = self.commands.send("stop".into());
        if let Some(thread) = self.thread.take() {
            pump_until("fixture owner thread exits", || thread.is_finished());
            thread
                .join()
                .expect("fixture owner thread completed successfully");
        }
    }
}

struct ProcessFixture {
    child: Child,
    stdin: Option<ChildStdin>,
    replies: Receiver<String>,
    reader: Option<JoinHandle<()>>,
}

impl ProcessFixture {
    fn start(desktop: &str) -> (Self, FixtureHandles) {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                FIXTURE_TEST_NAME,
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FIXTURE_DESKTOP_VARIABLE, desktop)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .expect("start fixture subprocess without a console window");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (reply_sender, replies) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some((_, message)) = line.split_once(FIXTURE_MESSAGE_PREFIX) {
                    if reply_sender.send(message.to_owned()).is_err() {
                        break;
                    }
                }
            }
        });
        let fixture = Self {
            child,
            stdin,
            replies,
            reader: Some(reader),
        };
        let handles = serde_json::from_str(&receive_with_pump(
            &fixture.replies,
            "subprocess fixture ready",
        ))
        .unwrap();
        (fixture, handles)
    }

    fn command(&mut self, command: &str) {
        assert_eq!(self.request(command), "done");
    }

    fn initial_indicator_visibility(&mut self) -> [bool; 4] {
        serde_json::from_str(&self.request("snapshot")).unwrap()
    }

    fn create_late_indicators(&mut self) -> [usize; 2] {
        serde_json::from_str(&self.request("create")).unwrap()
    }

    fn request(&mut self, command: &str) -> String {
        writeln!(self.stdin.as_mut().unwrap(), "{command}").unwrap();
        self.stdin.as_mut().unwrap().flush().unwrap();
        receive_with_pump(&self.replies, "subprocess command completion")
    }
}

impl Drop for ProcessFixture {
    fn drop(&mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = writeln!(stdin, "stop");
            let _ = stdin.flush();
        }
        let deadline = Instant::now() + NATIVE_TIMEOUT;
        let outcome = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status.success(),
                _ if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break false;
                }
                _ => thread::sleep(PUMP_INTERVAL),
            }
        };
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if !thread::panicking() {
            assert!(
                outcome,
                "fixture subprocess must exit cleanly before desktop teardown"
            );
        }
    }
}

fn run_fixture_loop(
    windows: &mut FixtureWindows,
    commands: Receiver<String>,
    acknowledge: impl Fn(String),
) {
    loop {
        pump_messages();
        match commands.try_recv() {
            Ok(command) if command == "stop" => return,
            Ok(command) if command == "pause" => {
                acknowledge("done".into());
                // A bounded receive deliberately does not pump Win32 messages.
                // Old synchronous caption reads would block until this expires.
                match commands.recv_timeout(NATIVE_TIMEOUT) {
                    Ok(command) if command == "resume" => acknowledge("done".into()),
                    Ok(command) if command == "stop" => return,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(mpsc::RecvTimeoutError::Timeout) => acknowledge("pause-expired".into()),
                    Ok(command) => panic!("unexpected command during owner pause: {command}"),
                }
            }
            Ok(command) if command == "snapshot" => {
                windows.handles.assert_controls_unchanged();
                acknowledge(
                    serde_json::to_string(&windows.handles.initial_indicators().map(is_visible))
                        .unwrap(),
                );
            }
            Ok(command) if command == "create" => {
                acknowledge(serde_json::to_string(&windows.create_late_indicators()).unwrap());
            }
            Ok(command) => {
                windows.apply(&command);
                acknowledge("done".into());
            }
            Err(TryRecvError::Disconnected) => return,
            Err(TryRecvError::Empty) => thread::sleep(PUMP_INTERVAL),
        }
    }
}

fn pump_messages() {
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        let _ = unsafe { TranslateMessage(&message) };
        unsafe { DispatchMessageW(&message) };
    }
}

fn pump_until(description: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + NATIVE_TIMEOUT;
    loop {
        pump_messages();
        if condition() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        thread::sleep(PUMP_INTERVAL);
    }
}

fn receive_with_pump(receiver: &Receiver<String>, description: &str) -> String {
    let mut received = None;
    pump_until(description, || match receiver.try_recv() {
        Ok(message) => {
            received = Some(message);
            true
        }
        Err(TryRecvError::Empty) => false,
        Err(TryRecvError::Disconnected) => panic!("fixture ended before {description}"),
    });
    received.unwrap()
}

fn native_window(window: usize) -> HWND {
    HWND(window as *mut _)
}

fn is_visible(window: usize) -> bool {
    unsafe { IsWindowVisible(native_window(window)) }.as_bool()
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[test]
fn native_guard_hides_only_matching_indicators_on_its_disposable_desktop() {
    let _lock = crate::ui::NATIVE_WEBVIEW_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let desktop = DisposableDesktop::new();
    let other_desktop = DisposableDesktop::new();
    let desktop_name = desktop.name.clone();
    let other_desktop_name = other_desktop.name.clone();
    thread::spawn(move || {
        let _desktop = ThreadDesktop::attach(&desktop_name);
        let (peer, peer_windows) = ThreadFixture::start(desktop_name.clone());
        let (mut process, process_windows) = ProcessFixture::start(&desktop_name);
        let (mut other_process, _other_windows) = ProcessFixture::start(&other_desktop_name);
        peer_windows.assert_native_owner();
        process_windows.assert_native_owner();
        assert_ne!(peer_windows.thread_id, unsafe { GetCurrentThreadId() });
        assert_eq!(peer_windows.process_id, unsafe { GetCurrentProcessId() });
        assert_ne!(process_windows.process_id, unsafe { GetCurrentProcessId() });
        assert!(
            indicator_identity(native_window(peer_windows.floating), &other_desktop_name).is_none()
        );
        assert!(indicator_identity(native_window(peer_windows.floating), &desktop_name).is_some());
        assert_eq!(other_process.initial_indicator_visibility(), [true; 4]);
        for windows in [&peer_windows, &process_windows] {
            assert!(
                windows.initial_indicators().into_iter().all(is_visible),
                "all initial indicator fixtures are visible: {windows:?}"
            );
            windows.assert_controls_unchanged();
        }

        peer.command("pause");
        let refresh_started = Instant::now();
        let mut guard = ScopedLanguageBarGuard::install_on_desktop(&desktop_name).unwrap();
        guard.refresh().unwrap();
        assert!(
            refresh_started.elapsed() < NONBLOCKING_REFRESH_LIMIT,
            "native caption lookup and hiding must not wait for the paused owner queue"
        );
        assert!(
            is_visible(peer_windows.floating),
            "async hide awaits the owner message pump"
        );
        peer.command("resume");
        let matching_are_hidden = || {
            [&peer_windows, &process_windows]
                .into_iter()
                .all(|windows| {
                    windows
                        .indicators()
                        .into_iter()
                        .all(|window| !is_visible(window))
                })
        };
        pump_until("initial cross-owner indicators hide", matching_are_hidden);
        for command in ["late", "reshow"] {
            peer.command(command);
            process.command(command);
            pump_until(
                "show hook hides late and repeated indicators",
                matching_are_hidden,
            );
            peer_windows.assert_controls_unchanged();
            process_windows.assert_controls_unchanged();
            // Win32 HWND queries are desktop-scoped; observe this negative control
            // from its owning process without ever switching the input desktop.
            assert_eq!(other_process.initial_indicator_visibility(), [true; 4]);
        }

        let created_indicators = [
            peer.create_late_indicators(),
            process.create_late_indicators(),
        ];
        let created_are_hidden = || {
            created_indicators
                .iter()
                .flatten()
                .all(|window| !is_visible(*window))
        };
        pump_until("new cross-owner indicator windows hide", created_are_hidden);
        peer.command("reshow");
        process.command("reshow");
        pump_until("existing indicators remain hidden", matching_are_hidden);
        pump_until(
            "new indicators hide after repeated show",
            created_are_hidden,
        );
        peer_windows.assert_controls_unchanged();
        process_windows.assert_controls_unchanged();
        assert_eq!(other_process.initial_indicator_visibility(), [true; 4]);

        drop(guard);
        pump_until(
            "drop restores originally visible cross-owner indicators",
            || {
                [&peer_windows, &process_windows]
                    .into_iter()
                    .all(|windows| windows.indicators().into_iter().all(is_visible))
                    && created_indicators
                        .iter()
                        .flatten()
                        .all(|window| is_visible(*window))
            },
        );
        peer_windows.assert_controls_unchanged();
        process_windows.assert_controls_unchanged();
        assert_eq!(other_process.initial_indicator_visibility(), [true; 4]);
    })
    .join()
    .expect("native language-bar regression completed successfully");
}

#[test]
fn public_guard_rejects_regular_desktop_before_enumerating_windows() {
    let thread_id = unsafe { GetCurrentThreadId() };
    if named_thread_desktop(thread_id, crate::config::DEFAULT_DESKTOP_NAME).is_ok() {
        assert!(ScopedLanguageBarGuard::install_for_current_thread(SAFE_DESKTOP_NAME).is_err());
    }
}

#[test]
fn invalid_window_has_no_input_indicator_identity() {
    assert!(indicator_identity(HWND::default(), SAFE_DESKTOP_NAME).is_none());
}

/// Invoked only as a child of the native regression; never opens a normal desktop window.
#[test]
#[ignore = "subprocess fixture requires a disposable desktop supplied by the parent regression"]
fn native_indicator_subprocess_fixture() {
    let Ok(desktop) = std::env::var(FIXTURE_DESKTOP_VARIABLE) else {
        return;
    };
    assert!(desktop.starts_with("SafeBrowseLanguageBarTest"));
    let _desktop = ThreadDesktop::attach(&desktop);
    let mut windows = FixtureWindows::new();
    println!(
        "{FIXTURE_MESSAGE_PREFIX}{}",
        serde_json::to_string(&windows.handles).unwrap()
    );
    let (commands, receiver) = mpsc::channel();
    thread::spawn(move || {
        for command in std::io::stdin().lock().lines().map_while(Result::ok) {
            if commands.send(command).is_err() {
                break;
            }
        }
    });
    run_fixture_loop(&mut windows, receiver, |message| {
        println!("{FIXTURE_MESSAGE_PREFIX}{message}")
    });
}
