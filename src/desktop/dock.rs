//! Default Desktop Companion & Taskbar Dock Window
//!
//! Provides the taskbar presence on the interactive default desktop (WinSta0\Default)
//! when the secure session is running on SafeBrowseDesktop.
//!
//! Matches Bitdefender SafePay behavior (screenshot 1):
//! - Displays an active taskbar dock icon on the Default desktop
//! - Clicking the dock window or pressing Ctrl+Alt+D switches display back to SafeBrowseDesktop
//! - Automatically terminates and cleans up when the worker browser process exits

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::WindowExtWindows;
use tao::window::WindowBuilder;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetExitCodeProcess;
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, SetWindowLongPtrW, GWLP_WNDPROC, WNDPROC,
};
use wry::{WebContext, WebViewBuilder};

use crate::desktop::DesktopManager;
use crate::security::HotkeyInterceptor;
use crate::ui::assets::generate_dock_companion_html;

/// Win32 process exit code indicating the process is still running.
const STILL_ACTIVE: u32 = 259;

/// Atomic storage holding previous window procedure pointer.
static DOCK_PREV_WNDPROC: AtomicIsize = AtomicIsize::new(0);

/// Global proxy for dispatching events from Win32 messages.
static DOCK_PROXY: Mutex<Option<EventLoopProxy<String>>> = Mutex::new(None);

/// Subclassed Win32 window procedure that intercepts `WM_HOTKEY` (Ctrl+Alt+D) and `WM_SYSCOMMAND` (SC_RESTORE).
unsafe extern "system" fn dock_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    const WM_HOTKEY: u32 = 0x0312;
    const WM_SYSCOMMAND: u32 = 0x0112;
    const SC_RESTORE: usize = 0xF120;

    if msg == WM_HOTKEY {
        if wparam.0 == crate::security::HOTKEY_SWITCH_DESKTOP_ID as usize {
            if let Ok(guard) = DOCK_PROXY.lock() {
                if let Some(ref p) = *guard {
                    let _ = p.send_event("{\"type\": \"SWITCH_TO_SAFE_DESKTOP\"}".to_string());
                }
            }
            return LRESULT(0);
        }
    } else if msg == WM_SYSCOMMAND && (wparam.0 & 0xFFF0) == SC_RESTORE {
        if let Ok(guard) = DOCK_PROXY.lock() {
            if let Some(ref p) = *guard {
                let _ = p.send_event("{\"type\": \"SWITCH_TO_SAFE_DESKTOP\"}".to_string());
            }
        }
    }

    let prev = DOCK_PREV_WNDPROC.load(Ordering::SeqCst);
    if prev != 0 {
        let prev_fn: WNDPROC = std::mem::transmute(prev);
        CallWindowProcW(prev_fn, hwnd, msg, wparam, lparam)
    } else {
        windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wparam, lparam)
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
) -> Result<(), String> {
    let event_loop: EventLoop<String> = EventLoopBuilder::<String>::with_user_event().build();

    let window = WindowBuilder::new()
        .with_title("Bitdefender SAFEPAY™")
        .with_inner_size(LogicalSize::new(460.0, 260.0))
        .with_resizable(false)
        .build(&event_loop)
        .map_err(|e| format!("Failed to create Dock companion window: {}", e))?;

    let hwnd = HWND(window.hwnd() as *mut _);

    // Register Ctrl+Alt+D hotkey on Default desktop to instantly return to SafeBrowseDesktop
    let mut hotkey_interceptor = HotkeyInterceptor::new(hwnd);
    let _ = hotkey_interceptor.register_desktop_toggle_hotkey();

    let proxy = event_loop.create_proxy();
    if let Ok(mut guard) = DOCK_PROXY.lock() {
        *guard = Some(proxy.clone());
    }

    // Install subclassing wndproc
    unsafe {
        let prev = SetWindowLongPtrW(
            hwnd,
            GWLP_WNDPROC,
            dock_wndproc as *const () as isize,
        );
        DOCK_PREV_WNDPROC.store(prev, Ordering::SeqCst);
    }

    let is_running = Arc::new(AtomicBool::new(true));
    let is_running_clone = Arc::clone(&is_running);

    // Background watcher checking if the worker process terminated
    let proxy_worker = event_loop.create_proxy();
    let worker_raw = worker_process.0 as isize;
    std::thread::spawn(move || {
        let handle = HANDLE(worker_raw as *mut _);
        while is_running_clone.load(Ordering::Relaxed) {
            let mut exit_code: u32 = 0;
            let success = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
            if success.is_ok() && exit_code != STILL_ACTIVE {
                let _ = proxy_worker.send_event("{\"type\": \"WORKER_TERMINATED\"}".to_string());
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });

    let mut web_context = WebContext::new(None);
    let html = generate_dock_companion_html();

    let _webview = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_html(html)
        .with_devtools(false)
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(req.body().clone());
        })
        .build(&window)
        .map_err(|e| format!("Failed to initialize Dock webview: {}", e))?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(msg) => {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&msg) {
                    if let Some(msg_type) = val.get("type").and_then(|t| t.as_str()) {
                        match msg_type {
                            "SWITCH_TO_SAFE_DESKTOP" => {
                                let _ = desktop_manager.switch_to_safe_desktop();
                            }
                            "TERMINATE_SESSION" => {
                                unsafe {
                                    let _ = windows::Win32::System::Threading::TerminateProcess(
                                        worker_process,
                                        0,
                                    );
                                }
                                is_running.store(false, Ordering::Relaxed);
                                *control_flow = ControlFlow::Exit;
                            }
                            "WORKER_TERMINATED" => {
                                is_running.store(false, Ordering::Relaxed);
                                *control_flow = ControlFlow::Exit;
                            }
                            _ => {}
                        }
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                let _ = desktop_manager.switch_to_safe_desktop();
            }
            _ => {}
        }
    });
}
