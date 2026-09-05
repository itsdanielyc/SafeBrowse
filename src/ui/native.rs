//! Native window bounds and desktop hotkey dispatch.
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use tao::event_loop::EventLoopProxy;
use windows::Win32::Foundation::{
    GetLastError, SetLastError, ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetSystemMetrics, GetWindowRect, IsIconic, IsZoomed, SetWindowLongPtrW,
    GWLP_WNDPROC, SM_CXSCREEN, SM_CYSCREEN, SWP_NOMOVE, SWP_NOSIZE, WINDOWPOS, WM_MOVING,
    WM_WINDOWPOSCHANGING, WNDPROC,
};

pub const MIN_VISIBLE_TOP_HEIGHT: i32 = 36;
pub const MIN_VISIBLE_SIDE_WIDTH: i32 = 60;
const MINIMIZED_COORDINATE_THRESHOLD: i32 = -10000;
static PREV_WNDPROC: AtomicIsize = AtomicIsize::new(0);
static KIOSK_PROXY: Mutex<Option<EventLoopProxy<super::kiosk::KioskEvent>>> = Mutex::new(None);

/// Restores the original window procedure before its owning window is destroyed.
pub(crate) struct WindowProcedureGuard {
    hwnd: HWND,
}
impl WindowProcedureGuard {
    /// Installs one subclass and reports Win32 errors before advertising working hotkey dispatch.
    pub(crate) fn install(
        hwnd: HWND,
        proxy: EventLoopProxy<super::kiosk::KioskEvent>,
    ) -> Result<Self, String> {
        if hwnd.is_invalid() {
            return Err("Cannot install controls on an invalid window".into());
        }
        let mut stored_proxy = KIOSK_PROXY
            .lock()
            .map_err(|_| "Native window proxy lock is unavailable")?;
        if stored_proxy.is_some() {
            return Err("Native browser controls are already installed".into());
        }
        let previous = unsafe {
            SetLastError(ERROR_SUCCESS);
            SetWindowLongPtrW(
                hwnd,
                GWLP_WNDPROC,
                clamped_browser_wndproc as *const () as isize,
            )
        };
        let error = unsafe { GetLastError() };
        if previous == 0 && error != ERROR_SUCCESS {
            return Err(format!(
                "Could not install native browser controls: Win32 error {}",
                error.0
            ));
        }
        PREV_WNDPROC.store(previous, Ordering::SeqCst);
        *stored_proxy = Some(proxy);
        Ok(Self { hwnd })
    }
}
impl Drop for WindowProcedureGuard {
    fn drop(&mut self) {
        let previous = PREV_WNDPROC.swap(0, Ordering::SeqCst);
        if previous != 0 {
            unsafe {
                SetWindowLongPtrW(self.hwnd, GWLP_WNDPROC, previous);
            }
        }
        if let Ok(mut proxy) = KIOSK_PROXY.lock() {
            *proxy = None;
        }
    }
}
/// Clamps a window's drag rectangle within the specified monitor boundaries.
///
/// Ensures the top of the window strictly remains in frame (accessible between screen_top
/// and screen_bottom - MIN_VISIBLE_TOP_HEIGHT), while the sides and bottom are allowed
/// to extend out of frame.
///
/// # Complexity
/// - Time: O(1)
/// - Space: O(1)
pub fn clamp_window_rect(
    r: &mut RECT,
    screen_left: i32,
    screen_top: i32,
    screen_right: i32,
    screen_bottom: i32,
) {
    let width = r.right - r.left;
    let height = r.bottom - r.top;

    // Top must remain in frame:
    // 1. Cannot be dragged above screen_top
    if r.top < screen_top {
        r.top = screen_top;
        r.bottom = screen_top + height;
    } else if r.top > screen_bottom - MIN_VISIBLE_TOP_HEIGHT {
        // 2. Cannot be dragged below the screen bottom / taskbar (top stays reachable)
        r.top = screen_bottom - MIN_VISIBLE_TOP_HEIGHT;
        r.bottom = r.top + height;
    }

    // Left and right can extend out of frame, but at least MIN_VISIBLE_SIDE_WIDTH remains on screen
    if r.right < screen_left + MIN_VISIBLE_SIDE_WIDTH {
        r.right = screen_left + MIN_VISIBLE_SIDE_WIDTH;
        r.left = r.right - width;
    } else if r.left > screen_right - MIN_VISIBLE_SIDE_WIDTH {
        r.left = screen_right - MIN_VISIBLE_SIDE_WIDTH;
        r.right = r.left + width;
    }
    // Bottom is allowed out of frame freely
}

