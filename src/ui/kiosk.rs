//! Native browser session. Trusted controls and website events have separate channels.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::ops::Deref;
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::{EventLoopBuilderExtWindows, WindowExtWindows};
#[cfg(test)]
use tao::window::WindowBuilder;
use tao::window::{Fullscreen, Window};
use windows::Win32::Foundation::HWND;
use wry::{Rect, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

use super::assets::{
    generate_bookmarks_page_html, generate_browser_chrome_html_with_session,
    generate_capture_warning_html, generate_desktop_shell_html_with_session,
    generate_language_picker_html, generate_settings_page_html_with_session,
    generate_virtual_keyboard_html,
};
use super::floating_keyboard::FloatingKeyboard;
use super::native::WindowProcedureGuard;
pub use super::native::{
    clamp_window_pos, clamp_window_rect, MIN_VISIBLE_SIDE_WIDTH, MIN_VISIBLE_TOP_HEIGHT,
};
use super::permission_ui::PermissionUi;
use super::shell_windows::{self, LANGUAGE_PICKER_HEIGHT, LANGUAGE_PICKER_WIDTH};
use super::trusted::{TrustedDocument, TrustedWebView};
use crate::bookmarks::{BookmarkCategory, BookmarkStore};
use crate::browser::downloads::{DownloadAttachment, DownloadBroker, DownloadEvent};
use crate::browser::health::{BrowserHealthEvent, BrowserHealthMonitor};
use crate::browser::navigation::{normalize_navigation_input, validate_web_url};
use crate::browser::permissions::{PermissionDecision, PermissionStore, SitePermission};
use crate::browser::printing::{
    require_printing_enabled, show_system_print_dialog, WEBSITE_PRINT_GUARD,
};
use crate::browser::requests::{RequestAttachment, RequestBroker, RequestEvent};
use crate::browser::security::harden_content_view;
use crate::browser::tabs::{TabKind, TabManager};
use crate::browser::{ProfileManager, ProfileMode};
use crate::config::{CHROMIUM_ARGS_SECURITY, DEFAULT_HOMEPAGE_URL};
use crate::desktop::DesktopManager;
use crate::keyboard::language::{self, InputLanguageState};
use crate::keyboard::VirtualKeyboard;
use crate::security::{CaptureProtector, HotkeyInterceptor};

pub const DESKTOP_TASKBAR_HEIGHT: i32 = 46;
pub const BROWSER_CHROME_HEIGHT: f64 = 110.0;
pub const BROWSER_OSK_HEIGHT: f64 = 230.0;
const MIN_WINDOW_WIDTH: f64 = 720.0;
const MIN_WINDOW_HEIGHT: f64 = 560.0;
const DEFAULT_WINDOW_WIDTH: f64 = 1180.0;
const DEFAULT_WINDOW_HEIGHT: f64 = 800.0;
const MAX_OPEN_TABS: usize = 24;
const MAX_IPC_BYTES: usize = 16 * 1024;
const WINDOW_SCREEN_MARGIN: f64 = 48.0;

mod file_requests;
mod site_requests;

/// Identifies a bundled surface; websites never construct a trusted event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
    Chrome,
    Taskbar,
    Keyboard,
    Internal,
    Warning,
    LanguagePicker,
    PermissionPrompt,
    DownloadPrompt,
}

/// Native callbacks bind website metadata to the originating tab, even in the background.
#[derive(Debug, Clone)]
pub(crate) enum KioskEvent {
    Trusted(Surface, String),
    Ready,
    PageLoad {
        id: usize,
        loading: bool,
    },
    Title {
        id: usize,
        title: String,
    },
    SourceChanged(usize),
    NavigationFailed(usize),
    ContentFocused(usize),
    InternalFocused,
    Shortcut {
        id: usize,
        command: &'static str,
    },
    Notice(&'static str),
    SwitchDesktop,
    BrowserRequest(RequestEvent),
    Download(DownloadEvent),
    PermissionProfileReady(Result<(), String>),
    EngineHealth {
        tab_id: Option<usize>,
        event: BrowserHealthEvent,
    },
}

/// The keyboard bridge can edit only its selected field and change its own presentation.
fn keyboard_command_allowed(surface: Surface, command: &str) -> bool {
    if surface == Surface::DownloadPrompt {
        return matches!(command, "UI_READY" | "RESOLVE_DOWNLOAD");
    }
    let keyboard_only = matches!(
        command,
        "KEY_INPUT" | "DETACH_OSK" | "ATTACH_OSK" | "START_OSK_DRAG"
    );
    if keyboard_only {
        return surface == Surface::Keyboard;
    }
    surface != Surface::Keyboard || matches!(command, "TOGGLE_OSK" | "UI_READY")
}

/// Rejects queued print commands if their source tab is no longer the active website.
fn validate_print_request(
    surface: Surface,
    tab_id: usize,
    tabs: &TabManager,
) -> Result<(), String> {
    if surface != Surface::Chrome {
        return Err(
            "Printing is only available from the browser's Print control or Ctrl+P.".into(),
        );
    }
    if tab_id != tabs.active_id() {
        return Err(
            "The active tab changed. Choose Print again on the page you want to print.".into(),
        );
    }
    if !tabs
        .active_tab()
        .is_some_and(|tab| tab.kind == TabKind::Web)
    {
        return Err("Open a website to print it.".into());
    }
    Ok(())
}

/// Computes logical child-view bounds without mixing physical and logical DPI units.
pub fn make_rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect {
        position: wry::dpi::Position::Logical(tao::dpi::LogicalPosition::new(x, y)),
        size: wry::dpi::Size::Logical(LogicalSize::new(width.max(1.0), height.max(1.0))),
    }
}

/// Builds a bundled control surface that cannot navigate to remote or user-provided HTML.
pub(super) fn build_trusted_view(
    window: &Window,
    context: &mut WebContext,
    html: String,
    surface: Surface,
    proxy: &EventLoopProxy<KioskEvent>,
) -> Result<TrustedWebView, String> {
    let profile_path = context
        .data_directory()
        .ok_or("A control profile is required")?
        .to_owned();
    let ipc_proxy = proxy.clone();
    let ready_proxy = proxy.clone();
    let document = TrustedDocument::new(&html);
    let navigation_document = document.clone();
    let view = WebViewBuilder::new_with_web_context(context)
        .with_visible(false)
        .with_bounds(make_rect(0.0, 0.0, 1.0, 1.0))
        .with_devtools(false)
        .with_initialization_script(CONTENT_INPUT_SCRIPT)
        .with_browser_accelerator_keys(false)
        .with_hotkeys_zoom(false)
        .with_default_context_menus(false)
        .with_additional_browser_args(CHROMIUM_ARGS_SECURITY.join(" "))
        .with_navigation_handler(move |url| navigation_document.allows_navigation(&url))
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_permission_handler(|_| wry::PermissionResponse::Deny)
        .with_on_page_load_handler(move |event, _| {
            if matches!(event, wry::PageLoadEvent::Finished) {
                let _ = ready_proxy.send_event(KioskEvent::Ready);
            }
        })
        .with_ipc_handler(move |request| {
            if request.body().len() <= MAX_IPC_BYTES {
                let _ = ipc_proxy.send_event(KioskEvent::Trusted(surface, request.body().clone()));
            }
        })
        .build_as_child(window)
        .map_err(|error| format!("Cannot create {surface:?} view: {error}"))?;
    crate::browser::runtime::validate_created_environment(&view, &profile_path)?;
    let health_proxy = proxy.clone();
    let health = BrowserHealthMonitor::attach(&view, move |event| {
        let _ = health_proxy.send_event(KioskEvent::EngineHealth {
            tab_id: None,
            event,
        });
    })?;
    view.load_html(&html)
        .map_err(|error| format!("Cannot load {surface:?} controls: {error}"))?;
    Ok(TrustedWebView::new(view, document, health))
}

/// Holds only the last explicitly focused editable element; keyboard input never guesses a field.
const CONTENT_INPUT_SCRIPT: &str = r#"
(() => {
  window.__safebrowse_last_input = null;
  document.addEventListener('focusin', event => {
    const field = event.composedPath()[0];
    if (field && (field.tagName === 'INPUT' || field.tagName === 'TEXTAREA' || field.isContentEditable)) {
      window.__safebrowse_last_input = field;
    }
  }, true);
})();
"#;

/// Creates one WebView per web tab without exposing any app messaging bridge to websites.
fn build_content_view(
    window: &Window,
    context: &mut WebContext,
    id: usize,
    proxy: &EventLoopProxy<KioskEvent>,
    requests: &RequestBroker,
    downloads: &DownloadBroker,
    environment: Option<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment>,
) -> Result<ContentView, String> {
    let profile_path = context
        .data_directory()
        .ok_or("A website profile is required")?
        .to_owned();
    let load_proxy = proxy.clone();
    let title_proxy = proxy.clone();
    let navigation_proxy = proxy.clone();
    let is_popup = environment.is_some();
    let mut builder = WebViewBuilder::new_with_web_context(context)
        .with_visible(false)
        .with_bounds(make_rect(0.0, 0.0, 1.0, 1.0))
        .with_devtools(false)
        .with_browser_accelerator_keys(false)
        .with_clipboard(false)
        .with_default_context_menus(false)
        .with_general_autofill_enabled(false)
        .with_additional_browser_args(CHROMIUM_ARGS_SECURITY.join(" "))
        .with_initialization_script(CONTENT_INPUT_SCRIPT)
        .with_initialization_script_for_main_only(WEBSITE_PRINT_GUARD, false)
        .with_navigation_handler(move |target| {
            let allowed =
                validate_web_url(&target).is_ok() || (is_popup && target == "about:blank");
            if !allowed {
                let _ = navigation_proxy.send_event(KioskEvent::Notice(
                    "Only HTTP and HTTPS web addresses are supported.",
                ));
            }
            allowed
        })
        .with_on_page_load_handler(move |event, _| {
            let _ = load_proxy.send_event(KioskEvent::PageLoad {
                id,
                loading: matches!(event, wry::PageLoadEvent::Started),
            });
        })
        .with_document_title_changed_handler(move |title| {
            let _ = title_proxy.send_event(KioskEvent::Title { id, title });
        });
    if let Some(environment) = environment {
        builder = builder.with_environment(environment);
    }
    let view = builder
        .build_as_child(window)
        .map_err(|error| format!("Cannot create browser tab: {error}"))?;
    crate::browser::runtime::validate_created_environment(&view, &profile_path)?;
    harden_content_view(&view)?;
    let health_proxy = proxy.clone();
    let health = BrowserHealthMonitor::attach(&view, move |event| {
        let _ = health_proxy.send_event(KioskEvent::EngineHealth {
            tab_id: Some(id),
            event,
        });
    })?;
    attach_navigation_events(&view, id, proxy)?;
    attach_input_events(&view, id, proxy)?;
    let request_proxy = proxy.clone();
    let attachment = requests.attach(&view, id, move |event| {
        let _ = request_proxy.send_event(KioskEvent::BrowserRequest(event));
    })?;
    let download_proxy = proxy.clone();
    let download_attachment = downloads.attach(&view, id, move |event| {
        let _ = download_proxy.send_event(KioskEvent::Download(event));
    })?;
    Ok(ContentView {
        _health: health,
        _requests: attachment,
        _downloads: download_attachment,
        view,
        is_popup,
    })
}

