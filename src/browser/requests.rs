//! Deferred WebView2 requests owned exclusively by the native UI thread.
//!
//! Content views must omit Wry's permission and new-window callbacks: their synchronous
//! responses cannot represent an application permission prompt. Attach before the first
//! navigation, retain the attachment alongside its WebView, and resolve requests from the
//! event loop rather than from inside a WebView2 callback.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

use serde::Serialize;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    take_pwstr, GetNonDefaultPermissionSettingsCompletedHandler, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, PermissionRequestedEventHandler,
    SetPermissionStateCompletedHandler, WindowCloseRequestedEventHandler,
};
use webview2_core::{Interface, BOOL, HSTRING, PWSTR};
use wry::{WebView, WebViewExtWindows};

pub type RequestId = u64;
const MAX_PENDING_REQUESTS: usize = 16;
const MAX_PENDING_REQUESTS_PER_TAB: usize = 8;
const MAX_REQUEST_URL_BYTES: usize = 8192;

use super::permissions::SitePermission;

/// Contains only owned, Send-safe data; no COM object can escape through a UI event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestNotice {
    pub id: RequestId,
    pub tab_id: usize,
    pub permission: SitePermission,
    pub origin: String,
    pub requesting_url: String,
    pub target_url: Option<String>,
    pub user_initiated: bool,
}

/// Native invalidation reaches the shell so a prompt cannot outlive its requesting document.
#[derive(Debug, Clone)]
pub enum RequestEvent {
    Requested(RequestNotice),
    Cancelled(Vec<RequestId>),
    Failed { tab_id: usize, message: String },
    CloseRequested { tab_id: usize },
}

type RequestNotifier = Rc<dyn Fn(RequestEvent)>;
type ResetCompletion = Box<dyn FnOnce(Result<(), String>)>;

/// Removes engine decisions left by earlier builds before the first website navigates.
///
/// Call once for the shared browser profile and continue startup from `completed`.
/// Only permission decisions are reset; cookies, storage, passwords, and browsing data are untouched.
/// Existing streams are not stopped by changing permission settings: reload or close their page.
pub fn reset_native_permission_decisions(
    view: &WebView,
    completed: impl FnOnce(Result<(), String>) + 'static,
) -> Result<(), String> {
    let profile = unsafe {
        view.webview()
            .cast::<ICoreWebView2_13>()
            .and_then(|view| view.Profile())
            .and_then(|profile| profile.cast::<ICoreWebView2Profile4>())
    }
    .map_err(|error| format!("Cannot initialize website permission policy: {error}"))?;
    let profile_for_callback = profile.clone();
    unsafe {
        profile.GetNonDefaultPermissionSettings(
            &GetNonDefaultPermissionSettingsCompletedHandler::create(Box::new(
                move |result, collection| {
                    let settings = result
                        .map_err(|error| error.to_string())
                        .and_then(|_| {
                            collection.ok_or_else(|| {
                                "WebView2 returned no permission settings".to_string()
                            })
                        })
                        .and_then(read_native_permission_settings);
                    match settings {
                        Ok(settings) => {
                            reset_next_permission(Rc::new(RefCell::new(PermissionReset {
                                profile: profile_for_callback,
                                settings: settings.into_iter(),
                                completed: Some(Box::new(completed)),
                            })));
                        }
                        Err(error) => completed(Err(error)),
                    }
                    Ok(())
                },
            )),
        )
    }
    .map_err(|error| format!("Cannot read existing website permissions: {error}"))
}

struct PermissionReset {
    profile: ICoreWebView2Profile4,
    settings: std::vec::IntoIter<(COREWEBVIEW2_PERMISSION_KIND, HSTRING)>,
    completed: Option<ResetCompletion>,
}

/// Reads N permission records in O(N) time and space; resets are then issued sequentially.
fn read_native_permission_settings(
    collection: ICoreWebView2PermissionSettingCollectionView,
) -> Result<Vec<(COREWEBVIEW2_PERMISSION_KIND, HSTRING)>, String> {
    let read = || unsafe {
        let mut count = 0;
        collection.Count(&mut count)?;
        let mut settings = Vec::with_capacity(count as usize);
        for index in 0..count {
            let setting = collection.GetValueAtIndex(index)?;
            let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
            let mut origin = PWSTR::null();
            setting.PermissionKind(&mut kind)?;
            setting.PermissionOrigin(&mut origin)?;
            settings.push((kind, HSTRING::from(take_pwstr(origin))));
        }
        Ok::<_, webview2_core::Error>(settings)
    };
    read().map_err(|error| format!("Cannot inspect existing website permissions: {error}"))
}

