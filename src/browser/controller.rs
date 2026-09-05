//! Browser Engine Controller
//!
//! Creates web content without granting remote pages native command access.

use std::sync::{Arc, Mutex};
use tao::window::Window;
use wry::{WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

use crate::browser::navigation::{normalize_navigation_input, validate_web_url};
use crate::browser::printing::WEBSITE_PRINT_GUARD;
use crate::browser::profile::{ProfileManager, ProfileMode};
use crate::browser::security::harden_content_view;
use crate::browser::tabs::TabManager;
use crate::config::{CHROMIUM_ARGS_SECURITY, DEFAULT_HOMEPAGE_URL};

/// Commands sent from the trusted UI shell or virtual keyboard over IPC.
#[derive(Debug, Clone)]
pub enum BrowserUiEvent {
    Navigate(String),
    GoBack,
    GoForward,
    Reload,
    NewTab(String),
    CloseTab(usize),
    SwitchTab(usize),
    ToggleVirtualKeyboard,
    SwitchToDesktop,
    AddBookmark { title: String, url: String },
    DeleteBookmark(String),
    DirectKeyInput(String),
    ExitSession,
}

/// Orchestrates the embedded Chromium webview and its event loop communication.
pub struct BrowserController {
    profile_manager: ProfileManager,
    tab_manager: Arc<Mutex<TabManager>>,
}

impl BrowserController {
    /// Initializes a new BrowserController with the specified profile mode.
    ///
    /// Time: O(N). Space: O(N), where N is the initial address length.
    pub fn new(mode: ProfileMode, initial_url: impl Into<String>) -> Result<Self, String> {
        let initial_url_str = normalize_navigation_input(&initial_url.into())?;
        let profile_manager = ProfileManager::new(mode)?;
        let tab_manager = Arc::new(Mutex::new(TabManager::new(initial_url_str)));

        Ok(Self {
            profile_manager,
            tab_manager,
        })
    }

    /// Returns a reference to the profile manager.
    #[inline]
    pub fn profile_manager(&self) -> &ProfileManager {
        &self.profile_manager
    }

    /// Returns the tab manager shared reference.
    #[inline]
    pub fn tab_manager(&self) -> Arc<Mutex<TabManager>> {
        Arc::clone(&self.tab_manager)
    }

    /// Constructs the Chromium WebView bound to the parent Tao window.
    /// Keep this controller alive until its returned views have been dropped so
    /// the profile can be cleaned up after WebView2 releases the storage handles.
    ///
    /// # Complexity
    /// - Time: O(1) (Spawns Edge WebView2 helper processes asynchronously)
    /// - Space: O(1)
    pub fn create_webview<F>(&self, window: &Window, _ipc_dispatcher: F) -> Result<WebView, String>
    where
        F: Fn(String) + 'static,
    {
        let data_dir = self.profile_manager.data_directory().to_path_buf();
        let is_ephemeral = self.profile_manager.mode() == ProfileMode::Ephemeral;

        let mut web_context = WebContext::new(Some(data_dir));

        let security_args = CHROMIUM_ARGS_SECURITY.join(" ");

        let active_url = {
            let tabs = self
                .tab_manager
                .lock()
                .map_err(|_| "The tab manager is unavailable".to_string())?;
            tabs.active_tab()
                .map(|t| t.url.clone())
                .unwrap_or_else(|| DEFAULT_HOMEPAGE_URL.to_string())
        };

        // Remote documents must never share the privileged shell IPC bridge.
        // The callback remains in this API for compatibility with existing callers.
        let builder = WebViewBuilder::new_with_web_context(&mut web_context)
            .with_incognito(is_ephemeral)
            .with_devtools(false)
            .with_browser_accelerator_keys(false)
            .with_default_context_menus(false)
            .with_general_autofill_enabled(false)
            .with_clipboard(false)
            .with_additional_browser_args(security_args)
            .with_initialization_script_for_main_only(WEBSITE_PRINT_GUARD, false)
            .with_navigation_handler(|url| validate_web_url(&url).is_ok())
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_permission_handler(|_| wry::PermissionResponse::Deny);

        let view = builder
            .build(window)
            .map_err(|error| format!("Failed to initialize WebView2: {error}"))?;
        crate::browser::runtime::validate_created_environment(
            &view,
            self.profile_manager.data_directory(),
        )?;
        harden_content_view(&view)?;
        view.load_url(&active_url)
            .map_err(|error| format!("Failed to navigate the browser: {error}"))?;
        Ok(view)
    }
}
