//! Navigation policy for bundled pages that carry the privileged application bridge.

use base64::{engine::general_purpose::STANDARD, Engine};
use std::cell::{Cell, RefCell};
use std::ops::Deref;
use std::rc::Rc;
use wry::WebView;

const HTML_DATA_URL_PREFIX: &str = "data:text/html;charset=utf-8;base64,";

/// Shares the single document authorized by native code with its navigation callback.
#[derive(Clone)]
pub(crate) struct TrustedDocument {
    expected_data_url: Rc<RefCell<String>>,
}

impl TrustedDocument {
    pub(crate) fn new(html: &str) -> Self {
        Self {
            expected_data_url: Rc::new(RefCell::new(Self::data_url(html))),
        }
    }

    /// WebView2 versions report NavigateToString as either about:blank or a data URL.
    /// Compare the full document so arbitrary data URLs never inherit the app bridge.
    pub(crate) fn allows_navigation(&self, target: &str) -> bool {
        target == "about:blank" || target == self.expected_data_url.borrow().as_str()
    }

    fn data_url(html: &str) -> String {
        format!("{HTML_DATA_URL_PREFIX}{}", STANDARD.encode(html))
    }
}

/// Keeps the navigation policy in step with native-initiated bundled page replacements.
pub(super) struct TrustedWebView {
    _health: crate::browser::health::BrowserHealthMonitor,
    view: WebView,
    document: TrustedDocument,
    visible: Cell<bool>,
}

impl TrustedWebView {
    pub(super) fn new(
        view: WebView,
        document: TrustedDocument,
        health: crate::browser::health::BrowserHealthMonitor,
    ) -> Self {
        Self {
            _health: health,
            view,
            document,
            visible: Cell::new(false),
        }
    }

    /// Repeated ShowWindow calls can reactivate a control while another window is being minimized.
    pub(super) fn set_visible(&self, visible: bool) -> wry::Result<()> {
        if self.visible.get() != visible {
            self.view.set_visible(visible)?;
            self.visible.set(visible);
        }
        Ok(())
    }

    /// Authorizes the exact replacement before WebView2 starts its navigation callback.
    pub(super) fn load_html(&self, html: &str) -> wry::Result<()> {
        let previous = self
            .document
            .expected_data_url
            .replace(TrustedDocument::data_url(html));
        if let Err(error) = self.view.load_html(html) {
            self.document.expected_data_url.replace(previous);
            return Err(error);
        }
        Ok(())
    }
}

impl Deref for TrustedWebView {
    type Target = WebView;

    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_native_supplied_document_can_navigate_with_the_bridge() {
        let html = "<!doctype html><p>SafeBrowse · 界</p>";
        let document = TrustedDocument::new(html);
        assert!(document.allows_navigation("about:blank"));
        assert!(document.allows_navigation(&TrustedDocument::data_url(html)));
        for target in [
            TrustedDocument::data_url("<script>window.compromised=true</script>"),
            "data:text/html,<script>alert(1)</script>".to_owned(),
            "https://example.com".to_owned(),
            "javascript:alert(1)".to_owned(),
            "about:blank#untrusted".to_owned(),
        ] {
            assert!(!document.allows_navigation(&target));
        }
    }
}
