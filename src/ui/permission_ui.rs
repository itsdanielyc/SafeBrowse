//! App-owned permission prompts, separate from untrusted website content.

use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event_loop::{EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowId};
use wry::WebContext;

use super::assets::{generate_download_prompt_html, generate_permission_prompt_html};
use super::kiosk::{build_trusted_view, make_rect, KioskEvent, Surface};
use super::shell_windows;
use super::trusted::TrustedWebView;
use crate::browser::downloads::DownloadNotice;
use crate::browser::requests::RequestNotice;

const PROMPT_WIDTH: f64 = 460.0;
const PROMPT_HEIGHT: f64 = 350.0;
const PROMPT_MARGIN: f64 = 16.0;

/// Owns the native prompt and releases its child WebView before the parent window.
pub(super) struct PermissionUi {
    view: TrustedWebView,
    window: Window,
    displayed_request: Option<u64>,
}

impl PermissionUi {
    /// Creates a hidden prompt with the same capture policy as the browser session.
    pub(super) fn new(
        target: &EventLoopWindowTarget<KioskEvent>,
        owner: &Window,
        context: &mut WebContext,
        proxy: &EventLoopProxy<KioskEvent>,
        capture_allowed: bool,
    ) -> Result<Self, String> {
        Self::create(target, owner, context, proxy, capture_allowed, false)
    }

    /// Creates a separate download prompt so file decisions cannot resolve device permissions.
    pub(super) fn new_download(
        target: &EventLoopWindowTarget<KioskEvent>,
        owner: &Window,
        context: &mut WebContext,
        proxy: &EventLoopProxy<KioskEvent>,
        capture_allowed: bool,
    ) -> Result<Self, String> {
        Self::create(target, owner, context, proxy, capture_allowed, true)
    }

    fn create(
        target: &EventLoopWindowTarget<KioskEvent>,
        owner: &Window,
        context: &mut WebContext,
        proxy: &EventLoopProxy<KioskEvent>,
        capture_allowed: bool,
        download: bool,
    ) -> Result<Self, String> {
        let window = shell_windows::create_shell_window(
            target,
            owner,
            if download {
                "SafeBrowse download request"
            } else {
                "SafeBrowse website request"
            },
            LogicalSize::new(PROMPT_WIDTH, PROMPT_HEIGHT),
            capture_allowed,
        )?;
        let view = build_trusted_view(
            &window,
            context,
            if download {
                generate_download_prompt_html()
            } else {
                generate_permission_prompt_html()
            },
            if download {
                Surface::DownloadPrompt
            } else {
                Surface::PermissionPrompt
            },
            proxy,
        )?;
        Ok(Self {
            view,
            window,
            displayed_request: None,
        })
    }

    pub(super) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(super) fn displayed_request(&self) -> Option<u64> {
        self.displayed_request
    }

    /// Keeps prompts visible until an explicit decision, cancellation, or tab change.
    pub(super) fn present(
        &mut self,
        owner: &Window,
        request: &RequestNotice,
    ) -> Result<(), String> {
        self.present_payload(owner, request.id, serde_json::json!(request))
    }

    /// Displays native download metadata as text in the dedicated trusted download document.
    pub(super) fn present_download(
        &mut self,
        owner: &Window,
        request: &DownloadNotice,
    ) -> Result<(), String> {
        self.present_payload(owner, request.id, serde_json::json!(request))
    }

    fn present_payload(
        &mut self,
        owner: &Window,
        request_id: u64,
        payload: serde_json::Value,
    ) -> Result<(), String> {
        self.position(owner);
        self.resize()?;
        self.view
            .evaluate_script(&format!("window.showRequest?.({});", payload))
            .map_err(|error| error.to_string())?;
        self.view
            .set_visible(true)
            .map_err(|error| error.to_string())?;
        if self.displayed_request != Some(request_id) {
            self.window.set_visible(true);
            self.window.set_focus();
            self.view.focus().map_err(|error| error.to_string())?;
        }
        self.displayed_request = Some(request_id);
        Ok(())
    }

    /// Releases focus without redirecting it into a page the user may have left.
    pub(super) fn hide(&mut self) -> Result<(), String> {
        self.view
            .set_visible(false)
            .map_err(|error| error.to_string())?;
        if self.displayed_request.take().is_some() {
            self.window.set_visible(false);
        }
        Ok(())
    }

    pub(super) fn show_error(&self, error: &str) {
        let _ = self.view.evaluate_script(&format!(
            "window.showRequestError?.({});",
            serde_json::json!(error)
        ));
    }

    pub(super) fn resize(&self) -> Result<(), String> {
        let size = self
            .window
            .inner_size()
            .to_logical::<f64>(self.window.scale_factor());
        self.view
            .set_bounds(make_rect(0.0, 0.0, size.width, size.height))
            .map_err(|error| error.to_string())
    }

    fn position(&self, owner: &Window) {
        let Some(monitor) = owner.current_monitor() else {
            return;
        };
        let scale = monitor.scale_factor();
        let monitor_origin = monitor.position();
        let monitor_size = monitor.size();
        let owner_origin = owner.inner_position().unwrap_or(monitor_origin);
        let size = owner.inner_size();
        let margin = (PROMPT_MARGIN * scale).round() as i32;
        let width = (PROMPT_WIDTH * scale).round() as i32;
        let height = (PROMPT_HEIGHT * scale).round() as i32;
        let x = (owner_origin.x + (size.width as i32 - width) / 2)
            .min(monitor_origin.x + monitor_size.width as i32 - width - margin)
            .max(monitor_origin.x);
        let y = (owner_origin.y
            + (super::kiosk::BROWSER_CHROME_HEIGHT * scale).round() as i32
            + margin)
            .min(monitor_origin.y + monitor_size.height as i32 - height - margin)
            .max(monitor_origin.y);
        self.window.set_outer_position(PhysicalPosition::new(x, y));
    }
}
