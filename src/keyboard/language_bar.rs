//! Hides duplicate Windows input indicators on SafeBrowse's dedicated desktop.
//!
//! TSF can host its indicators on another thread or in another process. The
//! desktop, rather than the browser UI thread, is therefore the scope. This
//! guard changes window visibility only; it never changes Windows language-bar
//! preferences or disables text services, composition, or candidate windows.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::OnceLock;

use windows::core::BOOL;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::StationsAndDesktops::{
    EnumDesktopWindows, GetThreadDesktop, GetUserObjectInformationW, HDESK, UOI_NAME,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, ShowWindowAsync,
    EVENT_OBJECT_SHOW, GA_ROOT, OBJID_WINDOW, SW_HIDE, SW_SHOWNOACTIVATE, WINEVENT_OUTOFCONTEXT,
};

#[cfg(test)]
use crate::config::SAFE_DESKTOP_NAME;

// Microsoft documents the TSF title in Ctfutb.h as TF_FLOATINGLANGBAR_WNDTITLEW:
// https://learn.microsoft.com/windows/win32/tsf/tf-floatinglangbar--constants
const FLOATING_LANGUAGE_BAR_TITLE: &str = "TF_FloatingLangBar_WndTitle";
// This shell class is a Windows compatibility match, not a documented TSF API.
// Keep it exact: broad IME/CoreWindow matches would also hide candidate lists.
const TRAY_INPUT_INDICATOR_CLASS: &str = "TrayInputIndicatorWClass";
// On alternate desktops TSF also creates this overlay and its indicator window.
// These are exact compatibility matches, not documented Windows API contracts.
// Reproduced by the hidden worker_authorization_probe WebView2 fixture; the
// kiosk supplies only its authenticated session desktop, never Default/Winlogon.
const ALTERNATE_DESKTOP_INPUT_OVERLAY_CLASS: &str = "UAC_InputIndicatorOverlayWnd";
const ALTERNATE_DESKTOP_INPUT_INDICATOR_CLASS: &str = "UAC Input Indicator";
const NATIVE_NAME_CAPACITY: usize = 256;
const MAX_HIDDEN_INDICATORS: usize = 32;
const WINDOW_OBJECT_CHILD_ID: i32 = 0;
// One spare character distinguishes the exact title from a truncated prefix match.
const NATIVE_TITLE_CAPACITY: usize = FLOATING_LANGUAGE_BAR_TITLE.len() + 2;
type ReadStoredWindowTitle = unsafe extern "system" fn(HWND, *mut u16, i32) -> i32;
static STORED_WINDOW_TITLE_READER: OnceLock<Option<ReadStoredWindowTitle>> = OnceLock::new();

