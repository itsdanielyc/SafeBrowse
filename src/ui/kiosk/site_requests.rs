//! Website policy decisions and their application to the current browser session.

use super::*;
use crate::browser::requests::{RequestId, RequestNotice};

impl BrowserSession {
    /// Native profile decisions from older builds must not silently bypass the policy store.
    pub(super) fn navigate_after_permission_reset(
        &mut self,
        id: usize,
        url: &str,
    ) -> Result<(), String> {
        let view = self.content.get(&id).ok_or("Browser tab is unavailable")?;
        match &self.permission_profile_state {
            PermissionProfileState::Ready => {
                return view.load_url(url).map_err(|error| error.to_string())
            }
            PermissionProfileState::Failed(error) => return Err(error.clone()),
            _ => {}
        }
        self.pending_navigations.insert(id, url.to_owned());
        if matches!(
            self.permission_profile_state,
            PermissionProfileState::Uninitialized
        ) {
            self.permission_profile_state = PermissionProfileState::Loading;
            let proxy = self.proxy.clone();
            if let Err(error) =
                crate::browser::requests::reset_native_permission_decisions(view, move |result| {
                    let _ = proxy.send_event(KioskEvent::PermissionProfileReady(result));
                })
            {
                self.permission_profile_state = PermissionProfileState::Failed(error.clone());
                self.pending_navigations.clear();
                return Err(error);
            }
        }
        Ok(())
    }

