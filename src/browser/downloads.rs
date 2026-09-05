//! Deferred downloads on the WebView's UI thread, with no automatic opening or threat bypass.
//!
//! Content builders must omit both Wry download callbacks: their synchronous DownloadStarting
//! handler cannot represent an app-owned approval prompt. Attach this broker before navigation
//! and keep its attachment alive until the corresponding view is destroyed.

mod destination;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use serde::Serialize;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    take_pwstr, DownloadStartingEventHandler, NavigationStartingEventHandler,
    StateChangedEventHandler, WindowCloseRequestedEventHandler,
};
use webview2_core::{Interface, HSTRING, PWSTR};
use wry::{WebView, WebViewExtWindows};

use super::permissions::normalize_origin;
use destination::{safe_file_name, DownloadDestination};

pub type DownloadId = u64;
const MAX_PENDING_DOWNLOADS: usize = 8;
const MAX_PENDING_DOWNLOADS_PER_TAB: usize = 3;
const MAX_ACTIVE_DOWNLOADS: usize = 4;
const MAX_DOWNLOAD_URL_BYTES: usize = 8192;

/// Owned metadata only; the origin identifies the top-level page, not an attested child frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadNotice {
    pub id: DownloadId,
    pub tab_id: usize,
    pub origin: String,
    pub requesting_url: String,
    pub url: String,
    pub file_name: String,
    pub total_bytes: Option<u64>,
}

/// Reports decisions and terminal states without exposing COM objects to the application loop.
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Requested(DownloadNotice),
    Started {
        notice: DownloadNotice,
        path: String,
    },
    Completed {
        notice: DownloadNotice,
        path: String,
    },
    Cancelled(Vec<DownloadId>),
    Failed {
        tab_id: usize,
        message: String,
    },
    /// Neither native cancellation mechanism succeeded; the host must tear down this tab.
    ProtectionFailed {
        tab_id: usize,
        message: String,
    },
}

type DownloadNotifier = Rc<dyn Fn(DownloadEvent)>;
type RegistryReference = Weak<RefCell<DownloadRegistry>>;

/// A pending transfer remains canceled until the application's approval completes its deferral.
struct PendingDownload {
    notice: DownloadNotice,
    arguments: ICoreWebView2DownloadStartingEventArgs,
    operation: ICoreWebView2DownloadOperation,
    deferral: Option<ICoreWebView2Deferral>,
    notify: DownloadNotifier,
}

impl PendingDownload {
    fn complete(&mut self) -> Result<(), String> {
        let deferral = self
            .deferral
            .take()
            .ok_or("Download request already completed")?;
        if let Err(error) = unsafe { deferral.Complete() } {
            unsafe {
                let _ = self.arguments.SetCancel(true);
                let _ = self.operation.Cancel();
            }
            return Err(format!("Could not complete the download decision: {error}"));
        }
        Ok(())
    }
}

impl Drop for PendingDownload {
    fn drop(&mut self) {
        if let Some(deferral) = self.deferral.take() {
            unsafe {
                let _ = self.arguments.SetCancel(true);
                let _ = self.arguments.SetHandled(true);
                let _ = deferral.Complete();
            }
        }
    }
}

/// Owns native completion observation and the fresh output directory until the transfer ends.
struct ActiveDownload {
    notice: DownloadNotice,
    operation: ICoreWebView2DownloadOperation,
    state_token: Option<i64>,
    destination: DownloadDestination,
    notify: DownloadNotifier,
    completed: bool,
}

impl Drop for ActiveDownload {
    fn drop(&mut self) {
        unsafe {
            if let Some(token) = self.state_token.take() {
                let _ = self.operation.remove_StateChanged(token);
            }
            if !self.completed {
                let _ = self.operation.Cancel();
            }
        }
    }
}

