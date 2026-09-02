//! Browser Engine Controller
//!
//! Encapsulates WebView2 runtime instantiation, anti-automation flags,
//! navigation controls, script evaluation, and IPC message routing.

use std::sync::{Arc, Mutex};
use tao::window::Window;
use wry::{WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

use crate::browser::profile::{ProfileManager, ProfileMode};
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
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn new(mode: ProfileMode, initial_url: impl Into<String>) -> Result<Self, String> {
        let initial_url_str = initial_url.into();
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
    ///
    /// # Complexity
    /// - Time: O(1) (Spawns Edge WebView2 helper processes asynchronously)
    /// - Space: O(1)
    pub fn create_webview<F>(
        &self,
        window: &Window,
        ipc_dispatcher: F,
    ) -> Result<WebView, String>
    where
        F: Fn(String) + 'static,
    {
        let data_dir = self.profile_manager.data_directory().to_path_buf();
        let is_ephemeral = self.profile_manager.mode() == ProfileMode::Ephemeral;

        let mut web_context = WebContext::new(Some(data_dir));

        let security_args = CHROMIUM_ARGS_SECURITY.join(" ");

        let active_url = {
            let tabs = self.tab_manager.lock().unwrap();
            tabs.active_tab()
                .map(|t| t.url.clone())
                .unwrap_or_else(|| DEFAULT_HOMEPAGE_URL.to_string())
        };

        // Why: Configure the webview with full isolation, anti-automation suppression,
        // and direct IPC routing for the trusted UI and virtual keyboard.
        let builder = WebViewBuilder::new_with_web_context(&mut web_context)
            .with_incognito(is_ephemeral)
            .with_url(&active_url)
            .with_devtools(false)
            .with_browser_accelerator_keys(true)
            .with_additional_browser_args(security_args)
            .with_ipc_handler(move |req| {
                let payload = req.body().clone();
                ipc_dispatcher(payload);
            });

        builder
            .build(window)
            .map_err(|e| format!("Failed to initialize WebView2: {}", e))
    }
}