/// Clamps window position during `WM_WINDOWPOSCHANGING`.
///
/// # Complexity
/// - Time: O(1)
/// - Space: O(1)
pub fn clamp_window_pos(
    pos: &mut WINDOWPOS,
    screen_left: i32,
    screen_top: i32,
    screen_right: i32,
    screen_bottom: i32,
) {
    // Preserve minimized iconic coordinates (-32000, -32000)
    if pos.x <= MINIMIZED_COORDINATE_THRESHOLD || pos.y <= MINIMIZED_COORDINATE_THRESHOLD {
        return;
    }

    // Top must remain in frame
    if pos.y < screen_top {
        pos.y = screen_top;
    } else if pos.y > screen_bottom - MIN_VISIBLE_TOP_HEIGHT {
        pos.y = screen_bottom - MIN_VISIBLE_TOP_HEIGHT;
    }

    // Left and right can extend out of frame, but at least MIN_VISIBLE_SIDE_WIDTH remains on screen
    let cx = if pos.cx > 0 {
        pos.cx
    } else {
        MIN_VISIBLE_SIDE_WIDTH
    };
    if pos.x + cx < screen_left + MIN_VISIBLE_SIDE_WIDTH {
        pos.x = screen_left + MIN_VISIBLE_SIDE_WIDTH - cx;
    } else if pos.x > screen_right - MIN_VISIBLE_SIDE_WIDTH {
        pos.x = screen_right - MIN_VISIBLE_SIDE_WIDTH;
    }
    // Bottom is allowed out of frame freely
}

/// Selects the destination from the reachable title bar, not the window's old monitor.
/// Time and space: O(1), excluding the supplied operating-system monitor lookup.
fn clamp_proposed_window_rect(
    proposed: &mut RECT,
    previous: Option<&RECT>,
    monitor_bounds: impl FnOnce(POINT) -> RECT,
) {
    let width = proposed.right.saturating_sub(proposed.left).max(1);
    // The leading edge must cross first; using the whole window traps upward drags,
    // while always using its top traps downward drags at the last reachable title bar.
    let titlebar_y = if previous.is_some_and(|rect| proposed.top > rect.top) {
        proposed.top.saturating_add(MIN_VISIBLE_TOP_HEIGHT - 1)
    } else {
        proposed.top
    };
    let bounds = monitor_bounds(POINT {
        x: proposed.left.saturating_add(width / 2),
        y: titlebar_y,
    });
    clamp_window_rect(
        proposed,
        bounds.left,
        bounds.top,
        bounds.right,
        bounds.bottom,
    );
}

/// Resolves ignored WINDOWPOS dimensions before selecting a destination monitor.
/// Time and space: O(1).
fn proposed_position_rect(position: &WINDOWPOS, previous: Option<&RECT>) -> RECT {
    let keep_size = position.flags.0 & SWP_NOSIZE.0 != 0;
    let width = if keep_size || position.cx <= 0 {
        previous
            .map(|rect| rect.right.saturating_sub(rect.left))
            .unwrap_or(MIN_VISIBLE_SIDE_WIDTH)
    } else {
        position.cx
    }
    .max(1);
    let height = if keep_size || position.cy <= 0 {
        previous
            .map(|rect| rect.bottom.saturating_sub(rect.top))
            .unwrap_or(MIN_VISIBLE_TOP_HEIGHT)
    } else {
        position.cy
    }
    .max(1);
    RECT {
        left: position.x,
        top: position.y,
        right: position.x.saturating_add(width),
        bottom: position.y.saturating_add(height),
    }
}

/// Preserves Windows' minimized coordinates and messages that do not request a move.
fn position_needs_clamping(position: &WINDOWPOS, minimized: bool) -> bool {
    position.flags.0 & SWP_NOMOVE.0 == 0
        && !minimized
        && position.x > MINIMIZED_COORDINATE_THRESHOLD
        && position.y > MINIMIZED_COORDINATE_THRESHOLD
}

/// Falls back to the current monitor only when Windows cannot resolve the proposed title bar.
unsafe fn destination_monitor_bounds(hwnd: HWND, titlebar: POINT) -> RECT {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let destination = MonitorFromPoint(titlebar, MONITOR_DEFAULTTONEAREST);
    if GetMonitorInfoW(destination, &mut info).as_bool() {
        return info.rcMonitor;
    }
    let current = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    if GetMonitorInfoW(current, &mut info).as_bool() {
        return info.rcMonitor;
    }
    RECT {
        left: 0,
        top: 0,
        right: GetSystemMetrics(SM_CXSCREEN),
        bottom: GetSystemMetrics(SM_CYSCREEN),
    }
}

