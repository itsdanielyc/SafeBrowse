//! Saved file policies and decisions for individual native download deferrals.

use super::*;

impl BrowserSession {
    /// Applies the committed global policy without accepting origins or paths from website IPC.
    pub(super) fn handle_download_event(&mut self, event: DownloadEvent) -> Result<(), String> {
        let result = match event {
            DownloadEvent::Requested(request) => {
                if self.downloads.pending(request.id).is_none() {
                    return Ok(());
                }
                if self.warning_pending {
                    self.downloads.resolve(request.id, false)
                } else {
                    match self.permissions.download_policy() {
                        PermissionDecision::Ask => Ok(()),
                        PermissionDecision::Allow => self.downloads.resolve(request.id, true),
                        PermissionDecision::Block => {
                            self.notice(
                                "Downloads are disabled. You can change this in Settings.",
                                true,
                            );
                            self.downloads.resolve(request.id, false)
                        }
                    }
                }
            }
            DownloadEvent::Started { notice, .. } => {
                self.notice(&format!("Downloading {}…", notice.file_name), false);
                Ok(())
            }
            DownloadEvent::Completed { path, .. } => {
                self.notice(&format!("Download saved to {path}"), false);
                Ok(())
            }
            DownloadEvent::Failed { message, .. } => {
                self.notice(&message, true);
                Ok(())
            }
            DownloadEvent::ProtectionFailed { tab_id, message } => {
                self.close_tab_without_download_protection(tab_id, &message)
            }
            DownloadEvent::Cancelled(_) => Ok(()),
        };
        let refresh = self.sync_request_prompt();
        result.and(refresh)
    }

    /// A view that cannot cancel an unapproved transfer must not remain available for browsing.
    fn close_tab_without_download_protection(
        &mut self,
        tab_id: usize,
        message: &str,
    ) -> Result<(), String> {
        if self.tabs.tab(tab_id).is_none() {
            return Ok(());
        }
        self.content.remove(&tab_id);
        self.pending_navigations.remove(&tab_id);
        if self.tabs.list().len() == 1 {
            self.tabs
                .open_or_switch_special("Settings", TabKind::Settings);
        }
        self.tabs.close_tab(tab_id);
        self.input_target = InputTarget::Address;
        self.show_active_tab()?;
        self.notice(
            &format!("This tab was closed because download protection failed. {message}"),
            true,
        );
        Ok(())
    }

    /// A closed prompt denies only its currently displayed download, never a queued replacement.
    pub(super) fn dismiss_download_request(&mut self) -> Result<(), String> {
        if let Some(id) = self.download_ui.displayed_request() {
            if self.downloads.pending(id).is_some() {
                self.downloads.resolve(id, false)?;
            }
        }
        self.sync_request_prompt()
    }

    pub(super) fn handle_download_command(
        &mut self,
        surface: Surface,
        command: &str,
        message: &Value,
    ) -> Result<bool, String> {
        if command == "RESOLVE_DOWNLOAD" {
            if surface != Surface::DownloadPrompt {
                return Err("Invalid download confirmation source.".into());
            }
            let id = message
                .get("id")
                .and_then(Value::as_u64)
                .ok_or("Missing download ID")?;
            let allow = message
                .get("allow")
                .and_then(Value::as_bool)
                .ok_or("Missing download decision")?;
            if self.download_ui.displayed_request() != Some(id) {
                return Ok(true);
            }
            let Some(request) = self.downloads.pending(id) else {
                self.sync_request_prompt()?;
                return Ok(true);
            };
            if request.tab_id != self.tabs.active_id() || self.warning_pending {
                return Ok(true);
            }
            let result = self.downloads.resolve(
                id,
                allow && self.permissions.download_policy() != PermissionDecision::Block,
            );
            self.sync_request_prompt()?;
            result?;
            return Ok(true);
        }
        if !matches!(command, "SET_DOWNLOAD_POLICY" | "SET_PRINTING_ENABLED") {
            return Ok(false);
        }
        if surface != Surface::Internal
            || self
                .tabs
                .active_tab()
                .is_none_or(|tab| tab.kind != TabKind::Settings)
        {
            return Err(
                "Download and printing preferences can only be changed in Settings.".into(),
            );
        }
        match command {
            "SET_DOWNLOAD_POLICY" => {
                let decision = serde_json::from_value::<PermissionDecision>(
                    message.get("decision").cloned().unwrap_or(Value::Null),
                )
                .map_err(|_| "Invalid download policy")?;
                self.permissions.set_download_policy(decision)?;
                // A policy change must not retroactively approve a request the user left pending.
                for request in self.downloads.pending_requests() {
                    self.downloads.resolve(request.id, false)?;
                }
                if decision == PermissionDecision::Block {
                    self.downloads.cancel_all();
                }
            }
            "SET_PRINTING_ENABLED" => {
                let enabled = message
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or("Invalid printing preference")?;
                self.permissions.set_printing_enabled(enabled)?;
            }
            _ => unreachable!(),
        }
        self.sync_permission_settings()?;
        self.sync_request_prompt()?;
        self.notice("Preferences saved.", false);
        Ok(true)
    }
}
