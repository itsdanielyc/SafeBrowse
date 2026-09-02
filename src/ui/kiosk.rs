//! Kiosk Window & Native SafePay Desktop Shell Execution
//!
//! Replicates the Bitdefender SafePay secure desktop environment:
//! - Full-Screen Desktop Shell with wallpaper and bottom taskbar ("Switch to Desktop")
//! - Floating Browser Window with SafePay chrome (title bar, tabs with "+", omnibox)
//! - Clamping wndproc: Window CANNOT be dragged out of frame, preventing "lost tabs"
//! - Content WebView loads websites directly (no iframes), resolving "domains refused to connect"
//! - Bookmarks and Settings tab integration matching SafePay screenshots 3 and 4
//! - Direct DOM injection for hook-immune Secure Virtual Keyboard

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::{EventLoopBuilderExtWindows, WindowExtWindows};
use tao::window::{Fullscreen, WindowBuilder};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetSystemMetrics, IsIconic, SetForegroundWindow, SetWindowLongPtrW, ShowWindow,
    GWLP_HWNDPARENT, GWLP_WNDPROC, SM_CXSCREEN, SM_CYSCREEN, SW_RESTORE, WINDOWPOS,
    WM_MOVING, WM_WINDOWPOSCHANGING, WNDPROC,
};
use wry::dpi::{Position, Size};
use wry::{Rect, WebContext, WebViewBuilder, WebViewBuilderExtWindows};

use crate::bookmarks::{BookmarkCategory, BookmarkStore};
use crate::browser::tabs::{TabItem, TabKind, TabManager};
use crate::browser::{ProfileManager, ProfileMode};
use crate::config::{CHROMIUM_ARGS_SECURITY, DEFAULT_HOMEPAGE_URL};
use crate::desktop::DesktopManager;
use crate::keyboard::VirtualKeyboard;
use crate::security::{CaptureProtector, HotkeyInterceptor};
use crate::ui::assets::{
    generate_bookmarks_page_html, generate_browser_chrome_html, generate_desktop_shell_html,
    generate_settings_page_html,
};

/// Height in pixels reserved for the bottom desktop taskbar.
pub const DESKTOP_TASKBAR_HEIGHT: i32 = 46;

/// Height in pixels reserved for the browser top chrome (title bar + tab strip + omnibox).
pub const BROWSER_CHROME_HEIGHT: f64 = 110.0;

/// Atomic storage holding the original window procedure pointer before clamping subclassing.
static PREV_WNDPROC: AtomicIsize = AtomicIsize::new(0);

/// Global proxy for dispatching kiosk events from Win32 messages.
static KIOSK_PROXY: Mutex<Option<EventLoopProxy<String>>> = Mutex::new(None);

/// Clamps a window's drag rectangle within the specified monitor boundaries.
///
/// Ensures the window cannot be dragged partially or completely out of the frame,
/// strictly fulfilling the requirement that users can never lose their browser window or tabs.
///
/// # Complexity
/// - Time: O(1)
/// - Space: O(1)
pub fn clamp_window_rect(
    r: &mut RECT,
    screen_left: i32,
    screen_top: i32,
    screen_right: i32,
    screen_bottom: i32,
) {
    let mut width = r.right - r.left;
    let mut height = r.bottom - r.top;
    let max_w = (screen_right - screen_left).max(100);
    let max_h = (screen_bottom - screen_top).max(100);

    if width > max_w {
        width = max_w;
    }
    if height > max_h {
        height = max_h;
    }

    if r.left < screen_left {
        r.left = screen_left;
        r.right = screen_left + width;
    } else if r.right > screen_right {
        r.right = screen_right;
        r.left = screen_right - width;
    }

    if r.top < screen_top {
        r.top = screen_top;
        r.bottom = screen_top + height;
    } else if r.bottom > screen_bottom {
        r.bottom = screen_bottom;
        r.top = screen_bottom - height;
    }
}