    pub(super) fn finish_permission_profile_reset(
        &mut self,
        result: Result<(), String>,
    ) -> Result<(), String> {
        if let Err(error) = result {
            self.permission_profile_state = PermissionProfileState::Failed(error.clone());
            self.pending_navigations.clear();
            return Err(error);
        }
        self.permission_profile_state = PermissionProfileState::Ready;
        let mut first_error = None;
        for (id, url) in std::mem::take(&mut self.pending_navigations) {
            if let Some(view) = self.content.get(&id) {
                if let Err(error) = view.load_url(&url) {
                    first_error.get_or_insert_with(|| error.to_string());
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Refreshes the trusted settings document from the committed policy store.
    pub(super) fn sync_permission_settings(&self) -> Result<(), String> {
        self.internal
            .evaluate_script(&format!(
                "window.updatePermissions?.({});",
                json!(self.permissions.snapshot())
            ))
            .map_err(|error| error.to_string())
    }

    /// Only the active tab can present a request; background tabs retain bounded native deferrals.
    pub(super) fn sync_request_prompt(&mut self) -> Result<(), String> {
        if self.warning_pending || self.window.is_minimized() {
            self.permission_ui.hide()?;
            return self.download_ui.hide();
        }
        let pending = self
            .requests
            .pending_requests()
            .into_iter()
            .find(|request| request.tab_id == self.tabs.active_id());
        match pending {
            Some(request) => {
                self.download_ui.hide()?;
                self.permission_ui.present(&self.window, &request)
            }
            None => {
                self.permission_ui.hide()?;
                let download = self
                    .downloads
                    .pending_requests()
                    .into_iter()
                    .find(|request| request.tab_id == self.tabs.active_id());
                if let Some(request) = download
                    .filter(|_| self.permissions.download_policy() == PermissionDecision::Ask)
                {
                    self.download_ui.present_download(&self.window, &request)
                } else {
                    self.download_ui.hide()
                }
            }
        }
    }

    /// Handles native request metadata without accepting website-supplied IPC or origin claims.
    pub(super) fn handle_browser_request(&mut self, event: RequestEvent) -> Result<bool, String> {
        match event {
            RequestEvent::Requested(request) => self.apply_saved_decision(request)?,
            RequestEvent::Cancelled(_) => self.sync_request_prompt()?,
            RequestEvent::Failed { tab_id, message } => {
                if tab_id == self.tabs.active_id() {
                    self.notice(&message, true);
                }
            }
            RequestEvent::CloseRequested { tab_id } => {
                if self.content.get(&tab_id).is_some_and(|view| view.is_popup) {
                    if self.tabs.list().len() == 1 {
                        return Ok(true);
                    }
                    self.tabs.close_tab(tab_id);
                    self.content.remove(&tab_id);
                    self.pending_navigations.remove(&tab_id);
                    self.input_target = InputTarget::Address;
                    self.show_active_tab()?;
                }
            }
        }
        Ok(false)
    }

    fn apply_saved_decision(&mut self, request: RequestNotice) -> Result<(), String> {
        if self.requests.pending(request.id).is_none() {
            return Ok(());
        }
        match self
            .permissions
            .decision(&request.origin, request.permission)?
        {
            PermissionDecision::Ask => self.sync_request_prompt(),
            decision => self.finish_request(request.id, decision == PermissionDecision::Allow),
        }
    }

    /// Cancels or grants the exact native request. Popup views inherit the opener's environment.
    fn finish_request(&mut self, id: RequestId, allow: bool) -> Result<(), String> {
        let Some(request) = self.requests.pending(id) else {
            return self.sync_request_prompt();
        };
        let result = if !allow {
            self.requests.deny(id)
        } else if request.permission == SitePermission::Popups {
            self.open_requested_popup(&request)
        } else {
            self.requests.resolve_permission(id, true)
        };
        if result.is_err() {
            let _ = self.requests.deny(id);
        }
        self.sync_request_prompt()?;
        result
    }

    fn open_requested_popup(&mut self, request: &RequestNotice) -> Result<(), String> {
        if self.tabs.list().len() >= MAX_OPEN_TABS {
            return Err(format!(
                "Close a tab before allowing another popup (limit: {MAX_OPEN_TABS})."
            ));
        }
        let environment = self.requests.popup_environment(request.id)?;
        let previous = self.tabs.active_id();
        let id = self
            .tabs
            .open_tab(request.target_url.as_deref().unwrap_or("about:blank"));
        let result = build_content_view(
            &self.window,
            &mut self.browser_context,
            id,
            &self.proxy,
            &self.requests,
            &self.downloads,
            Some(environment),
        )
        .and_then(|view| {
            self.requests.resolve_popup(request.id, &view)?;
            Ok(view)
        });
        let view = match result {
            Ok(view) => view,
            Err(error) => {
                self.tabs.close_tab(id);
                self.tabs.switch_to_tab(previous);
                return Err(error);
            }
        };
        self.content.insert(id, view);
        self.input_target = InputTarget::Content(id);
        self.show_active_tab()?;
        self.input_view().focus().map_err(|error| error.to_string())
    }

    /// Rechecks queued requests after policy changes so a revoked grant cannot remain pending.
    fn refresh_policy_decisions(&mut self) -> Result<(), String> {
        self.sync_permission_settings()?;
        for request in self.requests.pending_requests() {
            self.apply_saved_decision(request)?;
        }
        Ok(())
    }

    pub(super) fn dismiss_site_request(&mut self) -> Result<(), String> {
        if let Some(id) = self.permission_ui.displayed_request() {
            self.finish_request(id, false)?;
        }
        Ok(())
    }

    /// Parses only commands from the matching trusted surface and active prompt identifier.
    pub(super) fn handle_permission_command(
        &mut self,
        surface: Surface,
        command: &str,
        message: &Value,
    ) -> Result<bool, String> {
        if command == "RESOLVE_SITE_REQUEST" {
            if surface != Surface::PermissionPrompt {
                return Err("Invalid permission prompt source.".into());
            }
            let id = message
                .get("id")
                .and_then(Value::as_u64)
                .ok_or("Missing request ID")?;
            if self.permission_ui.displayed_request() != Some(id) {
                return Ok(true);
            }
            let Some(request) = self.requests.pending(id) else {
                self.sync_request_prompt()?;
                return Ok(true);
            };
            let decision: PermissionDecision =
                serde_json::from_value(message.get("decision").cloned().unwrap_or(Value::Null))
                    .map_err(|_| "Invalid permission decision")?;
            if decision == PermissionDecision::Ask {
                return Err("Choose allow or block for this request.".into());
            }
            let remember = message
                .get("remember")
                .and_then(Value::as_bool)
                .ok_or("Missing decision duration")?;
            if remember {
                self.permissions
                    .set_site_rule(&request.origin, request.permission, decision)?;
            }
            self.finish_request(id, decision == PermissionDecision::Allow)?;
            self.sync_permission_settings()?;
            return Ok(true);
        }
        if !matches!(
            command,
            "SET_POPUP_POLICY"
                | "SET_SITE_PERMISSION"
                | "RESET_SITE_PERMISSION"
                | "RELOAD_SITE_TABS"
        ) {
            return Ok(false);
        }
        if surface != Surface::Internal
            || self
                .tabs
                .active_tab()
                .is_none_or(|tab| tab.kind != TabKind::Settings)
        {
            return Err("Permission settings are only available in Settings.".into());
        }
        if command == "RELOAD_SITE_TABS" {
            let origin = crate::browser::permissions::normalize_origin(
                message
                    .get("origin")
                    .and_then(Value::as_str)
                    .ok_or("Missing website address")?,
            )?;
            let mut reloaded = 0;
            for (&id, view) in &self.content {
                let matches_origin = view
                    .url()
                    .ok()
                    .and_then(|url| crate::browser::permissions::normalize_origin(&url).ok())
                    .as_ref()
                    == Some(&origin);
                if matches_origin {
                    self.requests.cancel_tab(id);
                    unsafe {
                        view.controller()
                            .CoreWebView2()
                            .and_then(|core| core.Reload())
                    }
                    .map_err(|error| error.to_string())?;
                    reloaded += 1;
                }
            }
            self.notice(&format!("Reloaded {reloaded} tab(s) for {origin}."), false);
            return Ok(true);
        }
        let decision = || {
            serde_json::from_value::<PermissionDecision>(
                message.get("decision").cloned().unwrap_or(Value::Null),
            )
            .map_err(|_| "Invalid permission decision".to_owned())
        };
        match command {
            "SET_POPUP_POLICY" => self.permissions.set_popup_default(decision()?)?,
            _ => {
                let origin = message
                    .get("origin")
                    .and_then(Value::as_str)
                    .ok_or("Missing website address")?;
                let permission: SitePermission = serde_json::from_value(
                    message.get("permission").cloned().unwrap_or(Value::Null),
                )
                .map_err(|_| "Unknown site permission")?;
                if command == "SET_SITE_PERMISSION" {
                    self.permissions
                        .set_site_rule(origin, permission, decision()?)?;
                } else {
                    self.permissions.remove_site_rule(origin, permission)?;
                }
            }
        }
        self.refresh_policy_decisions()?;
        self.notice(
            "Permission settings saved. Reload or close a website to end access already in use.",
            false,
        );
        Ok(true)
    }
}
