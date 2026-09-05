//! User-confirmed printing through the installed WebView2 runtime and Windows dialog.

use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2_16, COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM,
};
use webview2_core::Interface;
use wry::{WebView, WebViewExtWindows};

const PRINTING_DISABLED_MESSAGE: &str =
    "SafeBrowse print controls are disabled. You can enable them in Settings.";

/// Suppresses ordinary website print calls before page scripts run. This is not an engine boundary:
/// a newly opened popup can expose a native function before document-created injection runs.
/// Host printing uses ShowPrintUI directly and never invokes the replaced JavaScript function.
pub(crate) const WEBSITE_PRINT_GUARD: &str = include_str!("printing/website_print_guard.js");

/// Checks saved policy before a host print action changes focus or invokes the runtime.
/// WebView2 does not expose a corresponding gate for website `window.print()` calls.
pub(crate) fn require_printing_enabled(enabled: bool) -> Result<(), String> {
    if enabled {
        Ok(())
    } else {
        Err(PRINTING_DISABLED_MESSAGE.into())
    }
}

/// Opens the system chooser on the view's UI thread, outside any WebView2 callback.
///
/// Success only means the runtime accepted the dialog request. This API supplies no
/// completion event, so callers must not report that a document was printed or saved.
/// Printer drivers, their dialogs, spool files and output are outside session cleanup
/// and are not covered by SafeBrowse's window capture protection.
pub(crate) fn show_system_print_dialog(view: &WebView, enabled: bool) -> Result<(), String> {
    require_printing_enabled(enabled)?;
    let core = unsafe { view.controller().CoreWebView2() }
        .map_err(|error| format!("Cannot access this page for printing: {error}"))?;
    let printing = core.cast::<ICoreWebView2_16>().map_err(|error| {
        format!("Printing requires a newer Microsoft Edge WebView2 Runtime. Update the runtime using the README instructions and restart SafeBrowse. ({error})")
    })?;
    // The chooser requires user confirmation; never substitute a silent Print/PDF API.
    unsafe { printing.ShowPrintUI(COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM) }
        .map_err(|error| format!("Cannot open the print dialog. Close any existing print dialog and try again. ({error})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_host_printing_points_to_settings() {
        assert_eq!(
            require_printing_enabled(false).unwrap_err(),
            PRINTING_DISABLED_MESSAGE
        );
        assert!(require_printing_enabled(true).is_ok());
    }
}