thread_local! {
    // Out-of-context WinEvent callbacks run on the installing thread. A weak
    // reference prevents late callbacks from extending the session lifetime.
    static ACTIVE_INDICATOR_GUARD: RefCell<Weak<RefCell<IndicatorState>>> = RefCell::default();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IndicatorKind {
    FloatingLanguageBar,
    TrayInputIndicator,
    AlternateDesktopInputOverlay,
    AlternateDesktopInputIndicator,
}

#[derive(Debug, PartialEq, Eq)]
struct IndicatorIdentity {
    process_id: u32,
    thread_id: u32,
    window_class: String,
    kind: IndicatorKind,
}

struct IndicatorState {
    desktop_name: String,
    hidden: HashMap<isize, IndicatorIdentity>,
}

impl IndicatorState {
    /// Hides only visible, positively matched indicators on the scoped desktop.
    /// Expected O(1) time and storage per event; the restoration map is bounded.
    fn suppress(&mut self, window: HWND) {
        if !unsafe { IsWindowVisible(window) }.as_bool() {
            return;
        }
        let Some(identity) = indicator_identity(window, &self.desktop_name) else {
            return;
        };
        let key = window.0 as isize;
        let previously_recorded = self.hidden.get(&key) == Some(&identity);
        if self.hidden.len() >= MAX_HIDDEN_INDICATORS && !self.hidden.contains_key(&key) {
            return;
        }
        // A TSF window may belong to a different GUI queue. Waiting for that
        // queue synchronously could freeze our picker or browser controls.
        if !unsafe { ShowWindowAsync(window, SW_HIDE) }.as_bool() {
            return;
        }
        if !previously_recorded {
            eprintln!(
                "[SafeBrowse] Hiding duplicate input indicator: kind={:?}, process={}, thread={}, class={:?}",
                identity.kind, identity.process_id, identity.thread_id, identity.window_class
            );
            self.hidden.insert(key, identity);
        }
    }

    /// Discards destroyed/replaced windows so a long session cannot exhaust the map.
    /// O(n) time and O(1) extra space for the bounded set of n recorded indicators.
    fn discard_stale_windows(&mut self) {
        self.hidden.retain(|handle, identity| {
            indicator_identity(HWND(*handle as *mut _), &self.desktop_name).as_ref()
                == Some(identity)
        });
    }

    /// Restores only windows this guard hid, after rechecking identity and desktop.
    fn restore(&mut self) {
        for (handle, identity) in self.hidden.drain() {
            let window = HWND(handle as *mut _);
            if indicator_identity(window, &self.desktop_name).as_ref() == Some(&identity) {
                let _ = unsafe { ShowWindowAsync(window, SW_SHOWNOACTIVATE) };
            }
        }
    }
}

/// Thread-bound visibility suppression with no persistent Windows settings.
///
/// Existing and newly shown indicators are handled across the dedicated desktop,
/// including TSF helper processes. The ordinary desktop is never enumerated.
/// Drop removes the hook and restores surviving indicators without activating them.
/// A forced process exit cannot restore visibility, but cannot change a window
/// on the ordinary desktop or a user's persistent language-bar preference.
pub struct ScopedLanguageBarGuard {
    hook: HWINEVENTHOOK,
    state: Rc<RefCell<IndicatorState>>,
}

impl ScopedLanguageBarGuard {
    /// Installs on the authenticated session desktop before creating any WebViews.
    pub fn install_for_current_thread(expected_desktop_name: &str) -> Result<Self, String> {
        Self::install_on_desktop(expected_desktop_name)
    }

    /// Keeps the desktop invariant explicit; tests use an unswitched private desktop.
    fn install_on_desktop(expected_desktop_name: &str) -> Result<Self, String> {
        named_thread_desktop(unsafe { GetCurrentThreadId() }, expected_desktop_name)?;
        if ACTIVE_INDICATOR_GUARD.with(|active| active.borrow().upgrade().is_some()) {
            return Err("Input indicator suppression is already installed on this thread".into());
        }
        let state = Rc::new(RefCell::new(IndicatorState {
            desktop_name: expected_desktop_name.to_owned(),
            hidden: HashMap::new(),
        }));
        // Microsoft scopes the zero PID/TID hook to the caller's current desktop:
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setwineventhook
        let hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_SHOW,
                EVENT_OBJECT_SHOW,
                None,
                Some(hide_shown_indicator),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if hook.is_invalid() {
            return Err("Cannot monitor the isolated desktop's input indicators".into());
        }
        ACTIVE_INDICATOR_GUARD.with(|active| *active.borrow_mut() = Rc::downgrade(&state));
        let mut guard = Self { hook, state };
        guard.refresh()?;
        Ok(guard)
    }

    /// Rescans after input-language changes as well as handling native show events.
    /// O(n + m) time for desktop windows n and bounded restoration records m.
    pub fn refresh(&mut self) -> Result<(), String> {
        let mut state = self.state.borrow_mut();
        let desktop = named_thread_desktop(unsafe { GetCurrentThreadId() }, &state.desktop_name)?;
        state.discard_stale_windows();
        unsafe {
            EnumDesktopWindows(
                Some(desktop),
                Some(inspect_indicator),
                LPARAM((&mut *state as *mut IndicatorState) as isize),
            )
        }
        .map_err(|error| format!("Cannot inspect SafeBrowse's input indicators: {error}"))
    }
}

impl Drop for ScopedLanguageBarGuard {
    fn drop(&mut self) {
        ACTIVE_INDICATOR_GUARD.with(|active| *active.borrow_mut() = Weak::new());
        if !unsafe { UnhookWinEvent(self.hook) }.as_bool() {
            eprintln!("[SafeBrowse] Could not remove the input-indicator visibility hook");
        }
        self.state.borrow_mut().restore();
    }
}

/// Returns a borrowed desktop handle only after checking its exact name.
fn named_thread_desktop(thread_id: u32, expected_name: &str) -> Result<HDESK, String> {
    let desktop = unsafe { GetThreadDesktop(thread_id) }
        .map_err(|error| format!("Cannot read the input thread's desktop: {error}"))?;
    let mut name = [0u16; NATIVE_NAME_CAPACITY];
    unsafe {
        GetUserObjectInformationW(
            HANDLE(desktop.0),
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            std::mem::size_of_val(&name) as u32,
            None,
        )
    }
    .map_err(|error| format!("Cannot identify the input thread's desktop: {error}"))?;
    let length = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    if !expected_name
        .encode_utf16()
        .eq(name[..length].iter().copied())
    {
        return Err("Input indicator suppression requires the isolated SafeBrowse desktop".into());
    }
    Ok(desktop)
}

/// Gets only native indicator metadata; ordinary window captions are not retained.
fn indicator_identity(window: HWND, desktop_name: &str) -> Option<IndicatorIdentity> {
    if window.is_invalid() || unsafe { GetAncestor(window, GA_ROOT) } != window {
        return None;
    }
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
    if thread_id == 0 || process_id == 0 {
        return None;
    }
    named_thread_desktop(thread_id, desktop_name).ok()?;
    let mut class_name = [0u16; NATIVE_NAME_CAPACITY];
    let class_length = unsafe { GetClassNameW(window, &mut class_name) }.max(0) as usize;
    let kind = [
        (
            TRAY_INPUT_INDICATOR_CLASS,
            IndicatorKind::TrayInputIndicator,
        ),
        (
            ALTERNATE_DESKTOP_INPUT_OVERLAY_CLASS,
            IndicatorKind::AlternateDesktopInputOverlay,
        ),
        (
            ALTERNATE_DESKTOP_INPUT_INDICATOR_CLASS,
            IndicatorKind::AlternateDesktopInputIndicator,
        ),
    ]
    .into_iter()
    .find_map(|(expected_class, kind)| {
        expected_class
            .encode_utf16()
            .eq(class_name[..class_length].iter().copied())
            .then_some(kind)
    })
    .or_else(|| {
        has_floating_language_bar_title(window).then_some(IndicatorKind::FloatingLanguageBar)
    })?;
    Some(IndicatorIdentity {
        process_id,
        thread_id,
        window_class: String::from_utf16_lossy(&class_name[..class_length]),
        kind,
    })
}

/// Reads stored native captions without dispatching messages to another GUI thread.
///
/// GetWindowText can hang on an unresponsive same-process window. The internal
/// caption reader avoids that dispatch. Microsoft reserves the right to remove
/// it, so resolve it optionally and stop matching TSF captions if unavailable.
/// https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-internalgetwindowtext
fn stored_window_title_reader() -> Option<ReadStoredWindowTitle> {
    *STORED_WINDOW_TITLE_READER.get_or_init(|| {
        let reader = unsafe {
            GetModuleHandleW(windows::core::w!("user32.dll"))
                .ok()
                .and_then(|module| GetProcAddress(module, windows::core::s!("InternalGetWindowText")))
                // The documented User32 export uses the exact Win32 signature above.
                .map(|function| std::mem::transmute::<unsafe extern "system" fn() -> isize, ReadStoredWindowTitle>(function))
        };
        if reader.is_none() {
            eprintln!("[SafeBrowse] Native TSF caption lookup is unavailable; floating indicators may remain visible");
        }
        reader
    })
}

/// Uses Microsoft's exact stored title, rejecting localized labels and prefixes.
fn has_floating_language_bar_title(window: HWND) -> bool {
    let expected_length = FLOATING_LANGUAGE_BAR_TITLE.len();
    let Some(read_title) = stored_window_title_reader() else {
        return false;
    };
    let mut title = [0u16; NATIVE_TITLE_CAPACITY];
    let length = unsafe { read_title(window, title.as_mut_ptr(), title.len() as i32) };
    length == expected_length as i32
        && FLOATING_LANGUAGE_BAR_TITLE
            .encode_utf16()
            .eq(title[..expected_length].iter().copied())
}

/// Out-of-context hooks execute on the installing thread without injecting a DLL.
unsafe extern "system" fn hide_shown_indicator(
    _hook: HWINEVENTHOOK,
    event: u32,
    window: HWND,
    object_id: i32,
    child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event != EVENT_OBJECT_SHOW
        || object_id != OBJID_WINDOW.0
        || child_id != WINDOW_OBJECT_CHILD_ID
    {
        return;
    }
    let state = ACTIVE_INDICATOR_GUARD.with(|active| active.borrow().upgrade());
    if let Some(state) = state {
        // WinEvent callbacks may reenter while native visibility messages run.
        if let Ok(mut state) = state.try_borrow_mut() {
            state.suppress(window);
        }
    }
}

/// EnumDesktopWindows runs synchronously, so the borrowed state outlives callbacks.
unsafe extern "system" fn inspect_indicator(window: HWND, context: LPARAM) -> BOOL {
    (&mut *(context.0 as *mut IndicatorState)).suppress(window);
    true.into()
}

#[cfg(test)]
mod tests;