/// Unsubscribes native callbacks and cancels deferrals before destroying a page.
struct ContentView {
    _health: BrowserHealthMonitor,
    _requests: RequestAttachment,
    _downloads: DownloadAttachment,
    view: WebView,
    is_popup: bool,
}

impl Deref for ContentView {
    type Target = WebView;
    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

/// Routes physical browser shortcuts and native focus without trusting page JavaScript.
fn attach_input_events(
    view: &WebView,
    id: usize,
    proxy: &EventLoopProxy<KioskEvent>,
) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
    };
    use webview2_com::{AcceleratorKeyPressedEventHandler, FocusChangedEventHandler};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
    let focus_proxy = proxy.clone();
    let key_proxy = proxy.clone();
    let attach = || unsafe {
        let controller = view.controller();
        let mut token = 0;
        controller.add_GotFocus(
            &FocusChangedEventHandler::create(Box::new(move |_, _| {
                let _ = focus_proxy.send_event(KioskEvent::ContentFocused(id));
                Ok(())
            })),
            &mut token,
        )?;
        controller.add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut kind = Default::default();
                args.KeyEventKind(&mut kind)?;
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                    && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                {
                    return Ok(());
                }
                let mut key = 0;
                args.VirtualKey(&mut key)?;
                let control = GetKeyState(i32::from(VK_CONTROL.0)) < 0;
                let alt = GetKeyState(i32::from(VK_MENU.0)) < 0;
                let shift = GetKeyState(i32::from(VK_SHIFT.0)) < 0;
                if let Some(command) = browser_shortcut(key, control, alt, shift) {
                    args.SetHandled(true)?;
                    if command == "PRINT" {
                        let mut physical_status = Default::default();
                        args.PhysicalKeyStatus(&mut physical_status)?;
                        if physical_status.WasKeyDown.as_bool() {
                            return Ok(());
                        }
                    }
                    let _ = key_proxy.send_event(KioskEvent::Shortcut { id, command });
                }
                Ok(())
            })),
            &mut token,
        )
    };
    attach().map_err(|error| format!("Cannot monitor browser focus: {error}"))
}

/// Maps only physical browser shortcuts; normal page typing remains untouched.
fn browser_shortcut(key: u32, control: bool, alt: bool, shift: bool) -> Option<&'static str> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        VIRTUAL_KEY, VK_D, VK_F5, VK_L, VK_LEFT, VK_P, VK_R, VK_RIGHT, VK_T, VK_TAB, VK_W,
    };
    let key = VIRTUAL_KEY(u16::try_from(key).ok()?);
    match (key, control, alt, shift) {
        (VK_L, true, false, false) => Some("FOCUS_ADDRESS"),
        (VK_T, true, false, false) => Some("NEW_TAB"),
        (VK_W, true, false, false) => Some("CLOSE_TAB"),
        (VK_R, true, false, _) | (VK_F5, false, false, _) => Some("RELOAD"),
        (VK_D, true, false, false) => Some("ADD_BOOKMARK"),
        (VK_P, true, false, false) => Some("PRINT"),
        (VK_TAB, true, false, false) => Some("NEXT_TAB"),
        (VK_TAB, true, false, true) => Some("PREVIOUS_TAB"),
        (VK_LEFT, false, true, false) => Some("GO_BACK"),
        (VK_RIGHT, false, true, false) => Some("GO_FORWARD"),
        _ => None,
    }
}

/// Reads navigation state from the engine, including same-document history changes and failed loads.
fn attach_navigation_events(
    view: &WebView,
    id: usize,
    proxy: &EventLoopProxy<KioskEvent>,
) -> Result<(), String> {
    use webview2_com::{
        HistoryChangedEventHandler, NavigationCompletedEventHandler, SourceChangedEventHandler,
    };
    let source_proxy = proxy.clone();
    let history_proxy = proxy.clone();
    let failure_proxy = proxy.clone();
    let attach = || unsafe {
        let core = view.controller().CoreWebView2()?;
        let mut token = 0;
        core.add_SourceChanged(
            &SourceChangedEventHandler::create(Box::new(move |_, _| {
                let _ = source_proxy.send_event(KioskEvent::SourceChanged(id));
                Ok(())
            })),
            &mut token,
        )?;
        core.add_HistoryChanged(
            &HistoryChangedEventHandler::create(Box::new(move |_, _| {
                let _ = history_proxy.send_event(KioskEvent::SourceChanged(id));
                Ok(())
            })),
            &mut token,
        )?;
        core.add_NavigationCompleted(&NavigationCompletedEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                let mut success = Default::default();
                args.IsSuccess(&mut success)?;
                if !success.as_bool() {
                    let mut status = Default::default();
                    args.WebErrorStatus(&mut status)?;
                    if status != webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                        let _ = failure_proxy.send_event(KioskEvent::NavigationFailed(id));
                    }
                }
            }
            Ok(())
        })), &mut token)
    };
    attach().map_err(|error| format!("Cannot monitor page navigation: {error}"))
}

#[derive(Clone, Copy)]
enum InputTarget {
    Address,
    Content(usize),
    Internal,
}

/// Prevents navigation until legacy engine grants have been reset to app-managed decisions.
enum PermissionProfileState {
    Uninitialized,
    Loading,
    Ready,
    Failed(String),
}

/// Owns views before contexts and the native window so their destructors run in dependency order.
struct BrowserSession {
    content: HashMap<usize, ContentView>,
    chrome: TrustedWebView,
    taskbar: TrustedWebView,
    keyboard: TrustedWebView,
    internal: TrustedWebView,
    warning: Option<TrustedWebView>,
    language_picker: TrustedWebView,
    permission_ui: PermissionUi,
    download_ui: PermissionUi,
    requests: RequestBroker,
    downloads: DownloadBroker,
    browser_context: WebContext,
    _shell_context: WebContext,
    _window_procedure: WindowProcedureGuard,
    _hotkeys: HotkeyInterceptor,
    language_bar_guard: Option<crate::keyboard::ScopedLanguageBarGuard>,
    taskbar_window: Option<Window>,
    language_window: Window,
    floating_keyboard: FloatingKeyboard,
    window: Window,
    tabs: TabManager,
    bookmarks: BookmarkStore,
    permissions: PermissionStore,
    permission_profile_state: PermissionProfileState,
    pending_navigations: HashMap<usize, String>,
    desktop: Option<DesktopManager>,
    proxy: EventLoopProxy<KioskEvent>,
    capture_allowed: bool,
    temporary_profile: bool,
    warning_pending: bool,
    keyboard_visible: bool,
    language_picker_visible: bool,
    input_language: Option<InputLanguageState>,
    input_target: InputTarget,
    last_internal_kind: Option<TabKind>,
    terminal_error: Option<String>,
    runtime_update_available: bool,
}

impl BrowserSession {
    /// Creates chrome first and keeps websites uninitialized until the recording warning is accepted.
    fn new(
        target: &EventLoopWindowTarget<KioskEvent>,
        proxy: EventLoopProxy<KioskEvent>,
        profile: &ProfileManager,
        url: &str,
        desktop: Option<DesktopManager>,
        is_fullscreen: bool,
        capture_allowed: bool,
    ) -> Result<Self, String> {
        let language_bar_guard = if let Some(desktop_manager) = desktop.as_ref() {
            match crate::keyboard::ScopedLanguageBarGuard::install_for_current_thread(
                desktop_manager.safe_desktop_name(),
            ) {
                Ok(guard) => Some(guard),
                Err(error) => {
                    eprintln!(
                        "[SafeBrowse] Floating language-bar suppression unavailable: {error}"
                    );
                    None
                }
            }
        } else {
            None
        };
        let window = super::branding::window_builder()
            .map_err(|error| format!("Cannot load the SafeBrowse window icon: {error}"))?
            .with_title("SafeBrowse")
            .with_visible(false)
            .with_decorations(false)
            .with_resizable(true)
            .with_inner_size(LogicalSize::new(
                DEFAULT_WINDOW_WIDTH,
                DEFAULT_WINDOW_HEIGHT,
            ))
            .with_min_inner_size(LogicalSize::new(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT))
            .build(target)
            .map_err(|error| format!("Cannot create window: {error}"))?;
        if let Some(monitor) = window.current_monitor() {
            let size = monitor.size().to_logical::<f64>(window.scale_factor());
            let width = DEFAULT_WINDOW_WIDTH.min(size.width);
            let height = DEFAULT_WINDOW_HEIGHT.min(size.height - WINDOW_SCREEN_MARGIN);
            window.set_inner_size(LogicalSize::new(width, height));
            let origin = monitor.position();
            let scale = window.scale_factor();
            window.set_outer_position(PhysicalPosition::new(
                origin.x + ((size.width - width) * scale / 2.0) as i32,
                origin.y + ((size.height - height) * scale / 2.0) as i32,
            ));
        }
        let hwnd = HWND(window.hwnd() as *mut _);
        if !capture_allowed {
            CaptureProtector::apply_protection(hwnd)?;
        }
        let window_procedure = WindowProcedureGuard::install(hwnd, proxy.clone())?;
        let mut hotkeys = HotkeyInterceptor::new(hwnd);
        if !capture_allowed {
            if let Err(error) = hotkeys.register_printscreen_blocker() {
                eprintln!("[SafeBrowse] PrintScreen shortcut unavailable: {error}");
            }
        }
        let tabs = TabManager::new(url);
        let isolated = desktop.is_some();
        let taskbar_window = if isolated {
            Some(shell_windows::create_shell_window(
                target,
                &window,
                "SafeBrowse taskbar",
                LogicalSize::new(DEFAULT_WINDOW_WIDTH, shell_windows::TASKBAR_HEIGHT),
                capture_allowed,
            )?)
        } else {
            None
        };
        let language_window = shell_windows::create_shell_window(
            target,
            &window,
            "SafeBrowse input language",
            LogicalSize::new(LANGUAGE_PICKER_WIDTH, LANGUAGE_PICKER_HEIGHT),
            capture_allowed,
        )?;
        let floating_keyboard = FloatingKeyboard::new(target, &window, capture_allowed)?;
        let mut shell_context = WebContext::new(Some(profile.data_directory().join("shell")));
        let browser_context = WebContext::new(Some(profile.data_directory().join("web")));
        let chrome = build_trusted_view(
            &window,
            &mut shell_context,
            generate_browser_chrome_html_with_session(
                tabs.list(),
                tabs.active_id(),
                capture_allowed,
                isolated,
            ),
            Surface::Chrome,
            &proxy,
        )?;
        let taskbar = build_trusted_view(
            taskbar_window.as_ref().unwrap_or(&window),
            &mut shell_context,
            generate_desktop_shell_html_with_session(capture_allowed, isolated),
            Surface::Taskbar,
            &proxy,
        )?;
        let keyboard = build_trusted_view(
            &window,
            &mut shell_context,
            generate_virtual_keyboard_html(),
            Surface::Keyboard,
            &proxy,
        )?;
        let internal = build_trusted_view(
            &window,
            &mut shell_context,
            "<!doctype html><title>SafeBrowse</title>".to_owned(),
            Surface::Internal,
            &proxy,
        )?;
        let internal_proxy = proxy.clone();
        unsafe {
            internal
                .controller()
                .add_GotFocus(
                    &webview2_com::FocusChangedEventHandler::create(Box::new(move |_, _| {
                        let _ = internal_proxy.send_event(KioskEvent::InternalFocused);
                        Ok(())
                    })),
                    &mut 0,
                )
                .map_err(|error| error.to_string())?;
        }
        let warning = if capture_allowed {
            Some(build_trusted_view(
                &window,
                &mut shell_context,
                generate_capture_warning_html(),
                Surface::Warning,
                &proxy,
            )?)
        } else {
            None
        };
        let language_picker = build_trusted_view(
            &language_window,
            &mut shell_context,
            generate_language_picker_html(),
            Surface::LanguagePicker,
            &proxy,
        )?;
        let permission_ui =
            PermissionUi::new(target, &window, &mut shell_context, &proxy, capture_allowed)?;
        let download_ui = PermissionUi::new_download(
            target,
            &window,
            &mut shell_context,
            &proxy,
            capture_allowed,
        )?;
        let mut session = Self {
            content: HashMap::new(),
            chrome,
            taskbar,
            keyboard,
            internal,
            warning,
            language_picker,
            permission_ui,
            download_ui,
            requests: RequestBroker::new(),
            downloads: DownloadBroker::new(),
            browser_context,
            _shell_context: shell_context,
            _window_procedure: window_procedure,
            _hotkeys: hotkeys,
            language_bar_guard,
            taskbar_window,
            language_window,
            floating_keyboard,
            window,
            tabs,
            bookmarks: BookmarkStore::initialize()?,
            permissions: PermissionStore::initialize()?,
            permission_profile_state: PermissionProfileState::Uninitialized,
            pending_navigations: HashMap::new(),
            desktop,
            proxy,
            capture_allowed,
            temporary_profile: profile.mode() == ProfileMode::Ephemeral,
            warning_pending: capture_allowed,
            keyboard_visible: false,
            language_picker_visible: false,
            input_language: None,
            input_target: InputTarget::Address,
            last_internal_kind: None,
            terminal_error: None,
            runtime_update_available: false,
        };
        session.layout()?;
        if !capture_allowed {
            session.show_active_tab()?;
        }
        // Tao may reveal a window when entering fullscreen, so protect and populate it first.
        if is_fullscreen {
            session.window.set_fullscreen(Some(Fullscreen::Borderless(
                session.window.current_monitor(),
            )));
            session.layout()?;
        }
        session.window.set_visible(true);
        Ok(session)
    }

