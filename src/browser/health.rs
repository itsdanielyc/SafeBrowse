//! Native engine lifecycle signals; callbacks never reload a page or replay a transaction.

use std::cell::Cell;
use std::rc::Rc;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    BrowserProcessExitedEventHandler, NewBrowserVersionAvailableEventHandler,
    ProcessFailedEventHandler,
};
use webview2_core::Interface;
use wry::{WebView, WebViewExtWindows};

/// Application decisions are separate from WebView2's automatically recovered utility failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserHealthEvent {
    Failed(BrowserFailure),
    UpdateAvailable,
}

/// Describes a failed engine without collecting page addresses, form contents, or crash dumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserFailure {
    BrowserExited,
    RendererExited,
    RendererUnresponsive,
    FrameRendererExited,
    Unexpected,
}

impl BrowserFailure {
    /// A deliberate session exit avoids guessing whether a payment was already submitted.
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::BrowserExited => "The browser engine stopped unexpectedly.",
            Self::RendererExited => "A page stopped unexpectedly.",
            Self::RendererUnresponsive => "A page stopped responding.",
            Self::FrameRendererExited => "Part of a page stopped unexpectedly.",
            Self::Unexpected => "The browser reported an unexpected process failure.",
        }
    }
}

/// Keeps native subscriptions alive and disables them before the owning view is destroyed.
pub(crate) struct BrowserHealthMonitor {
    core: ICoreWebView2,
    environment: ICoreWebView2Environment5,
    listening: Rc<Cell<bool>>,
    process_token: Option<i64>,
    exit_token: Option<i64>,
    update_token: Option<i64>,
}

impl BrowserHealthMonitor {
    /// Attaches before navigation. Partial registration failures unsubscribe through Drop.
    pub(crate) fn attach(
        view: &WebView,
        notify: impl Fn(BrowserHealthEvent) + 'static,
    ) -> Result<Self, String> {
        let attach = || -> webview2_core::Result<Self> {
            let core = unsafe { view.controller().CoreWebView2()? };
            let environment = unsafe { core.cast::<ICoreWebView2_2>()?.Environment()? }
                .cast::<ICoreWebView2Environment5>()?;
            let mut monitor = Self {
                core,
                environment,
                listening: Rc::new(Cell::new(true)),
                process_token: None,
                exit_token: None,
                update_token: None,
            };
            let notify: Rc<dyn Fn(BrowserHealthEvent)> = Rc::new(notify);
            let failure_notify = Rc::clone(&notify);
            let failure_listening = Rc::clone(&monitor.listening);
            let mut token = 0;
            unsafe {
                monitor.core.add_ProcessFailed(
                    &ProcessFailedEventHandler::create(Box::new(move |_, arguments| {
                        if !failure_listening.get() {
                            return Ok(());
                        }
                        let kind = arguments.and_then(|arguments| {
                            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                            arguments.ProcessFailedKind(&mut kind).ok().map(|_| kind)
                        });
                        if let Some(failure) = failure_requiring_shutdown(kind) {
                            failure_notify(BrowserHealthEvent::Failed(failure));
                        }
                        Ok(())
                    })),
                    &mut token,
                )?;
            }
            monitor.process_token = Some(token);
            let exit_notify = Rc::clone(&notify);
            let exit_listening = Rc::clone(&monitor.listening);
            unsafe {
                monitor.environment.add_BrowserProcessExited(
                    &BrowserProcessExitedEventHandler::create(Box::new(move |_, _| {
                        if exit_listening.get() {
                            exit_notify(BrowserHealthEvent::Failed(BrowserFailure::BrowserExited));
                        }
                        Ok(())
                    })),
                    &mut token,
                )?;
            }
            monitor.exit_token = Some(token);
            let update_listening = Rc::clone(&monitor.listening);
            unsafe {
                monitor.environment.add_NewBrowserVersionAvailable(
                    &NewBrowserVersionAvailableEventHandler::create(Box::new(move |_, _| {
                        if update_listening.get() {
                            notify(BrowserHealthEvent::UpdateAvailable);
                        }
                        Ok(())
                    })),
                    &mut token,
                )?;
            }
            monitor.update_token = Some(token);
            Ok(monitor)
        };
        attach().map_err(|error| format!("Cannot monitor the browser engine: {error}"))
    }
}

impl Drop for BrowserHealthMonitor {
    fn drop(&mut self) {
        self.listening.set(false);
        // A crashed engine can reject unsubscription; no callback may outlive our listening flag.
        unsafe {
            if let Some(token) = self.process_token.take() {
                let _ = self.core.remove_ProcessFailed(token);
            }
            if let Some(token) = self.exit_token.take() {
                let _ = self.environment.remove_BrowserProcessExited(token);
            }
            if let Some(token) = self.update_token.take() {
                let _ = self.environment.remove_NewBrowserVersionAvailable(token);
            }
        }
    }
}

/// Only failures documented as automatically recoverable are left to the runtime.
fn failure_requiring_shutdown(
    kind: Option<COREWEBVIEW2_PROCESS_FAILED_KIND>,
) -> Option<BrowserFailure> {
    match kind {
        Some(COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED)
        | Some(COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED) => None,
        Some(COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED) => {
            Some(BrowserFailure::BrowserExited)
        }
        Some(COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED) => {
            Some(BrowserFailure::RendererExited)
        }
        Some(COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE) => {
            Some(BrowserFailure::RendererUnresponsive)
        }
        Some(COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED) => {
            Some(BrowserFailure::FrameRendererExited)
        }
        _ => Some(BrowserFailure::Unexpected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_pages_and_unknown_failures_require_a_safe_exit() {
        for kind in [
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
            COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND(999),
        ] {
            assert!(failure_requiring_shutdown(Some(kind)).is_some());
        }
        assert_eq!(
            failure_requiring_shutdown(None),
            Some(BrowserFailure::Unexpected)
        );
    }

    #[test]
    fn auto_recovered_processes_do_not_interrupt_transactions() {
        for kind in [
            COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED,
        ] {
            assert_eq!(failure_requiring_shutdown(Some(kind)), None);
        }
    }
}
