//! Mandatory native security settings for every view that can load website content.

use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings8;
use webview2_core::{Interface, BOOL};
use wry::{WebView, WebViewExtWindows};

/// Applies mandatory security settings before a website or popup can navigate.
///
/// Trusted bundled controls intentionally do not use this policy: they cannot navigate to HTTP(S),
/// open popups, request permissions, or download files, and their app messaging bridge is required.
/// Windows or enterprise policy can still disable effective SmartScreen checking while WebView2
/// retains a `true` setting, so the readback verifies the app's requirement rather than OS policy.
pub(crate) fn harden_content_view(view: &WebView) -> Result<(), String> {
    // Wry installs its messaging transport even when no application IPC handler is supplied.
    let read_settings = || unsafe { view.controller().CoreWebView2()?.Settings() };
    let settings = read_settings()
        .map_err(|error| format!("Cannot read website security settings: {error}"))?;
    let disable_bridges = || unsafe {
        settings.SetIsWebMessageEnabled(false)?;
        settings.SetAreHostObjectsAllowed(false)
    };
    disable_bridges().map_err(|error| {
        format!("Cannot disable website access to native host bridges: {error}")
    })?;

    let reputation_settings = settings.cast::<ICoreWebView2Settings8>().map_err(|error| {
        format!(
            "Microsoft Edge WebView2 does not support the required SmartScreen setting; update the WebView2 Runtime: {error}"
        )
    })?;
    let mut reputation_checking_required = BOOL::default();
    let mut require_reputation_checking = || unsafe {
        reputation_settings.SetIsReputationCheckingRequired(true)?;
        reputation_settings.IsReputationCheckingRequired(&mut reputation_checking_required)
    };
    require_reputation_checking()
        .map_err(|error| format!("Cannot require SmartScreen reputation checking: {error}"))?;
    if !reputation_checking_required.as_bool() {
        return Err(
            "WebView2 did not retain the required SmartScreen reputation-checking setting".into(),
        );
    }
    Ok(())
}