/// Subclassed Win32 window procedure that clamps window movements to the monitor frame
/// and intercepts defensive hotkeys.
///
/// Keeps a reachable part of the title bar on the destination monitor while allowing
/// the browser's sides and bottom to extend beyond it.
///
/// # Complexity
/// - Time: O(1)
/// - Space: O(1)
unsafe extern "system" fn clamped_browser_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    const WM_HOTKEY: u32 = 0x0312;
    if msg == WM_HOTKEY {
        let hotkey_id = wparam.0 as i32;
        if hotkey_id == crate::security::HOTKEY_SWITCH_DESKTOP_ID {
            if let Ok(guard) = KIOSK_PROXY.lock() {
                if let Some(ref p) = *guard {
                    let _ = p.send_event(super::kiosk::KioskEvent::SwitchDesktop);
                }
            }
            return LRESULT(0);
        } else if hotkey_id == crate::security::HOTKEY_PRINTSCREEN_ID {
            return LRESULT(0);
        }
    }

    // Most native messages do not move the window; avoid a monitor query on every event.
    if msg != WM_MOVING && msg != WM_WINDOWPOSCHANGING {
        return forward_window_message(hwnd, msg, wparam, lparam);
    }

    // Windows owns maximized frame coordinates; clamping its negative borders shifts the work area.
    if IsZoomed(hwnd).as_bool() {
        return forward_window_message(hwnd, msg, wparam, lparam);
    }

    if msg == WM_MOVING {
        let rect_ptr = lparam.0 as *mut RECT;
        if !rect_ptr.is_null() {
            let mut previous = RECT::default();
            let previous = GetWindowRect(hwnd, &mut previous).ok().map(|_| previous);
            clamp_proposed_window_rect(&mut *rect_ptr, previous.as_ref(), |point| {
                destination_monitor_bounds(hwnd, point)
            });
            return LRESULT(1);
        }
    } else if msg == WM_WINDOWPOSCHANGING {
        let pos_ptr = lparam.0 as *mut WINDOWPOS;
        if !pos_ptr.is_null() {
            let pos = &mut *pos_ptr;
            if position_needs_clamping(pos, IsIconic(hwnd).as_bool()) {
                let mut previous = RECT::default();
                let previous = GetWindowRect(hwnd, &mut previous).ok().map(|_| previous);
                let mut proposed = proposed_position_rect(pos, previous.as_ref());
                clamp_proposed_window_rect(&mut proposed, previous.as_ref(), |point| {
                    destination_monitor_bounds(hwnd, point)
                });
                pos.x = proposed.left;
                pos.y = proposed.top;
            }
        }
    }

    forward_window_message(hwnd, msg, wparam, lparam)
}

