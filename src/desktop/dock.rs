//! Default Desktop Companion & Taskbar Dock Window
//!
//! Provides the taskbar presence on the interactive default desktop (WinSta0\Default)
//! when the secure session is running on SafeBrowseDesktop.
//!
//! Provides taskbar presence and re-entry behavior:
//! - Displays an active taskbar dock icon on the Default desktop
//! - Clicking the dock window or pressing Ctrl+Alt+D switches display back to SafeBrowseDesktop
//! - Automatically terminates and cleans up when the worker browser process exits

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::WindowExtWindows;
#[cfg(test)]
use tao::window::WindowBuilder;
use windows::Win32::Foundation::{
    HANDLE, HWND, LPARAM, LRESULT, WAIT_FAILED, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, PostThreadMessageW, SetWindowLongPtrW, GWLP_WNDPROC, WM_QUIT,
    WNDPROC,
};
use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows};

use crate::browser::health::{BrowserHealthEvent, BrowserHealthMonitor};
use crate::browser::{ProfileManager, ProfileMode};
use crate::config::{WATCHDOG_POLL_INTERVAL, WORKER_GRACEFUL_SHUTDOWN_TIMEOUT};
use crate::desktop::manager::SessionDesktop;
use crate::desktop::DesktopManager;
use crate::security::HotkeyInterceptor;
use crate::ui::assets::generate_dock_companion_html;
use crate::ui::trusted::TrustedDocument;

/// Win32 message identifiers for taskbar interaction and hotkeys.
const WM_SYSCOMMAND: u32 = 0x0112;
const WM_HOTKEY: u32 = 0x0312;

/// Win32 system command parameters for window restoration and maximization.
const SC_RESTORE: usize = 0xF120;
const SC_MAXIMIZE: usize = 0xF030;

/// Atomic storage holding previous window procedure pointer.
static DOCK_PREV_WNDPROC: AtomicIsize = AtomicIsize::new(0);

/// Distinguishes a taskbar return request from the session-wide toggle shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockEvent {
    ReturnToSession,
    ToggleDesktop,
    TerminateSession,
    EngineHealth(BrowserHealthEvent),
}

/// Global proxy for dispatching events from Win32 messages.
static DOCK_PROXY: Mutex<Option<EventLoopProxy<DockEvent>>> = Mutex::new(None);

/// Restricts the local companion document to its two explicit native commands.
fn parse_dock_command(message: &str) -> Option<DockEvent> {
    let command = serde_json::from_str::<serde_json::Value>(message).ok()?;
    match command.get("type")?.as_str()? {
        "SWITCH_TO_SAFE_DESKTOP" => Some(DockEvent::ReturnToSession),
        "TERMINATE_SESSION" => Some(DockEvent::TerminateSession),
        _ => None,
    }
}

/// Accepts explicit taskbar restoration and shortcut requests, never passive focus changes.
fn native_dock_command(message: u32, parameter: usize) -> Option<DockEvent> {
    if message == WM_HOTKEY && parameter == crate::security::HOTKEY_SWITCH_DESKTOP_ID as usize {
        return Some(DockEvent::ToggleDesktop);
    }
    if message == WM_SYSCOMMAND && matches!(parameter & 0xFFF0, SC_RESTORE | SC_MAXIMIZE) {
        return Some(DockEvent::ReturnToSession);
    }
    None
}