    /// Bounds updates are O(T) for T open views; a fixed tab limit bounds native resources.
    fn layout(&self) -> Result<(), String> {
        let size = self
            .window
            .inner_size()
            .to_logical::<f64>(self.window.scale_factor());
        let taskbar_height = if let Some(taskbar_window) = &self.taskbar_window {
            shell_windows::position_taskbar(taskbar_window, &self.window);
            shell_windows::taskbar_overlap(&self.window, taskbar_window)
        } else {
            f64::from(DESKTOP_TASKBAR_HEIGHT)
        };
        let keyboard_height = if self.keyboard_visible && !self.floating_keyboard.is_detached() {
            BROWSER_OSK_HEIGHT
        } else {
            0.0
        };
        let content_height =
            (size.height - BROWSER_CHROME_HEIGHT - taskbar_height - keyboard_height).max(1.0);
        let content_bounds = make_rect(0.0, BROWSER_CHROME_HEIGHT, size.width, content_height);
        if self.language_picker_visible {
            shell_windows::position_language_picker(
                &self.language_window,
                self.taskbar_window.as_ref().unwrap_or(&self.window),
            );
        }
        let apply = || -> wry::Result<()> {
            self.chrome
                .set_bounds(make_rect(0.0, 0.0, size.width, BROWSER_CHROME_HEIGHT))?;
            if let Some(taskbar_window) = &self.taskbar_window {
                let bar_size = taskbar_window
                    .inner_size()
                    .to_logical::<f64>(taskbar_window.scale_factor());
                self.taskbar
                    .set_bounds(make_rect(0.0, 0.0, bar_size.width, bar_size.height))?;
                if taskbar_window.is_visible() == self.warning_pending {
                    taskbar_window.set_visible(!self.warning_pending);
                }
            } else {
                self.taskbar.set_bounds(make_rect(
                    0.0,
                    size.height - taskbar_height,
                    size.width,
                    taskbar_height,
                ))?;
            }
            self.floating_keyboard.layout(
                &self.keyboard,
                &self.window,
                make_rect(
                    0.0,
                    size.height - taskbar_height - keyboard_height,
                    size.width,
                    keyboard_height,
                ),
                !self.warning_pending && self.keyboard_visible,
            )?;
            self.internal.set_bounds(content_bounds)?;
            for view in self.content.values() {
                view.set_bounds(content_bounds)?;
            }
            self.chrome.set_visible(!self.warning_pending)?;
            self.taskbar.set_visible(!self.warning_pending)?;
            if let Some(warning) = &self.warning {
                warning.set_bounds(make_rect(0.0, 0.0, size.width, size.height))?;
                warning.set_visible(self.warning_pending)?;
            }
            let picker_size = self
                .language_window
                .inner_size()
                .to_logical::<f64>(self.language_window.scale_factor());
            self.language_picker.set_bounds(make_rect(
                0.0,
                0.0,
                picker_size.width,
                picker_size.height,
            ))?;
            self.language_picker
                .set_visible(self.language_picker_visible && !self.warning_pending)?;
            Ok(())
        };
        apply().map_err(|error| format!("Cannot resize browser controls: {error}"))
    }

    /// Switches visibility without reloading pages, preserving forms, history, and scroll position.
    fn show_active_tab(&mut self) -> Result<(), String> {
        if self.warning_pending {
            return Ok(());
        }
        let active = self
            .tabs
            .active_tab()
            .cloned()
            .ok_or("No active browser tab")?;
        if active.kind == TabKind::Web && !self.content.contains_key(&active.id) {
            let view = build_content_view(
                &self.window,
                &mut self.browser_context,
                active.id,
                &self.proxy,
                &self.requests,
                &self.downloads,
                None,
            )?;
            self.content.insert(active.id, view);
            self.navigate_after_permission_reset(active.id, &active.url)?;
        }
        for (&id, view) in &self.content {
            view.set_visible(id == active.id && active.kind == TabKind::Web)
                .map_err(|error| error.to_string())?;
        }
        self.internal
            .set_visible(active.kind != TabKind::Web)
            .map_err(|error| error.to_string())?;
        if active.kind != TabKind::Web && self.last_internal_kind != Some(active.kind) {
            self.refresh_internal_page(active.kind)?;
        }
        self.layout()?;
        self.sync_chrome();
        self.sync_request_prompt()?;
        Ok(())
    }

    fn refresh_internal_page(&mut self, kind: TabKind) -> Result<(), String> {
        let html = match kind {
            TabKind::Bookmarks => generate_bookmarks_page_html(self.bookmarks.list()),
            TabKind::Settings => generate_settings_page_html_with_session(
                self.capture_allowed,
                self.desktop.is_some(),
                self.temporary_profile,
            ),
            TabKind::Web => return Ok(()),
        };
        self.internal
            .load_html(&html)
            .map_err(|error| error.to_string())?;
        self.last_internal_kind = Some(kind);
        Ok(())
    }

    fn sync_chrome(&self) {
        let script = format!("window.updateTabs?.({},{}); window.setOskActive?.({}); window.setMaximizedState?.({});",
            json!(self.tabs.list()), self.tabs.active_id(), self.keyboard_visible,
            self.window.is_maximized() || self.window.fullscreen().is_some());
        if let Err(error) = self.chrome.evaluate_script(&script) {
            eprintln!("[SafeBrowse] Cannot update controls: {error}");
        }
        let _ = self.chrome.evaluate_script(&format!(
            "window.setRuntimeUpdateAvailable?.({});",
            self.runtime_update_available
        ));
        let mut back = Default::default();
        let mut forward = Default::default();
        if let Some(view) = self.content.get(&self.tabs.active_id()) {
            unsafe {
                if let Ok(core) = view.controller().CoreWebView2() {
                    let _ = core.CanGoBack(&mut back);
                    let _ = core.CanGoForward(&mut forward);
                }
            }
        }
        let _ = self.chrome.evaluate_script(&format!(
            "window.updateNavigationState?.({},{});",
            back.as_bool(),
            forward.as_bool()
        ));
    }

    /// Child views need this notification to keep native popups aligned after a window move.
    fn notify_window_moved(&self) -> Result<(), String> {
        let fixed_views = [
            &*self.chrome,
            &*self.taskbar,
            &*self.keyboard,
            &*self.internal,
        ];
        for view in fixed_views
            .into_iter()
            .chain(self.warning.iter().map(|warning| &**warning))
            .chain(self.content.values().map(|content| &content.view))
        {
            unsafe { view.controller().NotifyParentWindowPositionChanged() }
                .map_err(|error| format!("Cannot update browser window position: {error}"))?;
        }
        Ok(())
    }

    fn notice(&self, message: &str, error: bool) {
        let payload = json!(message);
        let _ = self
            .chrome
            .evaluate_script(&format!("window.showStatus?.({payload},{error});"));
        let _ = self
            .internal
            .evaluate_script(&format!("window.showBookmarkStatus?.({payload},{error});"));
        let _ = self
            .internal
            .evaluate_script(&format!("window.showStatus?.({payload},{error});"));
    }

    /// Retains the editing destination while a shell surface temporarily owns focus.
    fn input_view(&self) -> &WebView {
        match self.input_target {
            InputTarget::Content(id) => self
                .content
                .get(&id)
                .map(|content| &content.view)
                .unwrap_or(&self.chrome),
            InputTarget::Internal => &self.internal,
            InputTarget::Address => &self.chrome,
        }
    }

    /// Resolves only an owned WebView host for Windows input-language requests.
    fn input_window(&self) -> Result<HWND, String> {
        let mut host = Default::default();
        unsafe { self.input_view().controller().ParentWindow(&mut host) }
            .map_err(|error| error.to_string())?;
        Ok(HWND(host.0))
    }