/// Clamps window position during `WM_WINDOWPOSCHANGING`.
///
/// # Complexity
/// - Time: O(1)
/// - Space: O(1)
pub fn clamp_window_pos(
    pos: &mut WINDOWPOS,
    screen_left: i32,
    screen_top: i32,
    screen_right: i32,
    screen_bottom: i32,
) {
    let max_w = (screen_right - screen_left).max(100);
    let max_h = (screen_bottom - screen_top).max(100);

    if pos.cx > max_w {
        pos.cx = max_w;
    }
    if pos.cy > max_h {
        pos.cy = max_h;
    }

    if pos.x < screen_left {
        pos.x = screen_left;
    } else if pos.x + pos.cx > screen_right {
        pos.x = screen_right - pos.cx;
    }

    if pos.y < screen_top {
        pos.y = screen_top;
    } else if pos.y + pos.cy > screen_bottom {
        pos.y = screen_bottom - pos.cy;
    }
}

/// Subclassed Win32 window procedure that clamps window movements to the monitor frame
/// and intercepts defensive hotkeys.
///
/// Prevents the browser window from being dragged partially or completely out of view,
/// strictly fulfilling the requirement that users can never lose their browser window or tabs.
///
/// # Complexity
/// - Time: O(1)
/// - Space: O(1)
unsafe extern "system" fn clamped_browser_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    const WM_HOTKEY: u32 = 0x0312;
    if msg == WM_HOTKEY {
        let hotkey_id = wparam.0 as i32;
        if hotkey_id == crate::security::HOTKEY_SWITCH_DESKTOP_ID {
            if let Ok(guard) = KIOSK_PROXY.lock() {
                if let Some(ref p) = *guard {
                    let _ = p.send_event("{\"type\": \"SWITCH_DESKTOP\"}".to_string());
                }
            }
            return LRESULT(0);
        } else if hotkey_id == crate::security::HOTKEY_PRINTSCREEN_ID {
            return LRESULT(0);
        }
    }

    let (screen_left, screen_top, screen_right, screen_bottom) = {
        let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            (
                mi.rcMonitor.left,
                mi.rcMonitor.top,
                mi.rcMonitor.right,
                mi.rcMonitor.bottom - DESKTOP_TASKBAR_HEIGHT,
            )
        } else {
            (
                0,
                0,
                GetSystemMetrics(SM_CXSCREEN),
                GetSystemMetrics(SM_CYSCREEN) - DESKTOP_TASKBAR_HEIGHT,
            )
        }
    };

    if msg == WM_MOVING {
        let rect_ptr = lparam.0 as *mut RECT;
        if !rect_ptr.is_null() {
            let r = &mut *rect_ptr;
            clamp_window_rect(r, screen_left, screen_top, screen_right, screen_bottom);
            return LRESULT(1);
        }
    } else if msg == WM_WINDOWPOSCHANGING {
        let pos_ptr = lparam.0 as *mut WINDOWPOS;
        if !pos_ptr.is_null() {
            let pos = &mut *pos_ptr;
            const SWP_NOMOVE: u32 = 0x0002;
            let is_iconic = pos.x <= -10000 || pos.y <= -10000 || IsIconic(hwnd).as_bool();
            if (pos.flags.0 & SWP_NOMOVE) == 0 && !is_iconic {
                clamp_window_pos(pos, screen_left, screen_top, screen_right, screen_bottom);
            }
        }
    }

    let prev = PREV_WNDPROC.load(Ordering::SeqCst);
    if prev != 0 {
        let prev_fn: WNDPROC = std::mem::transmute(prev);
        CallWindowProcW(prev_fn, hwnd, msg, wparam, lparam)
    } else {
        windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// Helper function to construct a wry `Rect` given logical position and size.
#[inline]
pub fn make_rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect {
        position: Position::Logical(tao::dpi::LogicalPosition::new(x, y)),
        size: Size::Logical(tao::dpi::LogicalSize::new(width, height)),
    }
}