/// Issues one asynchronous reset at a time, keeping only one native operation outstanding.
fn reset_next_permission(reset: Rc<RefCell<PermissionReset>>) {
    let next = {
        let mut state = reset.borrow_mut();
        state
            .settings
            .next()
            .map(|setting| (state.profile.clone(), setting))
    };
    let Some((profile, (kind, origin))) = next else {
        finish_permission_reset(&reset, Ok(()));
        return;
    };
    let next_reset = Rc::clone(&reset);
    let result = unsafe {
        profile.SetPermissionState(
            kind,
            &origin,
            COREWEBVIEW2_PERMISSION_STATE_DEFAULT,
            &SetPermissionStateCompletedHandler::create(Box::new(move |result| {
                match result {
                    Ok(()) => reset_next_permission(next_reset),
                    Err(error) => finish_permission_reset(&next_reset, Err(error.to_string())),
                }
                Ok(())
            })),
        )
    };
    if let Err(error) = result {
        finish_permission_reset(&reset, Err(error.to_string()));
    }
}

fn finish_permission_reset(reset: &Rc<RefCell<PermissionReset>>, result: Result<(), String>) {
    let callback = reset.borrow_mut().completed.take();
    if let Some(callback) = callback {
        callback(result);
    }
}

enum NativeRequest {
    Permission(ICoreWebView2PermissionRequestedEventArgs),
    Popup {
        arguments: ICoreWebView2NewWindowRequestedEventArgs,
        environment: ICoreWebView2Environment,
    },
}

/// Owns a deferral until a decision, navigation, closure, or broker shutdown completes it.
struct PendingRequest {
    notice: RequestNotice,
    native: NativeRequest,
    deferral: Option<ICoreWebView2Deferral>,
}

impl PendingRequest {
    fn deny(&self) {
        unsafe {
            match &self.native {
                NativeRequest::Permission(arguments) => {
                    let _ = arguments.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY);
                }
                NativeRequest::Popup { arguments, .. } => {
                    let _ = arguments.SetHandled(true);
                }
            }
        }
    }

    fn complete(mut self) -> Result<(), String> {
        let deferral = self.deferral.take().ok_or("Request already completed")?;
        unsafe { deferral.Complete() }.map_err(|error| {
            self.deny();
            format!("Could not complete the website request: {error}")
        })
    }
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        if let Some(deferral) = self.deferral.take() {
            self.deny();
            unsafe {
                let _ = deferral.Complete();
            }
        }
    }
}

struct RequestRegistry {
    next_id: RequestId,
    pending: BTreeMap<RequestId, PendingRequest>,
}

/// Coordinates decisions while retaining native arguments on their original COM apartment.
pub struct RequestBroker {
    registry: Rc<RefCell<RequestRegistry>>,
}

impl Default for RequestBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestBroker {
    pub fn new() -> Self {
        Self {
            registry: Rc::new(RefCell::new(RequestRegistry {
                next_id: 1,
                pending: BTreeMap::new(),
            })),
        }
    }