    fn update_input_language(&mut self, state: InputLanguageState) -> Result<(), String> {
        let changed_layout = self
            .input_language
            .as_ref()
            .map(|previous| &previous.active_id)
            != Some(&state.active_id);
        if let Some(active) = state.active() {
            self.taskbar
                .evaluate_script(&format!(
                    "window.updateLanguageUI?.({});",
                    json!(active.code)
                ))
                .map_err(|error| error.to_string())?;
            if changed_layout {
                let rows = language::virtual_key_rows(&active.id)?;
                self.keyboard
                    .evaluate_script(&format!(
                        "window.setKeyboardLayout?.({},{},{});",
                        json!(rows),
                        json!(active.label),
                        active.ime
                    ))
                    .map_err(|error| error.to_string())?;
            }
        }
        self.language_picker
            .evaluate_script(&format!("window.updateLanguages?.({});", json!(state)))
            .map_err(|error| error.to_string())?;
        self.input_language = Some(state);
        Ok(())
    }

    fn refresh_input_language(&mut self) -> Result<(), String> {
        if let Some(guard) = &mut self.language_bar_guard {
            if let Err(error) = guard.refresh() {
                eprintln!("[SafeBrowse] Cannot refresh floating language-bar suppression: {error}");
            }
        }
        self.update_input_language(language::snapshot(self.input_window()?)?)
    }

    fn set_language_picker_visible(&mut self, visible: bool) -> Result<(), String> {
        if !visible && !self.language_picker_visible {
            return Ok(());
        }
        if visible {
            self.refresh_input_language()?;
            shell_windows::position_language_picker(
                &self.language_window,
                self.taskbar_window.as_ref().unwrap_or(&self.window),
            );
        }
        self.language_picker_visible = visible;
        if !visible {
            self.language_window.set_visible(false);
        }
        let result = (|| {
            self.layout()?;
            if visible {
                shell_windows::show_focused_popup(&self.language_window);
                self.language_picker
                    .focus()
                    .map_err(|error| error.to_string())?;
                self.language_picker
                    .evaluate_script(
                        "window.showLanguageError?.('');window.focusSelectedLanguage?.();",
                    )
                    .map_err(|error| error.to_string())?;
            }
            self.taskbar
                .evaluate_script(&format!("window.setLanguagePickerOpen?.({visible});"))
                .map_err(|error| error.to_string())
        })();
        if result.is_err() && visible {
            // A failed show must not leave a hidden popup marked open and block the next toggle.
            self.language_picker_visible = false;
            self.language_window.set_visible(false);
            let _ = self.language_picker.set_visible(false);
            let _ = self
                .taskbar
                .evaluate_script("window.setLanguagePickerOpen?.(false);");
        }
        result
    }

    /// Explicit dismissal returns to the editor; background blur closures must not steal focus.
    fn close_language_picker_and_restore_input(&mut self) -> Result<(), String> {
        if !self.language_picker_visible {
            return Ok(());
        }
        self.set_language_picker_visible(false)?;
        self.window.set_focus();
        self.input_view().focus().map_err(|error| error.to_string())
    }

    fn open_tab(&mut self, input: &str) -> Result<(), String> {
        if self.tabs.list().len() >= MAX_OPEN_TABS {
            return Err(format!(
                "Close a tab before opening another (limit: {MAX_OPEN_TABS})."
            ));
        }
        let url = normalize_navigation_input(input)?;
        let previous = self.tabs.active_id();
        let id = self.tabs.open_tab(&url);
        if let Err(error) = self.show_active_tab() {
            self.tabs.close_tab(id);
            self.tabs.switch_to_tab(previous);
            self.content.remove(&id);
            self.pending_navigations.remove(&id);
            self.show_active_tab()?;
            return Err(error);
        }
        self.input_target = InputTarget::Address;
        Ok(())
    }

    fn navigate(&mut self, input: &str) -> Result<(), String> {
        let url = normalize_navigation_input(input)?;
        let active = self.tabs.active_tab().ok_or("No active browser tab")?;
        if active.kind != TabKind::Web {
            return self.open_tab(&url);
        }
        let id = active.id;
        self.navigate_after_permission_reset(id, &url)?;
        self.tabs.update_url(id, &url);
        self.tabs.set_loading(id, true);
        self.sync_chrome();
        Ok(())
    }

    fn switch_desktop(&mut self) -> Result<(), String> {
        self.requests.cancel_all();
        for request in self.downloads.pending_requests() {
            self.downloads.resolve(request.id, false)?;
        }
        self.permission_ui.hide()?;
        self.download_ui.hide()?;
        self.set_language_picker_visible(false)?;
        self.set_keyboard_visible(false)?;
        if let Some(desktop) = &self.desktop {
            desktop
                .switch_to_default_desktop()
                .map_err(|error| error.to_string())
        } else {
            self.window.set_minimized(true);
            Ok(())
        }
    }