/// Launches and manages the secure SafePay kiosk session.
///
/// # Complexity
/// - Time: Event-driven O(1) per dispatch
/// - Space: O(1)
pub fn run_kiosk_session(
    is_fullscreen: bool,
    profile_mode: ProfileMode,
    initial_url: Option<String>,
    desktop_manager: Option<DesktopManager>,
) -> Result<(), String> {
    let mut builder = EventLoopBuilder::<String>::with_user_event();
    builder.with_any_thread(true);
    let event_loop: EventLoop<String> = builder.build();

    let target_url = initial_url.unwrap_or_else(|| DEFAULT_HOMEPAGE_URL.to_string());

    // Screen dimensions
    let screen_w_phys = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h_phys = unsafe { GetSystemMetrics(SM_CYSCREEN) };

    // Initialize Profile Sandbox and Bookmark Store
    let profile_mgr = ProfileManager::new(profile_mode)?;
    let data_dir = profile_mgr.data_directory().to_path_buf();
    let is_ephemeral = profile_mode == ProfileMode::Ephemeral;
    let mut web_context = WebContext::new(Some(data_dir));
    let security_args = CHROMIUM_ARGS_SECURITY.join(" ");

    let bookmark_store = Arc::new(Mutex::new(
        BookmarkStore::initialize().map_err(|e| format!("Bookmarks init failed: {}", e))?,
    ));

    let tab_manager = Arc::new(Mutex::new(TabManager::new(&target_url)));
    let proxy = event_loop.create_proxy();
    if let Ok(mut guard) = KIOSK_PROXY.lock() {
        *guard = Some(proxy.clone());
    }

    // 1. Desktop Shell Window (full screen wallpaper + bottom taskbar)
    let shell_window = if is_fullscreen {
        let win = WindowBuilder::new()
            .with_title("SafeBrowse Desktop Shell")
            .with_fullscreen(Some(Fullscreen::Borderless(None)))
            .build(&event_loop)
            .map_err(|e| format!("Failed to create shell window: {}", e))?;

        let shell_hwnd = HWND(win.hwnd() as *mut _);
        let _ = CaptureProtector::apply_protection(shell_hwnd);

        let shell_proxy = proxy.clone();
        let _shell_view = WebViewBuilder::new_with_web_context(&mut web_context)
            .with_html(generate_desktop_shell_html())
            .with_devtools(false)
            .with_ipc_handler(move |req| {
                let _ = shell_proxy.send_event(req.body().clone());
            })
            .build(&win)
            .map_err(|e| format!("Failed to build shell webview: {}", e))?;

        Some((win, shell_hwnd))
    } else {
        None
    };

    // 2. Floating Browser Window dimensions
    let initial_browser_w = if is_fullscreen {
        (screen_w_phys as f64 * 0.82).clamp(1000.0, 1480.0)
    } else {
        1200.0
    };
    let initial_browser_h = if is_fullscreen {
        ((screen_h_phys - DESKTOP_TASKBAR_HEIGHT) as f64 * 0.85).clamp(650.0, 920.0)
    } else {
        760.0
    };
    let initial_browser_x = if is_fullscreen {
        ((screen_w_phys as f64) - initial_browser_w) / 2.0
    } else {
        40.0
    };
    let initial_browser_y = if is_fullscreen {
        (((screen_h_phys - DESKTOP_TASKBAR_HEIGHT) as f64) - initial_browser_h) / 2.0
    } else {
        30.0
    };

    let browser_window = WindowBuilder::new()
        .with_title("Bitdefender SAFEPAY™")
        .with_decorations(false)
        .with_inner_size(LogicalSize::new(initial_browser_w, initial_browser_h))
        .with_position(LogicalPosition::new(initial_browser_x, initial_browser_y))
        .build(&event_loop)
        .map_err(|e| format!("Failed to create browser window: {}", e))?;

    let browser_hwnd = HWND(browser_window.hwnd() as *mut _);

    // Apply anti-screen capture protection to the browser window
    let _ = CaptureProtector::apply_protection(browser_hwnd);

    // Register PrintScreen and Desktop toggle hotkeys
    let mut hotkey_interceptor = HotkeyInterceptor::new(browser_hwnd);
    let _ = hotkey_interceptor.register_printscreen_blocker();
    let _ = hotkey_interceptor.register_desktop_toggle_hotkey();

    // If on safe desktop, set browser window's Win32 owner to shell window
    // Why: Guarantees browser window stays permanently in front of the desktop shell.
    if let Some((_, shell_hwnd)) = shell_window {
        unsafe {
            let _ = SetWindowLongPtrW(browser_hwnd, GWLP_HWNDPARENT, shell_hwnd.0 as isize);
        }
    }

    // Install clamping subclass onto browser window procedure
    // Why: Enforces that dragging NEVER allows the window to escape outside the visible screen.
    unsafe {
        let prev = SetWindowLongPtrW(
            browser_hwnd,
            GWLP_WNDPROC,
            clamped_browser_wndproc as *const () as isize,
        );
        PREV_WNDPROC.store(prev, Ordering::SeqCst);
    }

    // 3. Browser Chrome WebView (Top Titlebar, Tabs, Omnibox, Virtual Keyboard)
    let chrome_proxy = proxy.clone();
    let tabs_snapshot: Vec<TabItem> = {
        let tm = tab_manager.lock().unwrap();
        tm.list().to_vec()
    };
    let active_tab_id = {
        let tm = tab_manager.lock().unwrap();
        tm.active_id()
    };
    let chrome_html = generate_browser_chrome_html(&tabs_snapshot, active_tab_id);

    let chrome_bounds = make_rect(0.0, 0.0, initial_browser_w, BROWSER_CHROME_HEIGHT);
    let browser_chrome = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_bounds(chrome_bounds)
        .with_html(chrome_html)
        .with_devtools(false)
        .with_additional_browser_args(security_args.clone())
        .with_ipc_handler(move |req| {
            let _ = chrome_proxy.send_event(req.body().clone());
        })
        .build(&browser_window)
        .map_err(|e| format!("Failed to create browser chrome webview: {}", e))?;

    // 4. Browser Content WebView (Top-level browsing context, ZERO iframes)
    let content_proxy = proxy.clone();
    let content_h = initial_browser_h - BROWSER_CHROME_HEIGHT;
    let content_bounds = make_rect(0.0, BROWSER_CHROME_HEIGHT, initial_browser_w, content_h);

    let init_script = r#"
    (function() {
        function notifyState() {
            if (window.ipc && window.ipc.postMessage) {
                window.ipc.postMessage(JSON.stringify({
                    type: 'CONTENT_STATE_CHANGE',
                    url: window.location.href,
                    title: document.title || window.location.hostname || 'SafeBrowse Web'
                }));
            }
        }
        if (document.readyState === 'complete') {
            notifyState();
        } else {
            window.addEventListener('load', notifyState);
        }
        const titleEl = document.querySelector('title');
        if (titleEl) {
            new MutationObserver(notifyState).observe(titleEl, { childList: true, characterData: true, subtree: true });
        }
    })();
    "#;

    let load_proxy = proxy.clone();
    let nw_proxy = proxy.clone();

    let browser_content = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_bounds(content_bounds)
        .with_url(&target_url)
        .with_incognito(is_ephemeral)
        .with_devtools(false)
        .with_additional_browser_args(security_args)
        .with_initialization_script(init_script)
        .with_on_page_load_handler(move |event, url| {
            if let wry::PageLoadEvent::Finished = event {
                let _ = load_proxy.send_event(
                    serde_json::json!({
                        "type": "PAGE_LOAD_FINISHED",
                        "url": url
                    })
                    .to_string(),
                );
            }
        })
        .with_new_window_req_handler(move |url, _disposition| {
            let _ = nw_proxy.send_event(
                serde_json::json!({
                    "type": "NEW_TAB_WITH_URL",
                    "url": url
                })
                .to_string(),
            );
            wry::NewWindowResponse::Deny
        })
        .with_ipc_handler(move |req| {
            let _ = content_proxy.send_event(req.body().clone());
        })
        .build(&browser_window)
        .map_err(|e| format!("Failed to create browser content webview: {}", e))?;

    let browser_chrome = Arc::new(Mutex::new(browser_chrome));
    let browser_content = Arc::new(Mutex::new(browser_content));

    // State for maximize / restore toggle
    let mut is_maximized = false;
    let mut restore_bounds = (
        initial_browser_x,
        initial_browser_y,
        initial_browser_w,
        initial_browser_h,
    );
    let mut current_window_size = (initial_browser_w, initial_browser_h);

    let dm_arc = desktop_manager.map(Arc::new);

    // Event Loop
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(ipc_msg) => {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&ipc_msg) {
                    if let Some(msg_type) = val.get("type").and_then(|t| t.as_str()) {
                        match msg_type {
                            // Shell Actions
                            "SWITCH_DESKTOP" => {
                                if let Some(ref dm) = dm_arc {
                                    let _ = dm.switch_to_default_desktop();
                                }
                            }
                            "FOCUS_BROWSER" => {
                                browser_window.set_minimized(false);
                                unsafe {
                                    let _ = ShowWindow(browser_hwnd, SW_RESTORE);
                                    let _ = SetForegroundWindow(browser_hwnd);
                                }
                                browser_window.set_focus();
                            }
                            "EXIT_APP" => {
                                if let Some(ref dm) = dm_arc {
                                    let _ = dm.switch_to_default_desktop();
                                }
                                *control_flow = ControlFlow::Exit;
                            }

                            // Window Controls
                            "START_DRAG" => {
                                if !is_maximized {
                                    let _ = browser_window.drag_window();
                                }
                            }
                            "MINIMIZE" => {
                                browser_window.set_minimized(true);
                            }
                            "TOGGLE_MAXIMIZE" => {
                                let scale = browser_window.scale_factor();
                                if is_maximized {
                                    browser_window.set_inner_size(LogicalSize::new(
                                        restore_bounds.2,
                                        restore_bounds.3,
                                    ));
                                    browser_window.set_outer_position(LogicalPosition::new(
                                        restore_bounds.0,
                                        restore_bounds.1,
                                    ));
                                    is_maximized = false;
                                } else {
                                    // Save restore bounds in logical units before maximizing
                                    if let Ok(pos) = browser_window.outer_position() {
                                        restore_bounds.0 = pos.x as f64 / scale;
                                        restore_bounds.1 = pos.y as f64 / scale;
                                    }
                                    restore_bounds.2 = current_window_size.0;
                                    restore_bounds.3 = current_window_size.1;

                                    let max_w = screen_w_phys as f64 / scale;
                                    let max_h = (screen_h_phys - DESKTOP_TASKBAR_HEIGHT) as f64 / scale;
                                    browser_window.set_outer_position(LogicalPosition::new(0.0, 0.0));
                                    browser_window.set_inner_size(LogicalSize::new(max_w, max_h));
                                    is_maximized = true;
                                }
                            }
                            "CLOSE_WINDOW" => {
                                if let Some(ref dm) = dm_arc {
                                    let _ = dm.switch_to_default_desktop();
                                }
                                *control_flow = ControlFlow::Exit;
                            }

                            // Navigation Controls
                            "NAVIGATE" => {
                                if let Some(url) = val.get("url").and_then(|u| u.as_str()) {
                                    let _active_id = {
                                        let mut tm = tab_manager.lock().unwrap();
                                        let id = tm.active_id();
                                        tm.update_tab(id, url.to_string(), "Loading...".to_string());
                                        id
                                    };

                                    if let Ok(content) = browser_content.lock() {
                                        let _ = content.load_url(url);
                                    }

                                    sync_tabs_to_chrome(&tab_manager, &browser_chrome);
                                }
                            }
                            "CONTENT_STATE_CHANGE" => {
                                if let (Some(url), Some(title)) = (
                                    val.get("url").and_then(|u| u.as_str()),
                                    val.get("title").and_then(|t| t.as_str()),
                                ) {
                                    let should_sync = {
                                        let mut tm = tab_manager.lock().unwrap();
                                        let active = tm.active_tab().cloned();
                                        if let Some(tab) = active {
                                            if tab.kind == TabKind::Web {
                                                tm.update_tab(tab.id, url.to_string(), title.to_string());
                                                true
                                            } else {
                                                false
                                            }
                                        } else {
                                            false
                                        }
                                    };
                                    if should_sync {
                                        sync_tabs_to_chrome(&tab_manager, &browser_chrome);
                                    }
                                }
                            }
                            "PAGE_LOAD_FINISHED" => {
                                if let Some(url) = val.get("url").and_then(|u| u.as_str()) {
                                    let should_sync = {
                                        let mut tm = tab_manager.lock().unwrap();
                                        if let Some(tab) = tm.active_tab().cloned() {
                                            if tab.kind == TabKind::Web {
                                                let title = if tab.title == "New Tab" || tab.title == "Loading..." {
                                                    url.to_string()
                                                } else {
                                                    tab.title.clone()
                                                };
                                                tm.update_tab(tab.id, url.to_string(), title);
                                                true
                                            } else {
                                                false
                                            }
                                        } else {
                                            false
                                        }
                                    };
                                    if should_sync {
                                        sync_tabs_to_chrome(&tab_manager, &browser_chrome);
                                    }
                                }
                            }
                            "GO_BACK" => {
                                if let Ok(content) = browser_content.lock() {
                                    let _ = content.go_back();
                                }
                            }
                            "GO_FORWARD" => {
                                if let Ok(content) = browser_content.lock() {
                                    let _ = content.go_forward();
                                }
                            }
                            "RELOAD" => {
                                if let Ok(content) = browser_content.lock() {
                                    let _ = content.reload();
                                }
                            }

                            // Tabs Management
                            "NEW_TAB" => {
                                let _new_id = {
                                    let mut tm = tab_manager.lock().unwrap();
                                    tm.open_tab("https://duckduckgo.com")
                                };
                                if let Ok(content) = browser_content.lock() {
                                    let _ = content.load_url("https://duckduckgo.com");
                                }
                                sync_tabs_to_chrome(&tab_manager, &browser_chrome);
                            }
                            "NEW_TAB_WITH_URL" => {
                                if let Some(url) = val.get("url").and_then(|u| u.as_str()) {
                                    let _new_id = {
                                        let mut tm = tab_manager.lock().unwrap();
                                        tm.open_tab(url)
                                    };
                                    if let Ok(content) = browser_content.lock() {
                                        let _ = content.load_url(url);
                                    }
                                    sync_tabs_to_chrome(&tab_manager, &browser_chrome);
                                }
                            }
                            "SWITCH_TAB" => {
                                if let Some(id) = val.get("id").and_then(|i| i.as_u64()) {
                                    let (target_url, kind) = {
                                        let mut tm = tab_manager.lock().unwrap();
                                        tm.switch_to_tab(id as usize);
                                        let active = tm.active_tab().cloned();
                                        match active {
                                            Some(t) => (t.url, t.kind),
                                            None => ("https://duckduckgo.com".to_string(), TabKind::Web),
                                        }
                                    };

                                    if let Ok(content) = browser_content.lock() {
                                        match kind {
                                            TabKind::Web => {
                                                let _ = content.load_url(&target_url);
                                            }
                                            TabKind::Bookmarks => {
                                                let bms = bookmark_store.lock().unwrap().list().to_vec();
                                                let html = generate_bookmarks_page_html(&bms);
                                                let _ = content.load_html(&html);
                                            }
                                            TabKind::Settings => {
                                                let html = generate_settings_page_html();
                                                let _ = content.load_html(&html);
                                            }
                                        }
                                    }
                                    sync_tabs_to_chrome(&tab_manager, &browser_chrome);
                                }
                            }
                            "CLOSE_TAB" => {
                                if let Some(id) = val.get("id").and_then(|i| i.as_u64()) {
                                    let next_tab = {
                                        let mut tm = tab_manager.lock().unwrap();
                                        tm.close_tab(id as usize);
                                        tm.active_tab().cloned()
                                    };
                                    if let (Some(tab), Ok(content)) = (next_tab, browser_content.lock()) {
                                        match tab.kind {
                                            TabKind::Web => {
                                                let _ = content.load_url(&tab.url);
                                            }
                                            TabKind::Bookmarks => {
                                                let bms = bookmark_store.lock().unwrap().list().to_vec();
                                                let _ = content.load_html(&generate_bookmarks_page_html(&bms));
                                            }
                                            TabKind::Settings => {
                                                let _ = content.load_html(&generate_settings_page_html());
                                            }
                                        }
                                    }
                                    sync_tabs_to_chrome(&tab_manager, &browser_chrome);
                                }
                            }
                            "OPEN_BOOKMARKS" => {
                                {
                                    let mut tm = tab_manager.lock().unwrap();
                                    tm.open_or_switch_special("Bookmarks", TabKind::Bookmarks);
                                }
                                if let Ok(content) = browser_content.lock() {
                                    let bms = bookmark_store.lock().unwrap().list().to_vec();
                                    let _ = content.load_html(&generate_bookmarks_page_html(&bms));
                                }
                                sync_tabs_to_chrome(&tab_manager, &browser_chrome);
                            }
                            "OPEN_SETTINGS" => {
                                {
                                    let mut tm = tab_manager.lock().unwrap();
                                    tm.open_or_switch_special("Settings", TabKind::Settings);
                                }
                                if let Ok(content) = browser_content.lock() {
                                    let _ = content.load_html(&generate_settings_page_html());
                                }
                                sync_tabs_to_chrome(&tab_manager, &browser_chrome);
                            }

                            // Bookmarks Management
                            "ADD_BOOKMARK" => {
                                let (title, url) = {
                                    let tm = tab_manager.lock().unwrap();
                                    let tab = tm.active_tab();
                                    match tab {
                                        Some(t) => (t.title.clone(), t.url.clone()),
                                        None => ("New Bookmark".to_string(), "https://duckduckgo.com".to_string()),
                                    }
                                };
                                if let Ok(mut store) = bookmark_store.lock() {
                                    let _ = store.add(&title, &url, BookmarkCategory::General);
                                }
                                if let Ok(chrome) = browser_chrome.lock() {
                                    let _ = chrome.evaluate_script("alert('Bookmark added to SafeBrowse store!');");
                                }
                            }
                            "ADD_BOOKMARK_DIRECT" => {
                                if let (Some(title), Some(url)) = (
                                    val.get("title").and_then(|t| t.as_str()),
                                    val.get("url").and_then(|u| u.as_str()),
                                ) {
                                    if let Ok(mut store) = bookmark_store.lock() {
                                        let _ = store.add(title, url, BookmarkCategory::General);
                                    }
                                    if let Ok(content) = browser_content.lock() {
                                        let bms = bookmark_store.lock().unwrap().list().to_vec();
                                        let _ = content.load_html(&generate_bookmarks_page_html(&bms));
                                    }
                                }
                            }

                            // Secure Virtual Keyboard Injection (Top-Level DOM Dispatch)
                            "KEY_INPUT" => {
                                if let Some(action) = val.get("action").and_then(|a| a.as_str()) {
                                    let injection_script = VirtualKeyboard::generate_dom_injection_script(action);
                                    if let Ok(content) = browser_content.lock() {
                                        let _ = content.evaluate_script(&injection_script);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            Event::WindowEvent {
                window_id,
                event: WindowEvent::Resized(phys_size),
                ..
            } if window_id == browser_window.id() => {
                let scale = browser_window.scale_factor();
                let width = phys_size.width as f64 / scale;
                let height = phys_size.height as f64 / scale;
                current_window_size = (width, height);

                let content_h = (height - BROWSER_CHROME_HEIGHT).max(10.0);

                if let Ok(chrome) = browser_chrome.lock() {
                    let _ = chrome.set_bounds(make_rect(0.0, 0.0, width, BROWSER_CHROME_HEIGHT));
                }
                if let Ok(content) = browser_content.lock() {
                    let _ = content.set_bounds(make_rect(0.0, BROWSER_CHROME_HEIGHT, width, content_h));
                }
            }

            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if window_id == browser_window.id() {
                    if let Some(ref dm) = dm_arc {
                        let _ = dm.switch_to_default_desktop();
                    }
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => {}
        }
    });
}

/// Helper function to synchronize tab state from `TabManager` to the Chrome webview.
fn sync_tabs_to_chrome(
    tab_manager: &Arc<Mutex<TabManager>>,
    browser_chrome: &Arc<Mutex<wry::WebView>>,
) {
    let (tabs_json, active_id) = {
        let tm = tab_manager.lock().unwrap();
        (serde_json::to_string(tm.list()).unwrap_or_else(|_| "[]".to_string()), tm.active_id())
    };

    if let Ok(chrome) = browser_chrome.lock() {
        let script = format!(
            "if (window.updateTabs) window.updateTabs({}, {});",
            tabs_json, active_id
        );
        let _ = chrome.evaluate_script(&script);
    }
}
