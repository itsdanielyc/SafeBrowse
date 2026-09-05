//! Moves one trusted keyboard view between the browser and an owned native window.

use std::cell::Cell;
use tao::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use tao::event_loop::EventLoopWindowTarget;
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Window, WindowBuilder, WindowId};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_LBUTTON};
use wry::{Rect, WebViewExtWindows};

use super::kiosk::{make_rect, KioskEvent, BROWSER_OSK_HEIGHT, DESKTOP_TASKBAR_HEIGHT};
use super::trusted::TrustedWebView;
use crate::security::CaptureProtector;

const FLOATING_WIDTH: f64 = 920.0;
const MONITOR_MARGIN: f64 = 8.0;

/// The view remains owned by the session and must be destroyed before this native parent.
pub(super) struct FloatingKeyboard {
    window: Window,
    detached: bool,
    positioned: bool,
    dragging: Cell<bool>,
}

impl FloatingKeyboard {
    /// Creates an invisible, app-owned window on the caller's session desktop.
    pub(super) fn new(
        target: &EventLoopWindowTarget<KioskEvent>,
        owner: &Window,
        capture_allowed: bool,
    ) -> Result<Self, String> {
        let window = WindowBuilder::new()
            .with_title("SafeBrowse on-screen keyboard")
            .with_inner_size(LogicalSize::new(FLOATING_WIDTH, BROWSER_OSK_HEIGHT))
            .with_visible(false)
            .with_focused(false)
            .with_decorations(false)
            .with_resizable(false)
            .with_skip_taskbar(true)
            .with_owner_window(owner.hwnd())
            .build(target)
            .map_err(|error| format!("Cannot create floating keyboard: {error}"))?;
        // Protection belongs to the top-level HWND, including while its child is being reparented.
        if !capture_allowed {
            CaptureProtector::apply_protection(HWND(window.hwnd() as *mut _))?;
        }
        Ok(Self {
            window,
            detached: false,
            positioned: false,
            dragging: Cell::new(false),
        })
    }

    pub(super) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(super) fn is_detached(&self) -> bool {
        self.detached
    }

    /// Reparents the existing view without reloading its document or changing its editing target.
    pub(super) fn set_detached(
        &mut self,
        detached: bool,
        view: &TrustedWebView,
        browser: &Window,
    ) -> Result<(), String> {
        if self.detached == detached {
            return Ok(());
        }
        if detached {
            self.constrain_geometry(browser, !self.positioned);
        }
        let parent = if detached { &self.window } else { browser };
        view.reparent(parent.hwnd())
            .map_err(|error| format!("Cannot move on-screen keyboard: {error}"))?;
        self.detached = detached;
        self.dragging.set(false);
        self.positioned |= detached;
        self.window.set_visible(false);
        self.sync_controls(view)
    }

    /// Native state controls the Attach/Detach label; bundled JavaScript cannot choose a parent.
    pub(super) fn sync_controls(&self, view: &TrustedWebView) -> Result<(), String> {
        view.evaluate_script(&format!("window.setKeyboardDetached?.({});", self.detached))
            .map_err(|error| error.to_string())
    }

    /// Hides the owned window before changing controller visibility to avoid focus stealing.
    pub(super) fn layout(
        &self,
        view: &TrustedWebView,
        browser: &Window,
        docked_bounds: Rect,
        visible: bool,
    ) -> wry::Result<()> {
        let visible = visible && !browser.is_minimized();
        let floating_visible = visible && self.detached;
        if !floating_visible && self.window.is_visible() {
            self.window.set_visible(false);
        }
        let bounds = if self.detached {
            self.constrain_geometry(browser, false);
            let size = self
                .window
                .inner_size()
                .to_logical::<f64>(self.window.scale_factor());
            make_rect(0.0, 0.0, size.width, size.height)
        } else {
            docked_bounds
        };
        view.set_bounds(bounds)?;
        view.set_visible(visible)?;
        if floating_visible && !self.window.is_visible() {
            self.window.set_visible(true);
        }
        Ok(())
    }

    /// Starts the standard Windows drag loop only for the detached bundled header.
    pub(super) fn start_drag(&self) -> Result<(), String> {
        if self.detached && self.window.is_visible() {
            self.window
                .drag_window()
                .map_err(|error| format!("Cannot move floating keyboard: {error}"))?;
            self.dragging.set(true);
        }
        Ok(())
    }

    /// Tao posts the native drag request asynchronously; defer clamping until its mouse loop ends.
    pub(super) fn finish_drag_if_released(&self) -> bool {
        if self.dragging.get() && unsafe { GetKeyState(i32::from(VK_LBUTTON.0)) } >= 0 {
            self.dragging.set(false);
            return true;
        }
        false
    }

