//! Kiosk Window & Native Shell Execution
//!
//! Creates the full-screen kiosk window, applies capture protection (`WDA_EXCLUDEFROMCAPTURE`),
//! configures defensive key interception, and runs the primary event loop.

use std::sync::{Arc, Mutex};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::platform::windows::WindowExtWindows;
use tao::window::{Fullscreen, WindowBuilder};
use windows::Win32::Foundation::HWND;
use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows};

use crate::bookmarks::BookmarkStore;
use crate::browser::{ProfileManager, ProfileMode};
use crate::config::{CHROMIUM_ARGS_SECURITY, DEFAULT_HOMEPAGE_URL};
use crate::desktop::DesktopManager;
use crate::keyboard::VirtualKeyboard;
use crate::security::{CaptureProtector, HotkeyInterceptor};
use crate::ui::assets::generate_kiosk_shell_html;

/// Launches and runs the full-screen kiosk browser window.
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
    let event_loop: EventLoop<String> = EventLoopBuilder::<String>::with_user_event().build();

    let target_url = initial_url.unwrap_or_else(|| DEFAULT_HOMEPAGE_URL.to_string());

    let mut window_builder = WindowBuilder::new()
        .with_title("SafeBrowse - Secure Banking & Payment Environment")
        .with_decorations(!is_fullscreen);

    if is_fullscreen {
        // Why: Borderless kiosk takes over the entire visual monitor area, matching SafePay.
        window_builder = window_builder.with_fullscreen(Some(Fullscreen::Borderless(None)));
    } else {
        window_builder = window_builder.with_inner_size(tao::dpi::LogicalSize::new(1280.0, 800.0));
    }

    let window = window_builder
        .build(&event_loop)
        .map_err(|e| format!("Failed to create Kiosk window: {}", e))?;

    let hwnd = HWND(window.hwnd() as *mut _);

    // Apply Screen Scraper & Screen Recorder Protection
    if let Err(e) = CaptureProtector::apply_protection(hwnd) {
        eprintln!("[SafeBrowse Warning] Capture protection notice: {}", e);
    }

    // Register defensive hotkey blockers (PrintScreen & Ctrl+Alt+D)
    let mut hotkey_interceptor = HotkeyInterceptor::new(hwnd);
    let _ = hotkey_interceptor.register_printscreen_blocker();
    let _ = hotkey_interceptor.register_desktop_toggle_hotkey();

    // Initialize Profile Sandbox
    let profile_mgr = ProfileManager::new(profile_mode)?;
    let data_dir = profile_mgr.data_directory().to_path_buf();
    let is_ephemeral = profile_mode == ProfileMode::Ephemeral;

    let bookmark_store = Arc::new(Mutex::new(BookmarkStore::initialize().unwrap_or_else(|_| {
        panic!("Failed to initialize secure bookmark store");
    })));

    let proxy = event_loop.create_proxy();
    let shell_html = generate_kiosk_shell_html(&target_url);

    let mut web_context = WebContext::new(Some(data_dir));
    let security_args = CHROMIUM_ARGS_SECURITY.join(" ");

    let webview = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_incognito(is_ephemeral)
        .with_html(shell_html)
        .with_devtools(false)
        .with_additional_browser_args(security_args)
        .with_ipc_handler(move |req| {
            let msg = req.body().clone();
            let _ = proxy.send_event(msg);
        })
        .build(&window)
        .map_err(|e| format!("Failed to initialize WebView2 inside kiosk: {}", e))?;

    let webview_shared = Arc::new(Mutex::new(webview));
    let webview_clone = Arc::clone(&webview_shared);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(ipc_msg) => {
                // Parse IPC JSON message
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&ipc_msg) {
                    if let Some(msg_type) = val.get("type").and_then(|t| t.as_str()) {
                        match msg_type {
                            "SWITCH_DESKTOP" => {
                                if let Some(ref dm) = desktop_manager {
                                    let _ = dm.switch_to_default_desktop();
                                }
                            }
                            "EXIT_APP" => {
                                if let Some(ref dm) = desktop_manager {
                                    let _ = dm.switch_to_default_desktop();
                                }
                                *control_flow = ControlFlow::Exit;
                            }
                            "KEY_INPUT" => {
                                if let Some(action) = val.get("action").and_then(|a| a.as_str()) {
                                    // Generate DOM injection script
                                    let script = VirtualKeyboard::generate_dom_injection_script(action);
                                    let iframe_wrapper = format!(
                                        r#"try {{
                                            const f = document.getElementById('content-frame');
                                            if (f && f.contentWindow && f.contentWindow.document) {{
                                                f.contentWindow.eval({:?});
                                            }}
                                        }} catch(e) {{
                                            console.error("DOM Injection cross-origin fallback:", e);
                                        }}"#,
                                        script
                                    );
                                    if let Ok(view) = webview_clone.lock() {
                                        let _ = view.evaluate_script(&iframe_wrapper);
                                    }
                                }
                            }
                            "ADD_BOOKMARK" => {
                                if let (Some(title), Some(url)) = (
                                    val.get("title").and_then(|t| t.as_str()),
                                    val.get("url").and_then(|u| u.as_str()),
                                ) {
                                    if let Ok(mut store) = bookmark_store.lock() {
                                        let _ = store.add(title, url, crate::bookmarks::BookmarkCategory::General);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                if let Some(ref dm) = desktop_manager {
                    let _ = dm.switch_to_default_desktop();
                }
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