/// Lets the worker release browser storage and clear its clipboard before the bounded fallback.
fn request_worker_shutdown(worker_thread_id: u32) -> Result<Instant, String> {
    unsafe { PostThreadMessageW(worker_thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }
        .map_err(|error| format!("Could not request browser shutdown: {error}"))?;
    Ok(Instant::now() + WORKER_GRACEFUL_SHUTDOWN_TIMEOUT)
}

/// Routes the one registered shortcut using the current desktop, including worker-initiated switches.
fn trigger_desktop_toggle(hwnd: HWND, desktop_manager: &DesktopManager) -> Result<(), String> {
    match desktop_manager.input_desktop()? {
        SessionDesktop::SafeBrowse => desktop_manager.switch_to_default_desktop(),
        SessionDesktop::Windows => trigger_safe_desktop_switch(hwnd, desktop_manager),
    }
}

/// Triggers foreground activation, switches to SafeBrowseDesktop, and re-minimizes the dock window.
///
/// # Complexity
/// - Time: O(1)
/// - Space: O(1)
pub fn trigger_safe_desktop_switch(
    hwnd: HWND,
    desktop_manager: &DesktopManager,
) -> Result<(), String> {
    // Win32 foreground activation sequence:
    // Unlock foreground permission via simulated Alt key-up (standard Win32 foreground unlock trick)
    // and designate target window as foreground WITHOUT attaching thread input queues.
    // Attaching thread input across processes causes message queue deadlocks with external apps and WebView2.
    if !hwnd.is_invalid() && !hwnd.0.is_null() {
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP};
            use windows::Win32::UI::WindowsAndMessaging::{
                AllowSetForegroundWindow, BringWindowToTop, SetForegroundWindow, ShowWindow,
                ASFW_ANY, SW_RESTORE,
            };

            keybd_event(0x12 /* VK_MENU */, 0, KEYEVENTF_KEYUP, 0);
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            let _ = BringWindowToTop(hwnd);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }

    // Switch physical display to the secure isolated desktop
    let switch_result = desktop_manager.switch_to_safe_desktop();

    // Re-minimize dock companion window on Default desktop ONLY if the switch succeeded,
    // ensuring the window remains active and recoverable if the desktop switch failed.
    if switch_result.is_ok() && !hwnd.is_invalid() && !hwnd.0.is_null() {
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_MINIMIZE};
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        }
    }

    switch_result
}