    /// Attaches before navigation; keep the returned guard alive for the lifetime of this tab.
    pub fn attach(
        &self,
        view: &WebView,
        tab_id: usize,
        notify: impl Fn(RequestEvent) + 'static,
    ) -> Result<RequestAttachment, String> {
        let notify: RequestNotifier = Rc::new(notify);
        let native = view.webview();
        let mut attachment = RequestAttachment {
            view: native.clone(),
            tab_id,
            registry: Rc::downgrade(&self.registry),
            notify: Rc::clone(&notify),
            permission_token: None,
            popup_token: None,
            navigation_token: None,
            frame_navigation_token: None,
            close_token: None,
        };
        unsafe {
            let registry = Rc::downgrade(&self.registry);
            let permission_notify = Rc::clone(&notify);
            let mut token = 0;
            native
                .add_PermissionRequested(
                    &PermissionRequestedEventHandler::create(Box::new(move |sender, arguments| {
                        if let Some(arguments) = arguments {
                            if let Err(error) = queue_permission(
                                &registry,
                                tab_id,
                                sender.as_ref(),
                                arguments,
                                &permission_notify,
                            ) {
                                permission_notify(RequestEvent::Failed {
                                    tab_id,
                                    message: error,
                                });
                            }
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(|error| format!("Cannot monitor website permissions: {error}"))?;
            attachment.permission_token = Some(token);

            let registry = Rc::downgrade(&self.registry);
            let popup_notify = Rc::clone(&notify);
            let environment = view.environment();
            native
                .add_NewWindowRequested(
                    &NewWindowRequestedEventHandler::create(Box::new(move |_, arguments| {
                        if let Some(arguments) = arguments {
                            if let Err(error) = queue_popup(
                                &registry,
                                tab_id,
                                arguments,
                                environment.clone(),
                                &popup_notify,
                            ) {
                                popup_notify(RequestEvent::Failed {
                                    tab_id,
                                    message: error,
                                });
                            }
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(|error| format!("Cannot monitor website popups: {error}"))?;
            attachment.popup_token = Some(token);

            let registry = Rc::downgrade(&self.registry);
            let navigation_notify = Rc::clone(&notify);
            let navigation_handler =
                NavigationStartingEventHandler::create(Box::new(move |_, _| {
                    if let Some(registry) = registry.upgrade() {
                        notify_cancelled(&registry, tab_id, &navigation_notify);
                    }
                    Ok(())
                }));
            native
                .add_NavigationStarting(&navigation_handler, &mut token)
                .map_err(|error| {
                    format!("Cannot invalidate website requests on navigation: {error}")
                })?;
            attachment.navigation_token = Some(token);
            native
                .add_FrameNavigationStarting(&navigation_handler, &mut token)
                .map_err(|error| {
                    format!("Cannot invalidate website requests on frame navigation: {error}")
                })?;
            attachment.frame_navigation_token = Some(token);
            let close_notify = Rc::clone(&notify);
            native
                .add_WindowCloseRequested(
                    &WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
                        close_notify(RequestEvent::CloseRequested { tab_id });
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(|error| format!("Cannot monitor popup closure: {error}"))?;
            attachment.close_token = Some(token);
        }
        Ok(attachment)
    }

    /// Returns metadata only while the original requesting document is still pending.
    pub fn pending(&self, id: RequestId) -> Option<RequestNotice> {
        self.registry
            .borrow()
            .pending
            .get(&id)
            .map(|request| request.notice.clone())
    }

    /// Returns pending requests in arrival order, bounded by the global request limit.
    pub fn pending_requests(&self) -> Vec<RequestNotice> {
        self.registry
            .borrow()
            .pending
            .values()
            .map(|request| request.notice.clone())
            .collect()
    }

    /// Completes a single permission decision; persistence is exclusively the application's policy.
    pub fn resolve_permission(&self, id: RequestId, allow: bool) -> Result<(), String> {
        let request = self.take(id)?;
        let NativeRequest::Permission(arguments) = &request.native else {
            return Err("This request opens a popup, not a device permission".into());
        };
        unsafe {
            arguments.SetState(if allow {
                COREWEBVIEW2_PERMISSION_STATE_ALLOW
            } else {
                COREWEBVIEW2_PERMISSION_STATE_DENY
            })
        }
        .map_err(|error| format!("Could not apply the permission decision: {error}"))?;
        request.complete()
    }

    /// Supplies the opener environment for a fresh child created without `.with_url` or `.with_html`.
    /// The child must also use the opener's profile and finish installing scripts before approval.
    pub fn popup_environment(&self, id: RequestId) -> Result<ICoreWebView2Environment, String> {
        let registry = self.registry.borrow();
        match registry.pending.get(&id).map(|request| &request.native) {
            Some(NativeRequest::Popup { environment, .. }) => Ok(environment.clone()),
            Some(_) => Err("This request does not open a popup".into()),
            None => Err("This website request has expired".into()),
        }
    }

    /// Hands the original window.open request a native child, retaining WindowProxy/opener behavior.
    pub fn resolve_popup(&self, id: RequestId, child: &WebView) -> Result<(), String> {
        let request = self.take(id)?;
        let NativeRequest::Popup {
            arguments,
            environment,
        } = &request.native
        else {
            return Err("This request does not open a popup".into());
        };
        if Interface::as_raw(environment) != Interface::as_raw(&child.environment()) {
            return Err("The popup must share its opener's browser environment".into());
        }
        let child_url = child.url().map_err(|error| error.to_string())?;
        if !child_url.is_empty() && child_url != "about:blank" {
            return Err("The popup must be a new browser view that has never navigated".into());
        }
        unsafe {
            arguments
                .SetHandled(true)
                .and_then(|_| arguments.SetNewWindow(&child.webview()))
        }
        .map_err(|error| format!("Could not open the popup inside SafeBrowse: {error}"))?;
        request.complete()
    }

    /// Denies a popup or permission when dismissed, blocked by policy, or unable to create a child.
    pub fn deny(&self, id: RequestId) -> Result<(), String> {
        let request = self.take(id)?;
        request.deny();
        request.complete()
    }

    /// Cancels a tab before its WebView is removed; completion happens outside registry borrows.
    pub fn cancel_tab(&self, tab_id: usize) -> Vec<RequestId> {
        cancel_tab(&self.registry, tab_id)
    }

    /// Denies every outstanding request before closing the session or changing its policy.
    pub fn cancel_all(&self) -> Vec<RequestId> {
        let pending = std::mem::take(&mut self.registry.borrow_mut().pending);
        let identifiers = pending.keys().copied().collect();
        drop(pending);
        identifiers
    }

    fn take(&self, id: RequestId) -> Result<PendingRequest, String> {
        self.registry
            .borrow_mut()
            .pending
            .remove(&id)
            .ok_or_else(|| "This website request has expired".into())
    }
}

impl Drop for RequestBroker {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

/// Unsubscribes before releasing the native view and denies any request still owned by that tab.
pub struct RequestAttachment {
    view: ICoreWebView2,
    tab_id: usize,
    registry: Weak<RefCell<RequestRegistry>>,
    notify: RequestNotifier,
    permission_token: Option<i64>,
    popup_token: Option<i64>,
    navigation_token: Option<i64>,
    frame_navigation_token: Option<i64>,
    close_token: Option<i64>,
}

impl Drop for RequestAttachment {
    fn drop(&mut self) {
        unsafe {
            if let Some(token) = self.permission_token.take() {
                let _ = self.view.remove_PermissionRequested(token);
            }
            if let Some(token) = self.popup_token.take() {
                let _ = self.view.remove_NewWindowRequested(token);
            }
            if let Some(token) = self.navigation_token.take() {
                let _ = self.view.remove_NavigationStarting(token);
            }
            if let Some(token) = self.frame_navigation_token.take() {
                let _ = self.view.remove_FrameNavigationStarting(token);
            }
            if let Some(token) = self.close_token.take() {
                let _ = self.view.remove_WindowCloseRequested(token);
            }
        }
        if let Some(registry) = self.registry.upgrade() {
            notify_cancelled(&registry, self.tab_id, &self.notify);
        }
    }
}

fn queue_permission(
    registry: &Weak<RefCell<RequestRegistry>>,
    tab_id: usize,
    sender: Option<&ICoreWebView2>,
    arguments: ICoreWebView2PermissionRequestedEventArgs,
    notify: &RequestNotifier,
) -> Result<(), String> {
    unsafe { arguments.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY) }
        .map_err(|error| error.to_string())?;
    let modern_arguments = arguments
        .cast::<ICoreWebView2PermissionRequestedEventArgs3>()
        .map_err(|_| "Update Microsoft Edge WebView2 to manage website permissions safely")?;
    unsafe { modern_arguments.SetSavesInProfile(false) }.map_err(|error| error.to_string())?;
    let sender = sender.ok_or("The requesting browser document is no longer available")?;
    let mut native_kind = COREWEBVIEW2_PERMISSION_KIND::default();
    let mut user_initiated = BOOL::default();
    let mut uri = PWSTR::null();
    unsafe {
        arguments
            .PermissionKind(&mut native_kind)
            .map_err(|error| error.to_string())?;
        arguments
            .IsUserInitiated(&mut user_initiated)
            .map_err(|error| error.to_string())?;
        arguments.Uri(&mut uri).map_err(|error| error.to_string())?;
    }
    let requesting_url = take_pwstr(uri);
    let permission =
        permission_kind(native_kind).ok_or("This website permission is not supported")?;
    let origin = exact_origin(&requesting_url)?;
    validate_permission_origin(&origin, &document_source(sender)?)?;
    let deferral = unsafe { arguments.GetDeferral() }.map_err(|error| error.to_string())?;
    enqueue(
        registry,
        PendingRequest {
            notice: RequestNotice {
                id: 0,
                tab_id,
                permission,
                origin,
                requesting_url,
                target_url: None,
                user_initiated: user_initiated.as_bool(),
            },
            native: NativeRequest::Permission(arguments),
            deferral: Some(deferral),
        },
        notify,
    )
}

fn queue_popup(
    registry: &Weak<RefCell<RequestRegistry>>,
    tab_id: usize,
    arguments: ICoreWebView2NewWindowRequestedEventArgs,
    environment: ICoreWebView2Environment,
    notify: &RequestNotifier,
) -> Result<(), String> {
    unsafe { arguments.SetHandled(true) }.map_err(|error| error.to_string())?;
    let source_frame = unsafe {
        arguments
            .cast::<ICoreWebView2NewWindowRequestedEventArgs3>()
            .and_then(|arguments| arguments.OriginalSourceFrameInfo())
    }.map_err(|_| "This popup was blocked because its requesting frame could not be identified. Update Microsoft Edge WebView2 and try again.")?;
    let mut source_uri = PWSTR::null();
    unsafe { source_frame.Source(&mut source_uri) }.map_err(|error| error.to_string())?;
    let requesting_url = take_pwstr(source_uri);
    let origin = exact_origin(&requesting_url)?;
    let mut destination_uri = PWSTR::null();
    unsafe { arguments.Uri(&mut destination_uri) }.map_err(|error| error.to_string())?;
    let destination_url = take_pwstr(destination_uri);
    validate_popup_destination(&destination_url)?;
    let mut user_initiated = BOOL::default();
    unsafe { arguments.IsUserInitiated(&mut user_initiated) }.map_err(|error| error.to_string())?;
    let deferral = unsafe { arguments.GetDeferral() }.map_err(|error| error.to_string())?;
    enqueue(
        registry,
        PendingRequest {
            notice: RequestNotice {
                id: 0,
                tab_id,
                permission: SitePermission::Popups,
                origin,
                requesting_url,
                target_url: Some(destination_url),
                user_initiated: user_initiated.as_bool(),
            },
            native: NativeRequest::Popup {
                arguments,
                environment,
            },
            deferral: Some(deferral),
        },
        notify,
    )
}

/// Insertion is O(N + log N) time with a strict 16-request bound, and O(1) extra space.
fn enqueue(
    registry: &Weak<RefCell<RequestRegistry>>,
    mut request: PendingRequest,
    notify: &RequestNotifier,
) -> Result<(), String> {
    let registry = registry.upgrade().ok_or("The browser session has closed")?;
    let notice = {
        let mut registry = registry.borrow_mut();
        let tab_requests = registry
            .pending
            .values()
            .filter(|pending| pending.notice.tab_id == request.notice.tab_id)
            .count();
        if registry.pending.len() >= MAX_PENDING_REQUESTS
            || tab_requests >= MAX_PENDING_REQUESTS_PER_TAB
        {
            return Err("Too many website requests are waiting for a decision".into());
        }
        let id = registry.next_id;
        registry.next_id = id
            .checked_add(1)
            .ok_or("Website request identifiers exhausted")?;
        request.notice.id = id;
        let notice = request.notice.clone();
        registry.pending.insert(id, request);
        notice
    };
    notify(RequestEvent::Requested(notice));
    Ok(())
}

/// Cancellation is O(N log N) time and O(N) space, bounded by MAX_PENDING_REQUESTS.
fn cancel_tab(registry: &Rc<RefCell<RequestRegistry>>, tab_id: usize) -> Vec<RequestId> {
    let (identifiers, removed) = {
        let mut registry = registry.borrow_mut();
        let identifiers = registry
            .pending
            .iter()
            .filter_map(|(id, request)| (request.notice.tab_id == tab_id).then_some(*id))
            .collect::<Vec<_>>();
        let removed = identifiers
            .iter()
            .filter_map(|id| registry.pending.remove(id))
            .collect::<Vec<_>>();
        (identifiers, removed)
    };
    // Deferral completion can dispatch native callbacks; release the RefCell before it runs.
    drop(removed);
    identifiers
}

fn notify_cancelled(
    registry: &Rc<RefCell<RequestRegistry>>,
    tab_id: usize,
    notify: &RequestNotifier,
) {
    let identifiers = cancel_tab(registry, tab_id);
    if !identifiers.is_empty() {
        notify(RequestEvent::Cancelled(identifiers));
    }
}

fn exact_origin(uri: &str) -> Result<String, String> {
    if uri.len() > MAX_REQUEST_URL_BYTES {
        return Err("The requesting website address is too long".into());
    }
    let parsed = url::Url::parse(uri).map_err(|_| "The requesting website address is invalid")?;
    if !matches!(parsed.scheme(), "https" | "http") || parsed.host_str().is_none() {
        return Err("Only HTTP and HTTPS websites can request permissions or popups".into());
    }
    Ok(parsed.origin().ascii_serialization())
}

/// Reads the top-level URI from WebView2, never from website-controlled IPC or JavaScript.
fn document_source(view: &ICoreWebView2) -> Result<String, String> {
    let mut uri = PWSTR::null();
    unsafe { view.Source(&mut uri) }.map_err(|error| error.to_string())?;
    Ok(take_pwstr(uri))
}

/// A single-origin permission store cannot safely reuse grants in unrelated embedding contexts.
fn validate_permission_origin(request_origin: &str, top_level_uri: &str) -> Result<(), String> {
    if request_origin == exact_origin(top_level_uri)? {
        return Ok(());
    }
    Err("Permissions requested by an embedded website from another origin are blocked. Open that website in its own tab.".into())
}

fn validate_popup_destination(uri: &str) -> Result<(), String> {
    if uri == "about:blank" {
        return Ok(());
    }
    exact_origin(uri).map(|_| ())
}

fn permission_kind(kind: COREWEBVIEW2_PERMISSION_KIND) -> Option<SitePermission> {
    Some(match kind {
        COREWEBVIEW2_PERMISSION_KIND_CAMERA => SitePermission::Camera,
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE => SitePermission::Microphone,
        COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION => SitePermission::Location,
        COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS => SitePermission::Notifications,
        COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ => SitePermission::ClipboardRead,
        COREWEBVIEW2_PERMISSION_KIND_LOCAL_FONTS => SitePermission::LocalFonts,
        COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS => SitePermission::OtherSensors,
        COREWEBVIEW2_PERMISSION_KIND_MIDI_SYSTEM_EXCLUSIVE_MESSAGES => {
            SitePermission::MidiSystemExclusive
        }
        COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY => SitePermission::Autoplay,
        COREWEBVIEW2_PERMISSION_KIND_WINDOW_MANAGEMENT => SitePermission::WindowManagement,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_origin_preserves_scheme_host_and_nondefault_port_only() {
        assert_eq!(
            exact_origin("https://EXAMPLE.com:443/private?q=secret").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            exact_origin("http://example.com:8080/private").unwrap(),
            "http://example.com:8080"
        );
        assert_ne!(
            exact_origin("http://example.com").unwrap(),
            exact_origin("https://example.com").unwrap()
        );
    }

    #[test]
    fn opaque_origins_and_external_popup_schemes_are_not_authorized() {
        for uri in [
            "about:blank",
            "data:text/html,hello",
            "file:///tmp/test.html",
            "javascript:alert(1)",
            "mailto:a@example.com",
        ] {
            assert!(exact_origin(uri).is_err());
        }
        assert!(validate_popup_destination("about:blank").is_ok());
        assert!(validate_popup_destination("https://example.com/login").is_ok());
        assert!(validate_popup_destination("javascript:alert(1)").is_err());
    }

    #[test]
    fn unknown_native_permissions_cannot_turn_into_an_allowable_prompt() {
        assert_eq!(
            permission_kind(COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION),
            None
        );
        assert_eq!(permission_kind(COREWEBVIEW2_PERMISSION_KIND(99)), None);
        assert_eq!(
            permission_kind(COREWEBVIEW2_PERMISSION_KIND_FILE_READ_WRITE),
            None
        );
        assert_eq!(
            permission_kind(COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS),
            None
        );
        assert_eq!(
            permission_kind(COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ),
            Some(SitePermission::ClipboardRead)
        );
    }

    #[test]
    fn embedded_permissions_cannot_reuse_a_top_level_origin_rule() {
        assert!(
            validate_permission_origin("https://example.com", "https://example.com/path").is_ok()
        );
        assert!(
            validate_permission_origin("https://embedded.example", "https://example.com").is_err()
        );
        assert!(
            validate_permission_origin("https://example.com:8443", "https://example.com").is_err()
        );
        assert!(validate_permission_origin("http://example.com", "https://example.com").is_err());
        assert!(validate_permission_origin("https://example.com", "about:blank").is_err());
    }
}