    /// Resizes for the destination monitor's DPI and keeps every key above the session taskbar.
    fn constrain_geometry(&self, browser: &Window, initial: bool) {
        if self.dragging.get() && !initial {
            // Clamping mid-drag would trap a window on the source monitor before it crosses the edge.
            return;
        }
        let monitor = if initial {
            browser.current_monitor()
        } else {
            self.window
                .current_monitor()
                .or_else(|| browser.current_monitor())
        };
        let Some(monitor) = monitor else { return };
        let origin = monitor.position();
        let monitor_size = monitor.size();
        let scale = monitor.scale_factor();
        let margin = (MONITOR_MARGIN * scale).round() as u32;
        let footer = (f64::from(DESKTOP_TASKBAR_HEIGHT) * scale).round() as u32;
        let available = PhysicalSize::new(
            monitor_size.width.saturating_sub(margin * 2).max(1),
            monitor_size
                .height
                .saturating_sub(footer + margin * 2)
                .max(1),
        );
        let size = PhysicalSize::new(
            ((FLOATING_WIDTH * scale).round() as u32).min(available.width),
            ((BROWSER_OSK_HEIGHT * scale).round() as u32).min(available.height),
        );
        let top_left = PhysicalPosition::new(origin.x + margin as i32, origin.y + margin as i32);
        let proposed = if initial {
            PhysicalPosition::new(
                top_left.x + (available.width - size.width) as i32 / 2,
                top_left.y + (available.height - size.height) as i32,
            )
        } else {
            self.window.outer_position().unwrap_or(top_left)
        };
        let position = clamp_position(proposed, size, top_left, available);
        if self.window.inner_size() != size {
            self.window.set_inner_size(size);
        }
        if self.window.outer_position().ok() != Some(position) {
            self.window.set_outer_position(position);
        }
    }
}

/// Clamps in physical pixels, including monitors with negative origins. Time/space: O(1).
fn clamp_position(
    proposed: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    origin: PhysicalPosition<i32>,
    available: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    PhysicalPosition::new(
        proposed.x.clamp(
            origin.x,
            origin.x + available.width.saturating_sub(size.width) as i32,
        ),
        proposed.y.clamp(
            origin.y,
            origin.y + available.height.saturating_sub(size.height) as i32,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tao::event_loop::EventLoopBuilder;
    use tao::platform::windows::EventLoopBuilderExtWindows;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowDisplayAffinity, GetWindowLongPtrW, GetWindowThreadProcessId,
        GWL_EXSTYLE, GW_OWNER, WDA_EXCLUDEFROMCAPTURE, WS_EX_TOPMOST,
    };

    #[test]
    fn floating_parent_is_hidden_owned_and_capture_protected_before_use() {
        let _native_test = crate::ui::NATIVE_WEBVIEW_TEST_LOCK.lock().unwrap();
        let event_loop = EventLoopBuilder::<KioskEvent>::with_user_event()
            .with_any_thread(true)
            .build();
        let owner = WindowBuilder::new()
            .with_visible(false)
            .with_focused(false)
            .build(&event_loop)
            .unwrap();
        let keyboard = FloatingKeyboard::new(&event_loop, &owner, false).unwrap();
        let keyboard_hwnd = HWND(keyboard.window.hwnd() as *mut _);
        let owner_hwnd = HWND(owner.hwnd() as *mut _);
        let mut affinity = 0;
        unsafe {
            GetWindowDisplayAffinity(keyboard_hwnd, &mut affinity).unwrap();
            assert_eq!(GetWindow(keyboard_hwnd, GW_OWNER).unwrap(), owner_hwnd);
            assert_eq!(
                GetWindowThreadProcessId(keyboard_hwnd, None),
                GetWindowThreadProcessId(owner_hwnd, None),
                "both windows must belong to the same session UI thread and desktop"
            );
            assert_eq!(
                GetWindowLongPtrW(keyboard_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0,
                0,
                "the floating keyboard must not float over unrelated applications"
            );
        }
        assert_eq!(affinity, WDA_EXCLUDEFROMCAPTURE.0);
        assert!(!keyboard.window.is_visible());
        assert!(!owner.is_visible());
        let recording_keyboard = FloatingKeyboard::new(&event_loop, &owner, true).unwrap();
        unsafe {
            GetWindowDisplayAffinity(
                HWND(recording_keyboard.window.hwnd() as *mut _),
                &mut affinity,
            )
            .unwrap();
        }
        assert_eq!(
            affinity, 0,
            "only the explicit session flag permits recording"
        );
        assert!(!recording_keyboard.window.is_visible());
    }

    #[test]
    fn floating_keyboard_stays_inside_negative_origin_and_small_monitors() {
        let origin = PhysicalPosition::new(-1912, -1072);
        let available = PhysicalSize::new(1904, 1018);
        let size = PhysicalSize::new(920, 230);
        assert_eq!(
            clamp_position(PhysicalPosition::new(-4000, -3000), size, origin, available),
            origin
        );
        assert_eq!(
            clamp_position(PhysicalPosition::new(0, 0), size, origin, available),
            PhysicalPosition::new(-928, -284)
        );
        assert_eq!(
            clamp_position(
                PhysicalPosition::new(100, 100),
                size,
                origin,
                PhysicalSize::new(500, 200)
            ),
            origin
        );
    }
}