    /// Initializes bundled controls while the warning gates user actions until acknowledgement.
    fn handle_command(&mut self, surface: Surface, body: &str) -> Result<bool, String> {
        let message: Value =
            serde_json::from_str(body).map_err(|_| "Invalid browser control message")?;
        let command = message
            .get("type")
            .and_then(Value::as_str)
            .ok_or("Missing browser command")?;
        if !keyboard_command_allowed(surface, command) {
            return Err("This control cannot send that keyboard command.".into());
        }
        if command == "UI_READY" {
            self.sync_chrome();
            if surface == Surface::Keyboard {
                self.input_language = None;
                self.floating_keyboard.sync_controls(&self.keyboard)?;
            }
            self.refresh_input_language()?;
            self.sync_permission_settings()?;
            self.sync_request_prompt()?;
            return Ok(false);
        }
        if self.warning_pending {
            if surface == Surface::Warning && command == "ACKNOWLEDGE_CAPTURE_RISK" {
                self.warning_pending = false;
                if let Err(error) = self.show_active_tab() {
                    self.warning_pending = true;
                    self.layout()?;
                    if let Some(warning) = &self.warning {
                        let _ = warning.evaluate_script(&format!("document.getElementById('acknowledge').disabled=false; document.getElementById('warning-error')?.remove(); const p=document.createElement('p'); p.id='warning-error'; p.textContent={}; document.querySelector('.content').append(p);", json!(error)));
                    }
                    return Err(error);
                }
                return Ok(false);
            }
            return Ok(surface == Surface::Warning && command == "EXIT_APP");
        }
        if surface == Surface::Warning {
            return Ok(false);
        }
        match self.handle_permission_command(surface, command, &message) {
            Ok(true) => return Ok(false),
            Err(error) => {
                if surface == Surface::PermissionPrompt {
                    self.permission_ui.show_error(&error);
                }
                let _ = self.sync_permission_settings();
                return Err(error);
            }
            Ok(false) => {}
        }
        match self.handle_download_command(surface, command, &message) {
            Ok(true) => return Ok(false),
            Err(error) => {
                let _ = self.sync_permission_settings();
                return Err(error);
            }
            Ok(false) => {}
        }
        if self.language_picker_visible
            && command == "SET_INPUT_TARGET"
            && !shell_windows::owns_foreground_window(&self.window)
        {
            // Focus IPC can arrive after the picker has already taken native focus.
            return Ok(false);
        }
        if self.language_picker_visible
            && surface != Surface::LanguagePicker
            && !matches!(
                command,
                "TOGGLE_LANGUAGE_PICKER" | "QUERY_INPUT_LANGUAGE" | "QUERY_BATTERY" | "UI_READY"
            )
        {
            self.set_language_picker_visible(false)?;
        }
        let string = |key: &str| {
            message
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("Missing {key}"))
        };
        let tab_id = || {
            message
                .get("id")
                .and_then(Value::as_u64)
                .and_then(|id| usize::try_from(id).ok())
                .ok_or("Invalid tab ID".to_owned())
        };
        match command {
            "CLOSE_WINDOW" | "EXIT_APP" => return Ok(true),
            "SWITCH_DESKTOP" | "MINIMIZE" => self.switch_desktop()?,
            "FOCUS_BROWSER" => {
                self.window.set_minimized(false);
                self.window.set_focus();
            }
            "START_DRAG" => {
                if !self.window.is_maximized() && self.window.fullscreen().is_none() {
                    let _ = self.window.drag_window();
                }
            }
            "TOGGLE_MAXIMIZE" => {
                if self.window.fullscreen().is_some() {
                    self.window.set_fullscreen(None);
                } else {
                    self.window.set_maximized(!self.window.is_maximized());
                }
                self.sync_chrome();
            }
            "NAVIGATE" => self.navigate(string("url")?)?,
            "PRINT" => {
                let id = tab_id()?;
                validate_print_request(surface, id, &self.tabs)?;
                require_printing_enabled(self.permissions.printing_enabled())?;
                if !self.content.contains_key(&id) {
                    return Err(
                        "This page is not ready to print. Wait for it to open and try again."
                            .into(),
                    );
                }
                self.set_keyboard_visible(false)?;
                let view = self
                    .content
                    .get(&id)
                    .ok_or("The page was closed before printing.")?;
                view.focus().map_err(|error| error.to_string())?;
                show_system_print_dialog(view, self.permissions.printing_enabled())?;
            }
            "NEW_TAB" => {
                self.open_tab(DEFAULT_HOMEPAGE_URL)?;
                self.focus_address();
            }
            "NEW_TAB_WITH_URL" => self.open_tab(string("url")?)?,
            "SWITCH_TAB" => {
                if self.tabs.switch_to_tab(tab_id()?) {
                    self.input_target = InputTarget::Address;
                    self.show_active_tab()?;
                }
            }
            "CLOSE_TAB" => {
                let id = tab_id()?;
                if self.tabs.tab(id).is_none() {
                    return Ok(false);
                }
                if self.tabs.list().len() == 1 {
                    return Ok(true);
                }
                self.tabs.close_tab(id);
                self.content.remove(&id);
                self.pending_navigations.remove(&id);
                self.input_target = InputTarget::Address;
                self.show_active_tab()?;
            }
            "NEXT_TAB" | "PREVIOUS_TAB" => {
                let tabs = self.tabs.list();
                if let Some(current) = tabs.iter().position(|tab| tab.id == self.tabs.active_id()) {
                    let next = if command == "NEXT_TAB" {
                        (current + 1) % tabs.len()
                    } else {
                        (current + tabs.len() - 1) % tabs.len()
                    };
                    let id = tabs[next].id;
                    self.tabs.switch_to_tab(id);
                    self.input_target = InputTarget::Address;
                    self.show_active_tab()?;
                }
            }
            "BACK" | "GO_BACK" | "FORWARD" | "GO_FORWARD" | "RELOAD" => {
                if let Some(view) = self.content.get(&self.tabs.active_id()) {
                    unsafe {
                        let core = view
                            .controller()
                            .CoreWebView2()
                            .map_err(|error| error.to_string())?;
                        match command {
                            "BACK" | "GO_BACK" => core.GoBack(),
                            "FORWARD" | "GO_FORWARD" => core.GoForward(),
                            _ => core.Reload(),
                        }
                        .map_err(|error| error.to_string())?;
                    }
                }
            }
            "OPEN_BOOKMARKS" | "OPEN_SETTINGS" => {
                let (title, kind) = if command == "OPEN_BOOKMARKS" {
                    ("Bookmarks", TabKind::Bookmarks)
                } else {
                    ("Settings", TabKind::Settings)
                };
                if self.tabs.list().len() >= MAX_OPEN_TABS
                    && !self.tabs.list().iter().any(|tab| tab.kind == kind)
                {
                    return Err(format!(
                        "Close a tab before opening another (limit: {MAX_OPEN_TABS})."
                    ));
                }
                self.tabs.open_or_switch_special(title, kind);
                self.input_target = InputTarget::Address;
                self.show_active_tab()?;
            }
            "ADD_BOOKMARK" => {
                let active = self.tabs.active_tab().ok_or("No page to bookmark")?;
                if active.kind != TabKind::Web {
                    return Err("Open a website to bookmark it.".into());
                }
                self.bookmarks
                    .add(&active.title, &active.url, BookmarkCategory::General)?;
                self.last_internal_kind = None;
                self.notice("Bookmark saved.", false);
            }
            "ADD_BOOKMARK_FROM_DATA" => {
                let url = normalize_navigation_input(string("url")?)?;
                self.bookmarks
                    .add(string("title")?, url, BookmarkCategory::General)?;
                self.refresh_bookmarks()?;
                self.notice("Bookmark saved.", false);
            }
            "REMOVE_BOOKMARK" => {
                self.bookmarks.remove(string("id")?)?;
                self.refresh_bookmarks()?;
                self.notice("Bookmark removed.", false);
            }
            "SET_INPUT_TARGET" => self.input_target = InputTarget::Address,
            "FOCUS_ADDRESS" => self.focus_address(),
            "TOGGLE_OSK" => {
                self.set_keyboard_visible(!self.keyboard_visible)?;
            }
            "DETACH_OSK" | "ATTACH_OSK" => {
                self.floating_keyboard.set_detached(
                    command == "DETACH_OSK",
                    &self.keyboard,
                    &self.window,
                )?;
                self.layout()?;
            }
            "START_OSK_DRAG" => self.floating_keyboard.start_drag()?,
            "TOGGLE_LANGUAGE_PICKER" => {
                self.set_language_picker_visible(!self.language_picker_visible)?
            }
            "CLOSE_LANGUAGE_PICKER" => self.close_language_picker_and_restore_input()?,
            "QUERY_INPUT_LANGUAGE" if !self.language_picker_visible => {
                self.refresh_input_language()?
            }
            "SELECT_INPUT_LANGUAGE" => {
                match language::select(self.input_window()?, string("id")?) {
                    Ok(state) => {
                        self.update_input_language(state)?;
                        self.close_language_picker_and_restore_input()?;
                    }
                    Err(error) => {
                        self.refresh_input_language()?;
                        self.language_picker
                            .evaluate_script(&format!(
                                "window.showLanguageError?.({});",
                                json!(error)
                            ))
                            .map_err(|error| error.to_string())?;
                    }
                }
            }
            "KEY_INPUT" => {
                let action = string("action")?;
                match self.input_target {
                    InputTarget::Address => self
                        .chrome
                        .evaluate_script(&format!("window.injectOmniboxKey?.({});", json!(action))),
                    InputTarget::Content(id) if id == self.tabs.active_id() => {
                        if let Some(view) = self.content.get(&id) {
                            view.evaluate_script(&VirtualKeyboard::generate_dom_injection_script(
                                action,
                            ))
                        } else {
                            Ok(())
                        }
                    }
                    InputTarget::Internal => self
                        .internal
                        .evaluate_script(&VirtualKeyboard::generate_dom_injection_script(action)),
                    _ => Ok(()),
                }
                .map_err(|error| error.to_string())?;
            }
            "QUERY_BATTERY" => {
                let (_, percentage, _) = super::assets::get_system_battery_status();
                let _ = self
                    .taskbar
                    .evaluate_script(&format!("window.updateBatteryUI?.(null,{percentage});"));
            }
            _ => {}
        }
        Ok(false)
    }

    fn refresh_bookmarks(&mut self) -> Result<(), String> {
        let bookmarks = json!(self.bookmarks.list());
        self.internal
            .evaluate_script(&format!("window.updateBookmarks?.({bookmarks});"))
            .map_err(|error| error.to_string())
    }

    /// Closing or leaving the desktop hides either placement while retaining the keyboard state.
    fn set_keyboard_visible(&mut self, visible: bool) -> Result<(), String> {
        self.keyboard_visible = visible;
        self.layout()?;
        self.sync_chrome();
        Ok(())
    }

    fn focus_address(&mut self) {
        self.input_target = InputTarget::Address;
        let _ = self.chrome.focus();
        let _ = self.chrome.evaluate_script("window.focusOmnibox?.();");
    }

    fn handle_event(&mut self, event: KioskEvent) -> Result<bool, String> {
        match event {
            KioskEvent::EngineHealth { tab_id, event } => {
                if tab_id.is_some_and(|id| !self.content.contains_key(&id)) {
                    return Ok(false);
                }
                match event {
                    BrowserHealthEvent::UpdateAvailable => {
                        self.runtime_update_available = true;
                        self.sync_chrome();
                    }
                    BrowserHealthEvent::Failed(failure) => {
                        self.terminal_error.get_or_insert_with(|| format!(
                            "{} SafeBrowse ended the session without reloading pages or repeating submissions. Reopen SafeBrowse and check the website's transaction status before trying again.",
                            failure.message()
                        ));
                        self.requests.cancel_all();
                        self.downloads.cancel_all();
                        self.pending_navigations.clear();
                        return Ok(true);
                    }
                }
            }
            KioskEvent::Trusted(surface, body) => return self.handle_command(surface, &body),
            KioskEvent::BrowserRequest(event) => return self.handle_browser_request(event),
            KioskEvent::PermissionProfileReady(result) => {
                self.finish_permission_profile_reset(result)?
            }
            KioskEvent::Ready => {
                self.layout()?;
                self.sync_chrome();
                self.refresh_input_language()?;
                self.sync_permission_settings()?;
                self.sync_request_prompt()?;
            }
            KioskEvent::PageLoad { id, loading } => {
                if let Some(view) = self.content.get(&id) {
                    if let Ok(url) = view.url() {
                        self.tabs.update_url(id, &url);
                    }
                    self.tabs.set_loading(id, loading);
                    self.sync_chrome();
                }
            }
            KioskEvent::Title { id, title } => {
                self.tabs.update_title(id, &title);
                self.sync_chrome();
            }
            KioskEvent::SourceChanged(id) => {
                if let Some(view) = self.content.get(&id) {
                    if let Ok(url) = view.url() {
                        self.tabs.update_url(id, &url);
                    }
                    self.sync_chrome();
                }
            }
            KioskEvent::NavigationFailed(id) => {
                self.tabs.set_loading(id, false);
                self.tabs.update_title(id, "Page could not be loaded");
                self.sync_chrome();
                if id == self.tabs.active_id() {
                    self.notice("Page could not be loaded. Check the address and your connection, then reload.", true);
                }
            }
            KioskEvent::ContentFocused(id)
                if !self.warning_pending && id == self.tabs.active_id() =>
            {
                if self.language_picker_visible
                    && !shell_windows::owns_foreground_window(&self.window)
                {
                    return Ok(false);
                }
                self.input_target = InputTarget::Content(id);
                self.set_language_picker_visible(false)?;
                self.refresh_input_language()?;
            }
            KioskEvent::InternalFocused if !self.warning_pending => {
                if self.language_picker_visible
                    && !shell_windows::owns_foreground_window(&self.window)
                {
                    return Ok(false);
                }
                self.input_target = InputTarget::Internal;
                self.set_language_picker_visible(false)?;
                self.refresh_input_language()?;
            }
            KioskEvent::Shortcut { id, command }
                if !self.warning_pending && id == self.tabs.active_id() =>
            {
                return self.handle_command(
                    Surface::Chrome,
                    &json!({"type":command,"id":id}).to_string(),
                );
            }
            KioskEvent::Notice(message) => self.notice(message, true),
            KioskEvent::Download(event) => self.handle_download_event(event)?,
            KioskEvent::SwitchDesktop => self.switch_desktop()?,
            _ => {}
        }
        Ok(false)
    }
}