/// Receives explicit taskbar restoration and keyboard requests without treating focus as consent.
unsafe extern "system" fn dock_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let command = native_dock_command(msg, wparam.0);
    if command == Some(DockEvent::ReturnToSession) {
        // Critical: Forward SC_RESTORE to default/previous wndproc first so Explorer's restore
        // request is fulfilled, the window transitions from iconic state, and Windows OS activates it.
        let prev = DOCK_PREV_WNDPROC.load(Ordering::SeqCst);
        let lresult = if prev != 0 {
            let prev_fn: WNDPROC = std::mem::transmute(prev);
            CallWindowProcW(prev_fn, hwnd, msg, wparam, lparam)
        } else {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        };

        // Post switch event asynchronously so Explorer finishes restoring and establishing foreground rights.
        // Synchronously sleeping or blocking inside SC_RESTORE blocks Explorer from handing over foreground.
        if let Ok(guard) = DOCK_PROXY.lock() {
            if let Some(ref p) = *guard {
                let _ = p.send_event(DockEvent::ReturnToSession);
            }
        }

        return lresult;
    }

    if command == Some(DockEvent::ToggleDesktop) {
        if let Ok(guard) = DOCK_PROXY.lock() {
            if let Some(ref p) = *guard {
                let _ = p.send_event(DockEvent::ToggleDesktop);
            }
        }
        return LRESULT(0);
    }

    let prev = DOCK_PREV_WNDPROC.load(Ordering::SeqCst);
    if prev != 0 {
        let prev_fn: WNDPROC = std::mem::transmute(prev);
        CallWindowProcW(prev_fn, hwnd, msg, wparam, lparam)
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// Runs the companion dock window on the Default desktop to maintain taskbar presence
/// and handle re-entry to the secure desktop.
///
/// # Complexity
/// - Time: O(1) event-driven
/// - Space: O(1)
pub fn run_default_desktop_dock(
    desktop_manager: Arc<DesktopManager>,
    worker_process: HANDLE,
    worker_thread_id: u32,
) -> Result<(), String> {
    let mut event_loop: EventLoop<DockEvent> = EventLoopBuilder::with_user_event().build();

    let window = crate::ui::branding::window_builder()
        .map_err(|error| format!("Cannot load the SafeBrowse companion icon: {error}"))?
        .with_title("SafeBrowse")
        .with_inner_size(LogicalSize::new(320.0, 160.0))
        .with_resizable(false)
        .build(&event_loop)
        .map_err(|e| format!("Failed to create Dock companion window: {}", e))?;

    window.set_minimized(true);

    let hwnd = HWND(window.hwnd() as *mut _);

    // RegisterHotKey is shared across Win32 desktops; the supervisor owns the toggle in both directions.
    let mut hotkey_interceptor = HotkeyInterceptor::new(hwnd);
    if let Err(error) = hotkey_interceptor.register_desktop_toggle_hotkey() {
        eprintln!(
            "[SafeBrowse] Desktop shortcut unavailable: {error}. Use the taskbar entry instead."
        );
    }

    let proxy = event_loop.create_proxy();
    if let Ok(mut guard) = DOCK_PROXY.lock() {
        *guard = Some(proxy.clone());
    }

    // Install subclassing wndproc
    unsafe {
        let prev = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, dock_wndproc as *const () as isize);
        DOCK_PREV_WNDPROC.store(prev, Ordering::SeqCst);
    }

    let dock_profile = ProfileManager::new(ProfileMode::Ephemeral)?;
    let mut web_context = WebContext::new(Some(dock_profile.data_directory().to_path_buf()));
    let health_proxy = proxy.clone();
    let dock_html = generate_dock_companion_html();
    let dock_document = TrustedDocument::new(&dock_html);

    let webview = dock_webview_builder(&mut web_context, dock_document)
        .with_ipc_handler(move |req| {
            if let Some(command) = parse_dock_command(req.body()) {
                let _ = proxy.send_event(command);
            }
        })
        .build(&window)
        .map_err(|e| format!("Failed to initialize Dock webview: {}", e))?;
    crate::browser::runtime::validate_created_environment(&webview, dock_profile.data_directory())?;
    let health = BrowserHealthMonitor::attach(&webview, move |event| {
        let _ = health_proxy.send_event(DockEvent::EngineHealth(event));
    })?;
    webview
        .load_html(&dock_html)
        .map_err(|error| format!("Cannot load the session control: {error}"))?;

    let mut shutdown_deadline = None;
    let mut dock_error = None;
    event_loop.run_return(|event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + WATCHDOG_POLL_INTERVAL);

        match event {
            Event::UserEvent(DockEvent::EngineHealth(BrowserHealthEvent::Failed(failure))) => {
                if shutdown_deadline.is_some() {
                    return;
                }
                dock_error = Some(format!(
                    "{} The session control failed, so SafeBrowse is closing the session. Check the website's transaction status before trying again.",
                    failure.message(),
                ));
                if let Err(error) = desktop_manager.switch_to_default_desktop() {
                    dock_error = Some(format!("{}\n\n{error}", dock_error.as_deref().unwrap_or_default()));
                }
                match request_worker_shutdown(worker_thread_id) {
                    Ok(deadline) => shutdown_deadline = Some(deadline),
                    Err(error) => {
                        dock_error = Some(format!("{}\n\n{error}", dock_error.as_deref().unwrap_or_default()));
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
            Event::UserEvent(DockEvent::ReturnToSession) | Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if shutdown_deadline.is_some() {
                    return;
                }
                if let Err(error) = trigger_safe_desktop_switch(hwnd, &desktop_manager) {
                    eprintln!("[SafeBrowse] {error}");
                }
            }
            Event::UserEvent(DockEvent::ToggleDesktop) => {
                if shutdown_deadline.is_some() {
                    return;
                }
                if let Err(error) = trigger_desktop_toggle(hwnd, &desktop_manager) {
                    eprintln!("[SafeBrowse] {error}");
                }
            }
            Event::UserEvent(DockEvent::TerminateSession) => {
                if shutdown_deadline.is_some() {
                    return;
                }
                match request_worker_shutdown(worker_thread_id) {
                    Ok(deadline) => shutdown_deadline = Some(deadline),
                    Err(error) => {
                        dock_error = Some(error);
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
            Event::MainEventsCleared => {
                let status = unsafe { WaitForSingleObject(worker_process, 0) };
                if status == WAIT_OBJECT_0 {
                    *control_flow = ControlFlow::Exit;
                } else if status == WAIT_FAILED {
                    dock_error = Some("Lost access to the supervised browser process".into());
                    *control_flow = ControlFlow::Exit;
                } else if shutdown_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    unsafe {
                        let _ = TerminateProcess(worker_process, 1);
                    }
                    dock_error = Some(
                        "Browser shutdown timed out; the worker was stopped. Temporary data may remain."
                            .into(),
                    );
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => {}
        }
    });
    hotkey_interceptor.unregister_all();
    // Wry must detach its parent subclass while our original forwarding procedure is intact.
    drop(health);
    drop(webview);
    let previous_procedure = DOCK_PREV_WNDPROC.swap(0, Ordering::SeqCst);
    if previous_procedure != 0 {
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_WNDPROC, previous_procedure);
        }
    }
    if let Ok(mut proxy) = DOCK_PROXY.lock() {
        *proxy = None;
    }
    drop(web_context);
    drop(window);
    let cleanup_result = dock_profile.purge_ephemeral_storage();
    match (dock_error, cleanup_result) {
        (Some(error), Err(cleanup)) => Err(format!("{error}\n\n{cleanup}")),
        (Some(error), Ok(())) => Err(error),
        (None, result) => result,
    }
}

/// Applies the shared exact-document policy before the dock receives its native bridge.
fn dock_webview_builder(context: &mut WebContext, document: TrustedDocument) -> WebViewBuilder<'_> {
    WebViewBuilder::new_with_web_context(context)
        .with_devtools(false)
        .with_browser_accelerator_keys(false)
        .with_default_context_menus(false)
        .with_navigation_handler(move |url| document.allows_navigation(&url))
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_permission_handler(|_| wry::PermissionResponse::Deny)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kiosk::{KioskEvent, Surface};
    use std::time::Duration;
    use tao::platform::windows::EventLoopBuilderExtWindows;

    #[test]
    fn passive_focus_cannot_return_to_the_session() {
        use windows::Win32::UI::WindowsAndMessaging::{WM_ACTIVATE, WM_ACTIVATEAPP, WM_SETFOCUS};
        for message in [WM_ACTIVATE, WM_ACTIVATEAPP, WM_SETFOCUS] {
            for parameter in [0, 1, 2] {
                assert_eq!(native_dock_command(message, parameter), None);
            }
        }
    }

    #[test]
    fn taskbar_return_and_global_toggle_remain_distinct_commands() {
        assert_eq!(
            native_dock_command(WM_SYSCOMMAND, SC_RESTORE),
            Some(DockEvent::ReturnToSession)
        );
        assert_eq!(
            native_dock_command(
                WM_HOTKEY,
                crate::security::HOTKEY_SWITCH_DESKTOP_ID as usize
            ),
            Some(DockEvent::ToggleDesktop)
        );
        assert_eq!(
            native_dock_command(WM_HOTKEY, crate::security::HOTKEY_PRINTSCREEN_ID as usize),
            None
        );
    }

    #[test]
    fn companion_bridge_only_accepts_documented_commands() {
        assert_eq!(
            parse_dock_command(r#"{"type":"SWITCH_TO_SAFE_DESKTOP"}"#),
            Some(DockEvent::ReturnToSession)
        );
        assert_eq!(
            parse_dock_command(r#"{"type":"TERMINATE_SESSION"}"#),
            Some(DockEvent::TerminateSession)
        );
        for message in [
            "broken JSON",
            "null",
            r#"{"type":1}"#,
            r#"{"type":"TOGGLE_DESKTOP"}"#,
        ] {
            assert_eq!(parse_dock_command(message), None);
        }
    }

    #[test]
    fn companion_document_renders_and_posts_its_return_command() {
        let _native_test = crate::ui::NATIVE_WEBVIEW_TEST_LOCK.lock().unwrap();
        const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
        let profile = ProfileManager::new(ProfileMode::Ephemeral).unwrap();
        let mut event_loop = EventLoopBuilder::<KioskEvent>::with_user_event()
            .with_any_thread(true)
            .build();
        let window = WindowBuilder::new()
            .with_visible(false)
            .with_inner_size(LogicalSize::new(320.0, 160.0))
            .build(&event_loop)
            .unwrap();
        let mut context = WebContext::new(Some(profile.data_directory().to_owned()));
        let load_proxy = event_loop.create_proxy();
        let command_proxy = event_loop.create_proxy();
        let html = generate_dock_companion_html();
        let view = dock_webview_builder(&mut context, TrustedDocument::new(&html))
            .with_on_page_load_handler(move |event, _| {
                if matches!(event, wry::PageLoadEvent::Finished) {
                    let _ = load_proxy.send_event(KioskEvent::Ready);
                }
            })
            .with_ipc_handler(move |request| {
                let _ = command_proxy.send_event(KioskEvent::Trusted(
                    Surface::Taskbar,
                    request.body().clone(),
                ));
            })
            .build(&window)
            .unwrap();
        crate::browser::runtime::validate_created_environment(&view, profile.data_directory())
            .unwrap();
        view.load_html(&html).unwrap();
        let deadline = Instant::now() + LOAD_TIMEOUT;
        let mut snapshot = None;
        let mut command = None;
        let snapshot_proxy = event_loop.create_proxy();
        event_loop.run_return(|event, _, control_flow| {
            *control_flow = ControlFlow::WaitUntil(deadline);
            match event {
                Event::UserEvent(KioskEvent::Ready) => {
                    let snapshot_proxy = snapshot_proxy.clone();
                    view.evaluate_script_with_callback(
                        "({title:document.title,button:document.querySelector('button strong')?.textContent,background:getComputedStyle(document.body).backgroundColor,bridge:typeof window.ipc?.postMessage})",
                        move |value| {
                            let _ = snapshot_proxy.send_event(KioskEvent::Trusted(Surface::Internal, value));
                        },
                    ).unwrap();
                }
                Event::UserEvent(KioskEvent::Trusted(Surface::Internal, value)) => {
                    snapshot = Some(serde_json::from_str::<serde_json::Value>(&value).unwrap());
                    view.evaluate_script("document.querySelector('button')?.click()").unwrap();
                }
                Event::UserEvent(KioskEvent::Trusted(Surface::Taskbar, value)) => {
                    command = Some(serde_json::from_str::<serde_json::Value>(&value).unwrap());
                }
                _ => {}
            }
            if command.is_some() || Instant::now() >= deadline {
                *control_flow = ControlFlow::Exit;
            }
        });
        let snapshot = snapshot.expect("dock document never finished loading");
        assert_eq!(snapshot["title"], "Return to SafeBrowse");
        assert_eq!(snapshot["button"], "Return to SafeBrowse");
        assert_eq!(snapshot["background"], "rgb(244, 244, 244)");
        assert_eq!(snapshot["bridge"], "function");
        let command = command.expect("dock return button did not post its native command");
        assert_eq!(command["type"], "SWITCH_TO_SAFE_DESKTOP");
        drop(view);
        drop(context);
        drop(window);
        drop(event_loop);
        profile.purge_ephemeral_storage().unwrap();
    }
}