/// Forwards unhandled messages to Tao without recursing through the installed subclass.
unsafe fn forward_window_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let prev = PREV_WNDPROC.load(Ordering::SeqCst);
    if prev != 0 {
        let prev_fn: WNDPROC = std::mem::transmute(prev);
        CallWindowProcW(prev_fn, hwnd, msg, wparam, lparam)
    } else {
        windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR_WIDTH: i32 = 1920;
    const MONITOR_HEIGHT: i32 = 1080;
    const BROWSER_WIDTH: i32 = 800;
    const BROWSER_HEIGHT: i32 = 600;

    fn browser_rect(left: i32, top: i32) -> RECT {
        RECT {
            left,
            top,
            right: left + BROWSER_WIDTH,
            bottom: top + BROWSER_HEIGHT,
        }
    }

    /// Simulates adjacent displays without creating windows or changing the user's display setup.
    fn stacked_monitor_bounds(point: POINT) -> RECT {
        let top = point.y.div_euclid(MONITOR_HEIGHT) * MONITOR_HEIGHT;
        RECT {
            left: 0,
            top,
            right: MONITOR_WIDTH,
            bottom: top + MONITOR_HEIGHT,
        }
    }

    #[test]
    fn proposed_titlebar_can_cross_upward_before_most_of_the_browser() {
        let previous = browser_rect(100, 0);
        let mut proposed = browser_rect(100, -1);
        clamp_proposed_window_rect(&mut proposed, Some(&previous), stacked_monitor_bounds);
        assert_eq!(proposed.top, -MIN_VISIBLE_TOP_HEIGHT);
        assert_eq!(proposed.bottom - proposed.top, BROWSER_HEIGHT);
        assert_eq!(proposed.left, 100);

        let previous = proposed;
        proposed.top -= 1;
        proposed.bottom -= 1;
        clamp_proposed_window_rect(&mut proposed, Some(&previous), stacked_monitor_bounds);
        assert_eq!(proposed.top, -MIN_VISIBLE_TOP_HEIGHT - 1);
    }

    #[test]
    fn proposed_titlebar_can_cross_downward_from_last_reachable_position() {
        let previous = browser_rect(100, MONITOR_HEIGHT - MIN_VISIBLE_TOP_HEIGHT);
        let mut proposed = browser_rect(100, previous.top + 1);
        clamp_proposed_window_rect(&mut proposed, Some(&previous), stacked_monitor_bounds);
        assert_eq!(proposed.top, MONITOR_HEIGHT);
        assert_eq!(proposed.bottom - proposed.top, BROWSER_HEIGHT);

        let previous = proposed;
        proposed.top += 1;
        proposed.bottom += 1;
        clamp_proposed_window_rect(&mut proposed, Some(&previous), stacked_monitor_bounds);
        assert_eq!(proposed.top, MONITOR_HEIGHT + 1);
    }

    #[test]
    fn proposed_titlebar_selects_left_monitor_instead_of_the_previous_monitor() {
        let previous = browser_rect(-BROWSER_WIDTH / 2 + 1, 100);
        let mut proposed = browser_rect(-BROWSER_WIDTH / 2 - 1, 100);
        clamp_proposed_window_rect(&mut proposed, Some(&previous), |point| {
            assert_eq!(point.x, -1);
            RECT {
                left: -MONITOR_WIDTH,
                top: 0,
                right: 0,
                bottom: MONITOR_HEIGHT,
            }
        });
        assert_eq!(proposed.left, -BROWSER_WIDTH / 2 - 1);
        assert_eq!(proposed.top, 100);
    }

    #[test]
    fn no_size_programmatic_move_uses_real_dimensions_for_destination() {
        let previous = browser_rect(100, 100);
        let position = WINDOWPOS {
            x: -MONITOR_WIDTH + 100,
            y: -MONITOR_HEIGHT + 100,
            cx: 0,
            cy: 0,
            flags: SWP_NOSIZE,
            ..Default::default()
        };
        let mut proposed = proposed_position_rect(&position, Some(&previous));
        clamp_proposed_window_rect(&mut proposed, Some(&previous), |point| {
            assert_eq!(point.x, position.x + BROWSER_WIDTH / 2);
            assert_eq!(point.y, position.y);
            RECT {
                left: -MONITOR_WIDTH,
                top: -MONITOR_HEIGHT,
                right: 0,
                bottom: 0,
            }
        });
        assert_eq!(proposed.left, position.x);
        assert_eq!(proposed.top, position.y);
        assert_eq!(proposed.right - proposed.left, BROWSER_WIDTH);
        assert_eq!(proposed.bottom - proposed.top, BROWSER_HEIGHT);
        assert_eq!((position.cx, position.cy), (0, 0));
    }

    #[test]
    fn resize_move_uses_requested_dimensions_for_destination() {
        let previous = browser_rect(100, 100);
        let position = WINDOWPOS {
            x: MONITOR_WIDTH + 100,
            y: 100,
            cx: BROWSER_WIDTH * 2,
            cy: BROWSER_HEIGHT / 2,
            ..Default::default()
        };
        let mut proposed = proposed_position_rect(&position, Some(&previous));
        clamp_proposed_window_rect(&mut proposed, Some(&previous), |point| {
            assert_eq!(point.x, position.x + position.cx / 2);
            RECT {
                left: MONITOR_WIDTH,
                top: 0,
                right: MONITOR_WIDTH * 2,
                bottom: MONITOR_HEIGHT,
            }
        });
        assert_eq!(proposed.left, position.x);
        assert_eq!(proposed.top, position.y);
        assert_eq!(proposed.right - proposed.left, position.cx);
        assert_eq!(proposed.bottom - proposed.top, position.cy);
    }

    #[test]
    fn off_display_destination_retains_a_reachable_titlebar() {
        let previous = browser_rect(100, 100);
        for (left, top) in [(-4000, -3000), (4000, 3000)] {
            let mut proposed = browser_rect(left, top);
            clamp_proposed_window_rect(&mut proposed, Some(&previous), |_| RECT {
                left: 0,
                top: 0,
                right: MONITOR_WIDTH,
                bottom: MONITOR_HEIGHT,
            });
            assert!(proposed.top >= 0);
            assert!(proposed.top + MIN_VISIBLE_TOP_HEIGHT <= MONITOR_HEIGHT);
            assert!(proposed.right >= MIN_VISIBLE_SIDE_WIDTH);
            assert!(proposed.left <= MONITOR_WIDTH - MIN_VISIBLE_SIDE_WIDTH);
            assert_eq!(proposed.right - proposed.left, BROWSER_WIDTH);
            assert_eq!(proposed.bottom - proposed.top, BROWSER_HEIGHT);
        }
    }

    #[test]
    fn minimized_and_no_move_messages_do_not_select_a_destination() {
        let mut position = WINDOWPOS {
            x: 100,
            y: 100,
            flags: SWP_NOMOVE,
            ..Default::default()
        };
        assert!(!position_needs_clamping(&position, false));
        position.flags = SWP_NOSIZE;
        assert!(!position_needs_clamping(&position, true));
        assert!(position_needs_clamping(&position, false));
        position.x = -32000;
        position.y = -32000;
        assert!(!position_needs_clamping(&position, false));
    }
}