/// Runs a session and returns normally so WebView resources and temporary profile cleanup actually execute.
pub fn run_kiosk_session(
    is_fullscreen: bool,
    profile_mode: ProfileMode,
    initial_url: Option<String>,
    desktop_manager: Option<DesktopManager>,
    allow_screen_recording: bool,
) -> Result<(), String> {
    let url = normalize_navigation_input(initial_url.as_deref().unwrap_or(DEFAULT_HOMEPAGE_URL))?;
    let profile = ProfileManager::new(profile_mode)?;
    let mut builder = EventLoopBuilder::<KioskEvent>::with_user_event();
    builder.with_any_thread(true);
    let mut event_loop = builder.build();
    let mut session = BrowserSession::new(
        &event_loop,
        event_loop.create_proxy(),
        &profile,
        &url,
        desktop_manager,
        is_fullscreen,
        allow_screen_recording,
    )?;
    event_loop.run_return(|event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let result = match event {
            Event::MainEventsCleared if session.floating_keyboard.finish_drag_if_released() => {
                session.layout().map(|_| false)
            }
            Event::UserEvent(event) => session.handle_event(event),
            Event::WindowEvent {
                window_id, event, ..
            } if window_id == session.window.id() => match event {
                WindowEvent::CloseRequested => Ok(true),
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => session
                    .layout()
                    .and_then(|_| session.sync_request_prompt())
                    .map(|_| false),
                WindowEvent::Focused(true) => session.sync_request_prompt().map(|_| false),
                WindowEvent::Moved(_) => session
                    .notify_window_moved()
                    .and_then(|_| session.layout())
                    .and_then(|_| session.sync_request_prompt())
                    .map(|_| false),
                _ => Ok(false),
            },
            Event::WindowEvent {
                window_id, event, ..
            } if window_id == session.floating_keyboard.window_id() => match event {
                WindowEvent::CloseRequested => session.set_keyboard_visible(false).map(|_| false),
                WindowEvent::Moved(_) => session
                    .notify_window_moved()
                    .and_then(|_| session.layout())
                    .map(|_| false),
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    session.layout().map(|_| false)
                }
                _ => Ok(false),
            },
            Event::WindowEvent {
                window_id, event, ..
            } if window_id == session.permission_ui.window_id() => match event {
                WindowEvent::CloseRequested => session.dismiss_site_request().map(|_| false),
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    session.permission_ui.resize().map(|_| false)
                }
                _ => Ok(false),
            },
            Event::WindowEvent {
                window_id, event, ..
            } if window_id == session.download_ui.window_id() => match event {
                WindowEvent::CloseRequested => session.dismiss_download_request().map(|_| false),
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    session.download_ui.resize().map(|_| false)
                }
                _ => Ok(false),
            },
            Event::WindowEvent {
                window_id, event, ..
            } if window_id == session.language_window.id() => match event {
                WindowEvent::CloseRequested if session.language_picker_visible => session
                    .close_language_picker_and_restore_input()
                    .map(|_| false),
                WindowEvent::Focused(false) if session.language_picker_visible => {
                    let anchor = session.taskbar_window.as_ref().unwrap_or(&session.window);
                    if shell_windows::popup_focus_moved_elsewhere(&session.language_window, anchor)
                    {
                        session.set_language_picker_visible(false).map(|_| false)
                    } else {
                        Ok(false)
                    }
                }
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    session.layout().map(|_| false)
                }
                _ => Ok(false),
            },
            Event::WindowEvent {
                window_id,
                event: WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. },
                ..
            } if session
                .taskbar_window
                .as_ref()
                .is_some_and(|window| window.id() == window_id) =>
            {
                session.layout().map(|_| false)
            }
            _ => Ok(false),
        };
        match result {
            Ok(true) => *control_flow = ControlFlow::Exit,
            Err(error) => {
                eprintln!("[SafeBrowse] {error}");
                session.notice(&error, true);
            }
            _ => {}
        }
    });
    let terminal_error = session.terminal_error.take();
    let desktop_result = session.desktop.as_ref().map_or(Ok(()), |desktop| {
        desktop
            .switch_to_default_desktop()
            .map_err(|error| error.to_string())
    });
    drop(session);
    drop(event_loop);
    let cleanup_result = profile.purge_ephemeral_storage();
    let errors: Vec<String> = terminal_error
        .into_iter()
        .chain(desktop_result.err())
        .chain(cleanup_result.err())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tao::event_loop::EventLoop;
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings8;
    use webview2_core::{Interface, BOOL};

    include!("kiosk/printing_ui_tests.rs");
    include!("kiosk/settings_policy_ui_tests.rs");
    include!("kiosk/download_ui_tests.rs");
    include!("kiosk/browser_safety_tests.rs");

    /// Uses a deadline so a failed native document reports a test failure instead of hanging.
    fn wait_for_native_result<T>(
        event_loop: &mut EventLoop<KioskEvent>,
        mut observe: impl FnMut(Event<KioskEvent>) -> Option<T>,
    ) -> T {
        const LOAD_TIMEOUT: Duration = Duration::from_secs(15);
        let deadline = Instant::now() + LOAD_TIMEOUT;
        let mut result = None;
        event_loop.run_return(|event, _, control_flow| {
            *control_flow = ControlFlow::WaitUntil(deadline);
            if result.is_none() {
                result = observe(event);
            }
            if result.is_some() || Instant::now() >= deadline {
                *control_flow = ControlFlow::Exit;
            }
        });
        result.expect("native document did not produce the expected result before the deadline")
    }

    fn inspect_bundled_document(
        event_loop: &mut EventLoop<KioskEvent>,
        view: &TrustedWebView,
    ) -> Value {
        evaluate_bundled_document(
            event_loop,
            view,
            "({title:document.querySelector('h1')?.textContent,background:getComputedStyle(document.body).backgroundColor,button:document.getElementById('acknowledge')?.textContent,bridge:typeof window.ipc?.postMessage,width:innerWidth,height:innerHeight})",
        )
    }

    /// Reads the actual WebView2 DOM after running one bounded test action.
    fn evaluate_bundled_document(
        event_loop: &mut EventLoop<KioskEvent>,
        view: &WebView,
        script: &str,
    ) -> Value {
        let (sender, receiver) = std::sync::mpsc::channel();
        let proxy = event_loop.create_proxy();
        view.evaluate_script_with_callback(script, move |value| {
            let _ = sender.send(value);
            let _ = proxy.send_event(KioskEvent::Notice("Native test inspection finished"));
        })
        .unwrap();
        let result = wait_for_native_result(event_loop, |_| receiver.try_recv().ok());
        serde_json::from_str(&result).unwrap()
    }

    fn wait_for_bundled_load(event_loop: &mut EventLoop<KioskEvent>) {
        wait_for_native_result(event_loop, |event| {
            matches!(event, Event::UserEvent(KioskEvent::Ready)).then_some(())
        });
    }

    /// Matches both surface and command so another view's queued readiness cannot pass a check.
    fn wait_for_surface_command(
        event_loop: &mut EventLoop<KioskEvent>,
        surface: Surface,
        command_type: &str,
    ) -> Value {
        wait_for_native_result(event_loop, |event| match event {
            Event::UserEvent(KioskEvent::Trusted(source, body)) if source == surface => {
                let command: Value = serde_json::from_str(&body).unwrap();
                (command["type"] == command_type).then_some(command)
            }
            _ => None,
        })
    }

    fn build_test_surface(
        event_loop: &mut EventLoop<KioskEvent>,
        window: &Window,
        context: &mut WebContext,
        html: String,
        surface: Surface,
        size: LogicalSize<f64>,
    ) -> TrustedWebView {
        let view = build_trusted_view(window, context, html, surface, &event_loop.create_proxy())
            .expect("build bundled test surface");
        view.set_bounds(make_rect(0.0, 0.0, size.width, size.height))
            .unwrap();
        view.set_visible(true).unwrap();
        wait_for_surface_command(event_loop, surface, "UI_READY");
        view
    }

    /// Exercises the production website builder without navigating to a real website.
    fn assert_native_content_security_settings(
        event_loop: &EventLoop<KioskEvent>,
        window: &Window,
        context: &mut WebContext,
    ) {
        const CONTENT_FIXTURE_TAB_ID: usize = 1;
        let requests = RequestBroker::new();
        let downloads = DownloadBroker::new();
        let view = build_content_view(
            window,
            context,
            CONTENT_FIXTURE_TAB_ID,
            &event_loop.create_proxy(),
            &requests,
            &downloads,
            None,
        )
        .expect("build website view with restricted host access");
        let mut web_messages_enabled = Default::default();
        let mut host_objects_allowed = Default::default();
        let mut reputation_checking_required = BOOL::default();
        unsafe {
            let settings = view
                .controller()
                .CoreWebView2()
                .unwrap()
                .Settings()
                .unwrap();
            settings
                .IsWebMessageEnabled(&mut web_messages_enabled)
                .unwrap();
            settings
                .AreHostObjectsAllowed(&mut host_objects_allowed)
                .unwrap();
            settings
                .cast::<ICoreWebView2Settings8>()
                .unwrap()
                .IsReputationCheckingRequired(&mut reputation_checking_required)
                .unwrap();
        }
        assert!(
            !web_messages_enabled.as_bool(),
            "website messaging transport must be disabled"
        );
        assert!(
            !host_objects_allowed.as_bool(),
            "website native host objects must be disabled"
        );
        assert!(
            reputation_checking_required.as_bool(),
            "website reputation checking must be required"
        );
        assert_eq!(
            show_system_print_dialog(&view, false).unwrap_err(),
            "SafeBrowse print controls are disabled. You can enable them in Settings."
        );
    }

    /// Installed language names and selection stay in the trusted picker IPC channel.
    fn assert_native_language_picker(
        event_loop: &mut EventLoop<KioskEvent>,
        window: &Window,
        context: &mut WebContext,
    ) {
        let picker_window = shell_windows::create_shell_window(
            event_loop,
            window,
            "SafeBrowse language-picker focus test",
            LogicalSize::new(340.0, 340.0),
            true,
        )
        .unwrap();
        let view = build_test_surface(
            event_loop,
            &picker_window,
            context,
            generate_language_picker_html(),
            Surface::LanguagePicker,
            LogicalSize::new(340.0, 340.0),
        );
        let state = language::snapshot(HWND(window.hwnd() as *mut _))
            .expect("read installed layouts without changing Windows input language");
        assert!(!state.layouts.is_empty());
        let selected = state.layouts.last().unwrap();
        let document = evaluate_bundled_document(event_loop, &view, &format!(
            "window.updateLanguages({}); ({{labels:Array.from(document.querySelectorAll('.language .name'), node => node.textContent), selected:Array.from(document.querySelectorAll('.language'), node => node.getAttribute('aria-checked'))}})",
            json!(state)
        ));
        assert_eq!(
            document["labels"],
            json!(state
                .layouts
                .iter()
                .map(|layout| &layout.label)
                .collect::<Vec<_>>())
        );
        assert_eq!(
            document["selected"],
            json!(state
                .layouts
                .iter()
                .map(|layout| (layout.id == state.active_id).to_string())
                .collect::<Vec<_>>())
        );
        view.evaluate_script("document.querySelectorAll('.language')[document.querySelectorAll('.language').length - 1].click()")
            .unwrap();
        let command =
            wait_for_surface_command(event_loop, Surface::LanguagePicker, "SELECT_INPUT_LANGUAGE");
        assert_eq!(command["id"], selected.id);
        view.evaluate_script(
            "document.querySelector('[data-command=\"CLOSE_LANGUAGE_PICKER\"]').click()",
        )
        .unwrap();
        wait_for_surface_command(event_loop, Surface::LanguagePicker, "CLOSE_LANGUAGE_PICKER");
        assert_native_picker_reopens_after_child_focus(event_loop, window, &picker_window, &view);
    }

    /// Real top-level-to-WebView focus transfers must not be treated as popup dismissal.
    fn assert_native_picker_reopens_after_child_focus(
        event_loop: &mut EventLoop<KioskEvent>,
        anchor: &Window,
        picker: &Window,
        view: &TrustedWebView,
    ) {
        const FOCUS_SETTLE_INTERVAL: Duration = Duration::from_millis(150);
        let mut child_focus_transfers = 0;
        for _ in 0..3 {
            shell_windows::show_focused_popup(picker);
            view.focus().unwrap();
            view.evaluate_script("window.focusSelectedLanguage?.();")
                .unwrap();
            let deadline = Instant::now() + FOCUS_SETTLE_INTERVAL;
            let mut dismissed = false;
            event_loop.run_return(|event, _, control_flow| {
                *control_flow = ControlFlow::WaitUntil(deadline);
                if matches!(event, Event::WindowEvent { window_id, event: WindowEvent::Focused(false), .. } if window_id == picker.id()) {
                    child_focus_transfers += 1;
                    if shell_windows::popup_focus_moved_elsewhere(picker, anchor) {
                        dismissed = true;
                        picker.set_visible(false);
                    }
                }
                if Instant::now() >= deadline {
                    *control_flow = ControlFlow::Exit;
                }
            });
            assert!(
                !dismissed,
                "a queued focus event dismissed the newly opened picker"
            );
            assert!(picker.is_visible());
            assert!(shell_windows::owns_foreground_window(picker));
            assert!(!shell_windows::popup_focus_moved_elsewhere(picker, anchor));
            let focused = evaluate_bundled_document(event_loop, view,
                "({documentFocused:document.hasFocus(),selected:document.activeElement.getAttribute('aria-checked')})");
            assert_eq!(focused["documentFocused"], true);
            assert_eq!(focused["selected"], "true");
            // Leave hide notifications queued to exercise stale blur events on the next open.
            picker.set_visible(false);
        }
        assert!(
            child_focus_transfers > 0,
            "the native child focus regression was not exercised"
        );
    }

    /// Saved decisions and prompt responses cross the trusted bridge with exact origin and scope.
    fn assert_native_permission_controls(
        event_loop: &mut EventLoop<KioskEvent>,
        window: &Window,
        context: &mut WebContext,
    ) {
        let view = build_test_surface(
            event_loop,
            window,
            context,
            generate_settings_page_html_with_session(true, false, true),
            Surface::Internal,
            LogicalSize::new(800.0, 600.0),
        );
        let snapshot = json!({"version":1,"popup_default":"ask","site_rules":[
            {"origin":"https://example.com:8443","permission":"camera","decision":"allow"}
        ]});
        let settings = evaluate_bundled_document(event_loop, &view, &format!(
            "window.updatePermissions({snapshot}); ({{popup:document.getElementById('popup-policy').value,origin:document.querySelector('.site-origin').textContent,decision:document.querySelector('.site-rule select').value}})"));
        assert_eq!(settings["popup"], "ask");
        assert_eq!(settings["origin"], "https://example.com:8443");
        assert_eq!(settings["decision"], "allow");
        view.evaluate_script("document.getElementById('popup-policy').value='allow';document.getElementById('popup-policy').dispatchEvent(new Event('change'));").unwrap();
        assert_eq!(
            wait_for_surface_command(event_loop, Surface::Internal, "SET_POPUP_POLICY")["decision"],
            "allow"
        );
        view.evaluate_script("document.querySelector('.site-rule select').value='block';document.querySelector('.site-rule select').dispatchEvent(new Event('change'));").unwrap();
        let blocked =
            wait_for_surface_command(event_loop, Surface::Internal, "SET_SITE_PERMISSION");
        assert_eq!(blocked["origin"], "https://example.com:8443");
        assert_eq!(blocked["permission"], "camera");
        assert_eq!(blocked["decision"], "block");
        view.evaluate_script("document.querySelector('.site-rule button').click()")
            .unwrap();
        assert_eq!(
            wait_for_surface_command(event_loop, Surface::Internal, "RESET_SITE_PERMISSION")
                ["origin"],
            "https://example.com:8443"
        );
        let reset_state = evaluate_bundled_document(event_loop, &view,
            "window.updatePermissions({popup_default:'ask',site_rules:[]}); ({empty:document.querySelectorAll('.site-rule').length,reloadAvailable:!document.getElementById('reload-notice').hidden})");
        assert_eq!(reset_state["empty"], 0);
        assert_eq!(reset_state["reloadAvailable"], true);
        view.evaluate_script("document.getElementById('reload-changed-site').click()")
            .unwrap();
        assert_eq!(
            wait_for_surface_command(event_loop, Surface::Internal, "RELOAD_SITE_TABS")["origin"],
            "https://example.com:8443"
        );
        drop(view);
        let prompt = build_test_surface(
            event_loop,
            window,
            context,
            super::super::assets::generate_permission_prompt_html(),
            Surface::PermissionPrompt,
            LogicalSize::new(460.0, 350.0),
        );
        let request = json!({"id":42,"permission":"popups","origin":"https://example.com:8443","target_url":"https://login.example.com/authorize?value=<script>"});
        let appearance = evaluate_bundled_document(event_loop, &prompt, &format!(
            "window.showRequest({request}); ({{origin:document.getElementById('origin').textContent,target:document.getElementById('target').textContent,buttons:Array.from(document.querySelectorAll('.actions button'),button=>button.textContent)}})"));
        assert_eq!(appearance["origin"], "https://example.com:8443");
        assert!(appearance["target"].as_str().unwrap().ends_with("<script>"));
        assert_eq!(
            appearance["buttons"],
            json!(["Block", "Allow once", "Always allow"])
        );
        let next_request = evaluate_bundled_document(event_loop, &prompt,
            "document.getElementById('always').focus(); window.showRequest({id:43,permission:'camera',origin:'https://'+'x'.repeat(250)+'.example',target_url:'https://example.com/?'+ 'x'.repeat(8000)}); ({focused:document.activeElement.id,buttonsVisible:Array.from(document.querySelectorAll('button'), button=>button.getBoundingClientRect()).every(rect=>rect.top>=0&&rect.bottom<=innerHeight),scrollable:document.querySelector('.request-details').scrollHeight>document.querySelector('.request-details').clientHeight})");
        assert_eq!(
            next_request["focused"], "later",
            "a new request must not inherit an approval button's focus"
        );
        assert_eq!(
            next_request["buttonsVisible"], true,
            "long website addresses must not push decisions offscreen"
        );
        assert_eq!(next_request["scrollable"], true);
        for (button, decision, remember) in [
            ("once", "allow", false),
            ("always", "allow", true),
            ("block", "block", true),
            ("later", "block", false),
        ] {
            prompt
                .evaluate_script(&format!(
                    "window.showRequest({request});document.getElementById('{button}').click()"
                ))
                .unwrap();
            let response = wait_for_surface_command(
                event_loop,
                Surface::PermissionPrompt,
                "RESOLVE_SITE_REQUEST",
            );
            assert_eq!(response["id"], 42);
            assert_eq!(response["decision"], decision);
            assert_eq!(response["remember"], remember);
        }
    }

    /// The taskbar language indicator is an actionable accessible button, not a display-only label.
    fn assert_native_taskbar_language_button(
        event_loop: &mut EventLoop<KioskEvent>,
        window: &Window,
        context: &mut WebContext,
    ) {
        let view = build_test_surface(
            event_loop,
            window,
            context,
            generate_desktop_shell_html_with_session(true, true),
            Surface::Taskbar,
            LogicalSize::new(800.0, 46.0),
        );
        let document = evaluate_bundled_document(event_loop, &view,
            "window.updateLanguageUI('ENG');window.setLanguagePickerOpen(true);({tag:document.getElementById('language').tagName,label:document.getElementById('language').textContent,expanded:document.getElementById('language').getAttribute('aria-expanded'),desktop:document.getElementById('desktop-label').textContent})");
        assert_eq!(document["tag"], "BUTTON");
        assert_eq!(document["label"], "ENG");
        assert_eq!(document["expanded"], "true");
        assert_eq!(document["desktop"], "Back to desktop");
        view.evaluate_script("document.getElementById('language').click()")
            .unwrap();
        wait_for_surface_command(event_loop, Surface::Taskbar, "TOGGLE_LANGUAGE_PICKER");
        let updated = evaluate_bundled_document(event_loop, &view,
            "window.updateLanguageUI('FRA');window.setLanguagePickerOpen(false);({label:document.getElementById('language').textContent,expanded:document.getElementById('language').getAttribute('aria-expanded')})");
        assert_eq!(updated["label"], "FRA");
        assert_eq!(updated["expanded"], "false");
    }

    /// Synthetic non-US rows exercise localized output without activating an OS input layout.
    fn assert_native_localized_keyboard(
        event_loop: &mut EventLoop<KioskEvent>,
        window: &Window,
        context: &mut WebContext,
    ) {
        let view = build_test_surface(
            event_loop,
            window,
            context,
            generate_virtual_keyboard_html(),
            Surface::Keyboard,
            LogicalSize::new(800.0, 230.0),
        );
        let rows = json!([
            [{"value":"²", "shifted_value":"~"}],
            [{"value":"é", "shifted_value":"2", "caps_value":"É", "shifted_caps_value":"2"}],
            [{"value":"ж", "shifted_value":"Ж"}],
            [{"value":",", "shifted_value":";"}]
        ]);
        let baseline = evaluate_bundled_document(event_loop, &view, &format!(
            "window.setKeyboardLayout({rows}, 'Localized test layout', false);Array.from(document.querySelectorAll('.key'), button => button.textContent)"
        ));
        assert_eq!(
            baseline,
            json!([
                "²",
                "Backspace",
                "é",
                "Caps lock",
                "ж",
                "Enter",
                "Shift",
                ",",
                "Shift",
                "Space"
            ])
        );
        view.evaluate_script("document.querySelector('[data-position=\"1:0\"]').click()")
            .unwrap();
        assert_eq!(
            wait_for_surface_command(event_loop, Surface::Keyboard, "KEY_INPUT")["action"],
            "é"
        );
        let shifted = evaluate_bundled_document(event_loop, &view,
            "document.querySelector('[data-position=\"3:0\"]').click();Array.from(document.querySelectorAll('.key'), button => button.textContent)");
        assert_eq!(shifted[0], "~");
        assert_eq!(shifted[2], "2");
        assert_eq!(shifted[4], "Ж");
        assert_eq!(shifted[7], ";");
        view.evaluate_script("document.querySelector('[data-position=\"0:0\"]').click()")
            .unwrap();
        assert_eq!(
            wait_for_surface_command(event_loop, Surface::Keyboard, "KEY_INPUT")["action"],
            "~"
        );
        let released = evaluate_bundled_document(
            event_loop,
            &view,
            "Array.from(document.querySelectorAll('.key'), button => button.textContent)",
        );
        assert_eq!(
            released, baseline,
            "Shift must reset after one printable virtual key"
        );
        let caps = evaluate_bundled_document(event_loop, &view,
            "document.querySelector('[data-position=\"2:0\"]').click();Array.from(document.querySelectorAll('.key'), button => button.textContent)");
        assert_eq!(caps[0], "²");
        assert_eq!(caps[2], "É");
        assert_eq!(caps[4], "Ж");
        assert_eq!(caps[7], ",");
        view.evaluate_script("document.querySelector('[data-position=\"1:0\"]').click()")
            .unwrap();
        assert_eq!(
            wait_for_surface_command(event_loop, Surface::Keyboard, "KEY_INPUT")["action"],
            "É"
        );
        let shifted_caps = evaluate_bundled_document(event_loop, &view,
            "document.querySelector('[data-position=\"3:0\"]').click();document.querySelector('[data-position=\"1:0\"]').textContent");
        assert_eq!(
            shifted_caps, "2",
            "Caps+Shift must use the installed layout's explicit map"
        );
        view.evaluate_script("document.querySelector('[data-position=\"1:0\"]').click()")
            .unwrap();
        assert_eq!(
            wait_for_surface_command(event_loop, Surface::Keyboard, "KEY_INPUT")["action"],
            "2"
        );
        let reset = evaluate_bundled_document(event_loop, &view,
            "document.getElementById('shuffle').click();document.getElementById('reset').click();({keys:Array.from(document.querySelectorAll('.key'), button => button.textContent),label:document.getElementById('layout-label').textContent,shuffled:document.getElementById('shuffle').getAttribute('aria-pressed')})");
        assert_eq!(
            reset["keys"], baseline,
            "Reset must retain the selected non-US layout"
        );
        assert_eq!(reset["label"], "Localized test layout");
        assert_eq!(reset["shuffled"], "false");
        let ime = evaluate_bundled_document(event_loop, &view, &format!(
            "window.setKeyboardLayout({rows}, 'IME test layout', true);document.getElementById('layout-label').textContent"
        ));
        assert_eq!(ime, "IME test layout · IME: use physical keyboard");
        assert_native_keyboard_attachment(event_loop, window, &view);
    }

    /// Reparenting must preserve the live document, its selection state, and its native controller.
    fn assert_native_keyboard_attachment(
        event_loop: &mut EventLoop<KioskEvent>,
        browser: &Window,
        view: &TrustedWebView,
    ) {
        use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};
        let mut floating = FloatingKeyboard::new(event_loop, browser, false).unwrap();
        let controller = view.controller();
        let original_core = unsafe { controller.CoreWebView2() }.unwrap();
        let state_script = "({keys:Array.from(document.querySelectorAll('.key'), button => button.textContent),shuffled:document.getElementById('shuffle').getAttribute('aria-pressed'),label:document.getElementById('layout-label').textContent})";
        let state_before = evaluate_bundled_document(event_loop, view, &format!(
            "document.getElementById('shuffle').click();document.querySelector('[data-position=\"2:0\"]').click();{state_script}"
        ));
        view.evaluate_script("document.getElementById('placement').click()")
            .unwrap();
        wait_for_surface_command(event_loop, Surface::Keyboard, "DETACH_OSK");
        floating.set_detached(true, view, browser).unwrap();
        floating
            .layout(
                view,
                browser,
                make_rect(0.0, 0.0, 800.0, BROWSER_OSK_HEIGHT),
                false,
            )
            .unwrap();
        assert!(floating.is_detached());
        let state_after = evaluate_bundled_document(event_loop, view, state_script);
        assert_eq!(
            state_before, state_after,
            "detach must preserve shuffle, Caps and installed layout"
        );
        let controls = evaluate_bundled_document(event_loop, view,
            "({label:document.getElementById('placement').textContent,detached:document.body.classList.contains('detached')})");
        assert_eq!(controls["label"], "Attach");
        assert_eq!(controls["detached"], true);
        let mut child_host = Default::default();
        unsafe { controller.ParentWindow(&mut child_host) }.unwrap();
        assert_ne!(
            unsafe { GetAncestor(HWND(child_host.0), GA_ROOT) },
            HWND(browser.hwnd() as *mut _),
            "the existing native child must actually leave the browser window"
        );
        view.evaluate_script("document.getElementById('keyboard-header').dispatchEvent(new PointerEvent('pointerdown',{button:0,bubbles:true}))")
            .unwrap();
        wait_for_surface_command(event_loop, Surface::Keyboard, "START_OSK_DRAG");
        view.evaluate_script("document.querySelector('[data-position=\"1:0\"]').click()")
            .unwrap();
        assert!(
            !wait_for_surface_command(event_loop, Surface::Keyboard, "KEY_INPUT")["action"]
                .as_str()
                .unwrap()
                .is_empty()
        );
        view.evaluate_script("document.getElementById('placement').click()")
            .unwrap();
        wait_for_surface_command(event_loop, Surface::Keyboard, "ATTACH_OSK");
        floating.set_detached(false, view, browser).unwrap();
        floating
            .layout(
                view,
                browser,
                make_rect(0.0, 0.0, 800.0, BROWSER_OSK_HEIGHT),
                false,
            )
            .unwrap();
        assert!(!floating.is_detached());
        assert_eq!(unsafe { controller.CoreWebView2() }.unwrap(), original_core);
        assert_eq!(
            evaluate_bundled_document(event_loop, view, state_script),
            state_before
        );
        assert_eq!(
            unsafe { GetAncestor(HWND(child_host.0), GA_ROOT) },
            HWND(browser.hwnd() as *mut _)
        );
        assert_eq!(
            evaluate_bundled_document(
                event_loop,
                view,
                "document.getElementById('placement').textContent"
            ),
            "Detach"
        );
        assert!(
            !browser.is_visible(),
            "the native test must never reveal the fixture"
        );
    }

    #[test]
    fn only_keyboard_surface_accepts_placement_drag_and_input_commands() {
        for command in ["DETACH_OSK", "ATTACH_OSK", "START_OSK_DRAG", "KEY_INPUT"] {
            assert!(keyboard_command_allowed(Surface::Keyboard, command));
            for surface in [
                Surface::Chrome,
                Surface::Taskbar,
                Surface::Internal,
                Surface::Warning,
                Surface::LanguagePicker,
                Surface::PermissionPrompt,
            ] {
                assert!(!keyboard_command_allowed(surface, command));
            }
        }
        assert!(keyboard_command_allowed(Surface::Chrome, "TOGGLE_OSK"));
        assert!(keyboard_command_allowed(Surface::Keyboard, "TOGGLE_OSK"));
        assert!(keyboard_command_allowed(Surface::Keyboard, "UI_READY"));
        assert!(!keyboard_command_allowed(Surface::Keyboard, "NAVIGATE"));
        assert!(!keyboard_command_allowed(Surface::Keyboard, "START_DRAG"));
    }

    #[test]
    fn download_prompt_cannot_issue_browser_commands() {
        for command in [
            "PRINT",
            "NAVIGATE",
            "KEY_INPUT",
            "SET_DOWNLOAD_POLICY",
            "SET_PRINTING_ENABLED",
            "RESOLVE_SITE_REQUEST",
            "EXIT_APP",
        ] {
            assert!(!keyboard_command_allowed(Surface::DownloadPrompt, command));
        }
        assert!(keyboard_command_allowed(
            Surface::DownloadPrompt,
            "RESOLVE_DOWNLOAD"
        ));
        assert!(keyboard_command_allowed(
            Surface::DownloadPrompt,
            "UI_READY"
        ));
    }

    #[test]
    fn bundled_shell_surfaces_load_and_deliver_commands_inside_native_webviews() {
        let _native_test = crate::ui::NATIVE_WEBVIEW_TEST_LOCK.lock().unwrap();
        let mut event_loop = EventLoopBuilder::<KioskEvent>::with_user_event()
            .with_any_thread(true)
            .build();
        let window = WindowBuilder::new()
            .with_visible(false)
            .with_inner_size(LogicalSize::new(800.0, 600.0))
            .build(&event_loop)
            .unwrap();
        let profile = ProfileManager::new(ProfileMode::Ephemeral).unwrap();
        let mut context = WebContext::new(Some(profile.data_directory().join("warning-test")));
        assert_native_content_security_settings(&event_loop, &window, &mut context);
        let view = build_trusted_view(
            &window,
            &mut context,
            generate_capture_warning_html(),
            Surface::Warning,
            &event_loop.create_proxy(),
        )
        .unwrap();
        let mut trusted_messages_enabled = Default::default();
        unsafe {
            view.controller()
                .CoreWebView2()
                .unwrap()
                .Settings()
                .unwrap()
                .IsWebMessageEnabled(&mut trusted_messages_enabled)
                .unwrap();
        }
        assert!(
            trusted_messages_enabled.as_bool(),
            "trusted controls must retain their messaging transport"
        );
        view.set_bounds(make_rect(0.0, 0.0, 800.0, 600.0)).unwrap();
        view.set_visible(true).unwrap();
        wait_for_bundled_load(&mut event_loop);
        let snapshot = inspect_bundled_document(&mut event_loop, &view);
        assert_eq!(snapshot["title"], "Screen recording is allowed");
        assert_eq!(snapshot["button"], "OK");
        assert_eq!(snapshot["background"], "rgb(244, 244, 244)");
        assert_eq!(snapshot["bridge"], "function");
        assert_eq!(snapshot["width"], 800);
        assert_eq!(snapshot["height"], 600);
        view.evaluate_script("document.getElementById('acknowledge').click()")
            .unwrap();
        wait_for_native_result(&mut event_loop, |event| match event {
            Event::UserEvent(KioskEvent::Trusted(Surface::Warning, message)) => {
                let command: Value = serde_json::from_str(&message).unwrap();
                (command["type"] == "ACKNOWLEDGE_CAPTURE_RISK").then_some(())
            }
            _ => None,
        });
        // The same policy must authorize subsequent native HTML, such as opening Settings.
        view.load_html(&generate_settings_page_html_with_session(true, false, true))
            .unwrap();
        wait_for_bundled_load(&mut event_loop);
        let replacement = inspect_bundled_document(&mut event_loop, &view);
        assert_eq!(replacement["title"], "Settings");
        assert_eq!(replacement["bridge"], "function");
        drop(view);
        assert_native_language_picker(&mut event_loop, &window, &mut context);
        assert_native_taskbar_language_button(&mut event_loop, &window, &mut context);
        assert_native_localized_keyboard(&mut event_loop, &window, &mut context);
        assert_native_print_controls(&mut event_loop, &window, &mut context);
        assert_native_download_and_printing_settings(&mut event_loop, &window, &mut context);
        assert_native_download_confirmation(&mut event_loop, &window, &mut context);
        assert_native_permission_controls(&mut event_loop, &window, &mut context);
        drop(context);
        drop(window);
        drop(event_loop);
        profile.purge_ephemeral_storage().unwrap();
    }

    #[test]
    fn native_shortcuts_do_not_intercept_normal_typing_or_altgr() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_L, VK_P, VK_TAB};
        assert_eq!(
            browser_shortcut(u32::from(VK_L.0), true, false, false),
            Some("FOCUS_ADDRESS")
        );
        assert_eq!(
            browser_shortcut(u32::from(VK_TAB.0), true, false, true),
            Some("PREVIOUS_TAB")
        );
        assert_eq!(
            browser_shortcut(u32::from(VK_L.0), false, false, false),
            None
        );
        assert_eq!(browser_shortcut(u32::from(VK_L.0), true, true, false), None);
        assert_eq!(browser_shortcut(u32::MAX, true, false, false), None);
        assert_eq!(
            browser_shortcut(u32::from(VK_P.0), true, false, false),
            Some("PRINT")
        );
        assert_eq!(browser_shortcut(u32::from(VK_P.0), true, true, false), None);
        assert_eq!(browser_shortcut(u32::from(VK_P.0), true, false, true), None);
        assert_eq!(
            browser_shortcut(u32::from(VK_P.0), false, false, false),
            None
        );
    }

    #[test]
    fn print_requests_reject_other_controls_and_stale_or_internal_tabs() {
        let mut tabs = TabManager::new("https://example.test/receipt");
        let receipt_id = tabs.active_id();
        assert!(validate_print_request(Surface::Chrome, receipt_id, &tabs).is_ok());
        for surface in [
            Surface::Taskbar,
            Surface::Keyboard,
            Surface::Internal,
            Surface::Warning,
            Surface::LanguagePicker,
            Surface::PermissionPrompt,
        ] {
            assert!(validate_print_request(surface, receipt_id, &tabs).is_err());
        }
        let other_id = tabs.open_tab("https://example.test/other");
        assert!(validate_print_request(Surface::Chrome, receipt_id, &tabs).is_err());
        assert!(validate_print_request(Surface::Chrome, other_id, &tabs).is_ok());
        tabs.close_tab(other_id);
        assert!(validate_print_request(Surface::Chrome, other_id, &tabs).is_err());
        for kind in [TabKind::Settings, TabKind::Bookmarks] {
            let internal_id = tabs.open_or_switch_special("Internal", kind);
            assert!(validate_print_request(Surface::Chrome, internal_id, &tabs).is_err());
        }
    }

    #[test]
    fn minimized_bounds_do_not_create_negative_webview_sizes() {
        let bounds = make_rect(0.0, 110.0, 0.0, -156.0);
        assert_eq!(
            bounds.size,
            wry::dpi::Size::Logical(LogicalSize::new(1.0, 1.0))
        );
    }
}