struct DownloadRegistry {
    next_id: DownloadId,
    pending: BTreeMap<DownloadId, PendingDownload>,
    active: BTreeMap<DownloadId, ActiveDownload>,
}

/// Clones share one bounded UI-thread registry. Dropping its final owner cancels all transfers.
#[derive(Clone)]
pub struct DownloadBroker {
    registry: Rc<RefCell<DownloadRegistry>>,
    destination_root: Option<PathBuf>,
}

impl Default for DownloadBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadBroker {
    /// Defers every transfer; the caller applies Ask/Disabled/Always allowed policy to each notice.
    pub fn new() -> Self {
        Self {
            registry: Rc::new(RefCell::new(DownloadRegistry {
                next_id: 1,
                pending: BTreeMap::new(),
                active: BTreeMap::new(),
            })),
            destination_root: None,
        }
    }

    /// Attaches before first navigation. The guard cancels this tab's transfers when dropped.
    pub fn attach(
        &self,
        view: &WebView,
        tab_id: usize,
        notify: impl Fn(DownloadEvent) + 'static,
    ) -> Result<DownloadAttachment, String> {
        let native = view.webview();
        let modern = native.cast::<ICoreWebView2_4>().map_err(|error| {
            format!("Update Microsoft Edge WebView2 Runtime to manage downloads safely: {error}")
        })?;
        let notify: DownloadNotifier = Rc::new(notify);
        let mut attachment = DownloadAttachment {
            native: native.clone(),
            modern,
            tab_id,
            registry: Rc::downgrade(&self.registry),
            download_token: None,
            navigation_token: None,
            close_token: None,
        };
        unsafe {
            let registry = Rc::downgrade(&self.registry);
            let download_notify = Rc::clone(&notify);
            let mut token = 0;
            attachment
                .modern
                .add_DownloadStarting(
                    &DownloadStartingEventHandler::create(Box::new(move |sender, arguments| {
                        if let Some(arguments) = arguments {
                            if let Err(message) = queue_download(
                                &registry,
                                tab_id,
                                sender.as_ref(),
                                arguments,
                                &download_notify,
                            ) {
                                download_notify(DownloadEvent::Failed { tab_id, message });
                            }
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(|error| format!("Cannot monitor downloads: {error}"))?;
            attachment.download_token = Some(token);

            let registry = Rc::downgrade(&self.registry);
            native
                .add_NavigationStarting(
                    &NavigationStartingEventHandler::create(Box::new(move |_, _| {
                        if let Some(registry) = registry.upgrade() {
                            cancel_matching(&registry, Some(tab_id));
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(|error| format!("Cannot invalidate downloads on navigation: {error}"))?;
            attachment.navigation_token = Some(token);

            let registry = Rc::downgrade(&self.registry);
            native
                .add_WindowCloseRequested(
                    &WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
                        if let Some(registry) = registry.upgrade() {
                            cancel_matching(&registry, Some(tab_id));
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(|error| format!("Cannot invalidate downloads on tab closure: {error}"))?;
            attachment.close_token = Some(token);
        }
        Ok(attachment)
    }

    /// Returns only requests whose original top-level document still owns their deferral.
    pub fn pending(&self, id: DownloadId) -> Option<DownloadNotice> {
        self.registry
            .borrow()
            .pending
            .get(&id)
            .map(|pending| pending.notice.clone())
    }

    /// Arrival ordering is deterministic and bounded by MAX_PENDING_DOWNLOADS.
    pub fn pending_requests(&self) -> Vec<DownloadNotice> {
        self.registry
            .borrow()
            .pending
            .values()
            .map(|pending| pending.notice.clone())
            .collect()
    }

    /// Applies one explicit policy decision; approval selects a fresh destination before release.
    pub fn resolve(&self, id: DownloadId, allow: bool) -> Result<(), String> {
        let mut pending = self
            .registry
            .borrow_mut()
            .pending
            .remove(&id)
            .ok_or("This download request expired or was already answered")?;
        if !allow {
            let notify = Rc::clone(&pending.notify);
            drop(pending);
            notify(DownloadEvent::Cancelled(vec![id]));
            return Ok(());
        }
        if self.registry.borrow().active.len() >= MAX_ACTIVE_DOWNLOADS {
            return Err(
                "Too many active downloads. Wait for a transfer to finish and try again.".into(),
            );
        }
        let destination =
            DownloadDestination::new(self.destination_root.as_deref(), &pending.notice.file_name)?;
        let path = destination.path().to_string_lossy().into_owned();
        let mut active = ActiveDownload {
            notice: pending.notice.clone(),
            operation: pending.operation.clone(),
            state_token: None,
            destination,
            notify: Rc::clone(&pending.notify),
            completed: false,
        };
        unsafe {
            pending
                .arguments
                .SetResultFilePath(&HSTRING::from(active.destination.path()))
                .map_err(|error| format!("Cannot choose the download destination: {error}"))?;
            let registry = Rc::downgrade(&self.registry);
            let mut token = 0;
            active
                .operation
                .add_StateChanged(
                    &StateChangedEventHandler::create(Box::new(move |operation, _| {
                        if let (Some(registry), Some(operation)) = (registry.upgrade(), operation) {
                            finish_download(&registry, id, &operation);
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(|error| format!("Cannot observe download completion: {error}"))?;
            active.state_token = Some(token);
            pending
                .arguments
                .SetHandled(true)
                .and_then(|_| pending.arguments.SetCancel(false))
                .map_err(|error| format!("Cannot allow this download: {error}"))?;
        }
        self.registry.borrow_mut().active.insert(id, active);
        (pending.notify)(DownloadEvent::Started {
            notice: pending.notice.clone(),
            path,
        });
        if let Err(error) = pending.complete() {
            let failed = self.registry.borrow_mut().active.remove(&id);
            drop(failed);
            return Err(error);
        }
        // Network failure can become terminal while approval is deferred; do not miss that state.
        finish_download(&self.registry, id, &pending.operation);
        Ok(())
    }

    /// Cancels unresolved requests and active transfers before their tab is destroyed or navigates.
    pub fn cancel_tab(&self, tab_id: usize) {
        cancel_matching(&self.registry, Some(tab_id));
    }

    /// Cancels all app-owned transfers when leaving or shutting down the browser session.
    pub fn cancel_all(&self) {
        cancel_matching(&self.registry, None);
    }
}

/// Unregisters callbacks before cancellation; it never owns or extends the broker's lifetime.
pub struct DownloadAttachment {
    native: ICoreWebView2,
    modern: ICoreWebView2_4,
    tab_id: usize,
    registry: RegistryReference,
    download_token: Option<i64>,
    navigation_token: Option<i64>,
    close_token: Option<i64>,
}

impl Drop for DownloadAttachment {
    fn drop(&mut self) {
        unsafe {
            if let Some(token) = self.download_token.take() {
                let _ = self.modern.remove_DownloadStarting(token);
            }
            if let Some(token) = self.navigation_token.take() {
                let _ = self.native.remove_NavigationStarting(token);
            }
            if let Some(token) = self.close_token.take() {
                let _ = self.native.remove_WindowCloseRequested(token);
            }
        }
        if let Some(registry) = self.registry.upgrade() {
            cancel_matching(&registry, Some(self.tab_id));
        }
    }
}

/// Makes failure default to denial before reading attacker-influenced metadata or allocating state.
fn queue_download(
    registry: &RegistryReference,
    tab_id: usize,
    sender: Option<&ICoreWebView2>,
    arguments: ICoreWebView2DownloadStartingEventArgs,
    notify: &DownloadNotifier,
) -> Result<(), String> {
    match establish_initial_denial(
        || unsafe { arguments.SetCancel(true) }.map_err(|error| error.to_string()),
        || {
            unsafe {
                arguments
                    .DownloadOperation()
                    .and_then(|operation| operation.Cancel())
            }
            .map_err(|error| error.to_string())
        },
    ) {
        Ok(()) => {}
        Err(InitialDenialFailure::Cancelled(message)) => return Err(message),
        Err(InitialDenialFailure::Unprotected(message)) => {
            notify(DownloadEvent::ProtectionFailed { tab_id, message });
            return Ok(());
        }
    }
    unsafe { arguments.SetHandled(true) }
        .map_err(|error| format!("Cannot suppress the canceled download's dialog: {error}"))?;
    let registry = registry
        .upgrade()
        .ok_or("The download session has closed")?;
    let id = {
        let mut state = registry.borrow_mut();
        if state.pending.len() >= MAX_PENDING_DOWNLOADS
            || state
                .pending
                .values()
                .filter(|pending| pending.notice.tab_id == tab_id)
                .count()
                >= MAX_PENDING_DOWNLOADS_PER_TAB
        {
            return Err(
                "Too many download requests. Answer an existing request before downloading again."
                    .into(),
            );
        }
        let id = state.next_id;
        state.next_id = id
            .checked_add(1)
            .ok_or("Download request identifiers exhausted")?;
        id
    };
    let read_metadata = || unsafe {
        let sender =
            sender.ok_or_else(|| "Cannot identify the download's browser tab".to_string())?;
        let mut source = PWSTR::null();
        sender
            .Source(&mut source)
            .map_err(|error| error.to_string())?;
        let requesting_url = bounded_url(take_pwstr(source))?;
        let origin = normalize_origin(&requesting_url)?;
        let operation = arguments
            .DownloadOperation()
            .map_err(|error| error.to_string())?;
        let mut uri = PWSTR::null();
        operation.Uri(&mut uri).map_err(|error| error.to_string())?;
        let url = bounded_url(take_pwstr(uri))?;
        validate_download_url(&url, &origin)?;
        let mut suggested_path = PWSTR::null();
        arguments
            .ResultFilePath(&mut suggested_path)
            .map_err(|error| error.to_string())?;
        let file_name = safe_file_name(&take_pwstr(suggested_path));
        let mut total_bytes = -1;
        operation
            .TotalBytesToReceive(&mut total_bytes)
            .map_err(|error| error.to_string())?;
        let notice = DownloadNotice {
            id,
            tab_id,
            origin,
            requesting_url,
            url,
            file_name,
            total_bytes: u64::try_from(total_bytes).ok(),
        };
        let deferral = arguments.GetDeferral().map_err(|error| error.to_string())?;
        Ok::<_, String>((notice, operation, deferral))
    };
    let (notice, operation, deferral) = read_metadata()?;
    registry.borrow_mut().pending.insert(
        id,
        PendingDownload {
            notice: notice.clone(),
            arguments,
            operation,
            deferral: Some(deferral),
            notify: Rc::clone(notify),
        },
    );
    notify(DownloadEvent::Requested(notice));
    Ok(())
}

/// Distinguishes a canceled request from loss of the boundary that blocks unapproved downloads.
#[derive(Debug, PartialEq, Eq)]
enum InitialDenialFailure {
    Cancelled(String),
    Unprotected(String),
}

/// Tries independent native cancellation paths before any metadata read or deferral is created.
fn establish_initial_denial(
    cancel_request: impl FnOnce() -> Result<(), String>,
    cancel_operation: impl FnOnce() -> Result<(), String>,
) -> Result<(), InitialDenialFailure> {
    let Err(request_error) = cancel_request() else {
        return Ok(());
    };
    match cancel_operation() {
        Ok(()) => Err(InitialDenialFailure::Cancelled(format!(
            "SafeBrowse could not defer this download, so it canceled the native transfer instead. ({request_error})"
        ))),
        Err(operation_error) => Err(InitialDenialFailure::Unprotected(format!(
            "SafeBrowse could not block an unapproved download. The affected tab must close. Request cancellation: {request_error}. Transfer cancellation: {operation_error}."
        ))),
    }
}

fn bounded_url(url: String) -> Result<String, String> {
    if url.len() > MAX_DOWNLOAD_URL_BYTES {
        return Err("The download address is too long to display safely".into());
    }
    Ok(url)
}

fn validate_download_url(uri: &str, source_origin: &str) -> Result<(), String> {
    let parsed = url::Url::parse(uri).map_err(|_| "The download address is invalid")?;
    match parsed.scheme() {
        "http" | "https" => normalize_origin(uri).map(|_| ()),
        "blob" if parsed.origin().ascii_serialization() == source_origin => Ok(()),
        _ => Err(
            "Only web downloads and files generated by the current website are supported.".into(),
        ),
    }
}

/// Removes ownership before COM calls, so synchronous callbacks cannot reborrow the registry.
/// Time: O(P log P + A log A), space: O(P + A); both registries have fixed count limits.
fn cancel_matching(registry: &Rc<RefCell<DownloadRegistry>>, tab_id: Option<usize>) {
    let (pending, active) = {
        let mut registry = registry.borrow_mut();
        let pending_ids: Vec<_> = registry
            .pending
            .iter()
            .filter(|(_, item)| tab_id.is_none_or(|tab| item.notice.tab_id == tab))
            .map(|(&id, _)| id)
            .collect();
        let active_ids: Vec<_> = registry
            .active
            .iter()
            .filter(|(_, item)| tab_id.is_none_or(|tab| item.notice.tab_id == tab))
            .map(|(&id, _)| id)
            .collect();
        let pending: Vec<_> = pending_ids
            .into_iter()
            .filter_map(|id| registry.pending.remove(&id))
            .collect();
        let active: Vec<_> = active_ids
            .into_iter()
            .filter_map(|id| registry.active.remove(&id))
            .collect();
        (pending, active)
    };
    for request in pending {
        let notice = request.notice.clone();
        let notify = Rc::clone(&request.notify);
        drop(request);
        notify(DownloadEvent::Cancelled(vec![notice.id]));
    }
    for transfer in active {
        let notice = transfer.notice.clone();
        let notify = Rc::clone(&transfer.notify);
        drop(transfer);
        notify(DownloadEvent::Cancelled(vec![notice.id]));
    }
}

/// Never resumes an interrupted transfer; runtime malware, reputation and policy blocks remain final.
fn finish_download(
    registry: &Rc<RefCell<DownloadRegistry>>,
    id: DownloadId,
    operation: &ICoreWebView2DownloadOperation,
) {
    let mut state = COREWEBVIEW2_DOWNLOAD_STATE::default();
    let state_result = unsafe { operation.State(&mut state) };
    if state_result.is_ok() && state == COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS {
        return;
    }
    let active = registry.borrow_mut().active.remove(&id);
    let Some(mut active) = active else { return };
    let notify = Rc::clone(&active.notify);
    let event = if state_result.is_ok() && state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
        active.completed = true;
        active.destination.keep();
        DownloadEvent::Completed {
            notice: active.notice.clone(),
            path: active.destination.path().to_string_lossy().into_owned(),
        }
    } else {
        let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON::default();
        let reason_result = unsafe { operation.InterruptReason(&mut reason) };
        let detail = match (state_result, reason_result) {
            (Err(error), _) | (_, Err(error)) => error.to_string(),
            _ => format!("WebView2 interruption code {}", reason.0),
        };
        DownloadEvent::Failed {
            tab_id: active.notice.tab_id,
            message: format!(
                "Download did not finish: {} ({detail}). Runtime security blocks are not bypassed.",
                active.notice.file_name
            ),
        }
    };
    drop(active);
    notify(event);
}

#[cfg(test)]
mod tests;
