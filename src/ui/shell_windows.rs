//! Native shell surfaces whose positions are independent of the browser's content.

use tao::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use tao::event_loop::EventLoopWindowTarget;
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Window, WindowBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsChild};

use super::kiosk::KioskEvent;
use crate::security::CaptureProtector;

pub(super) const TASKBAR_HEIGHT: f64 = super::kiosk::DESKTOP_TASKBAR_HEIGHT as f64;
pub(super) const LANGUAGE_PICKER_WIDTH: f64 = 340.0;
pub(super) const LANGUAGE_PICKER_HEIGHT: f64 = 340.0;
const PICKER_MARGIN: f64 = 8.0;

/// WebView2 moves focus into native child windows without deactivating its top-level popup.
fn contains_native_window(window: &Window, candidate: HWND) -> bool {
    if candidate.is_invalid() {
        return false;
    }
    let owner = HWND(window.hwnd() as *mut _);
    candidate == owner || unsafe { IsChild(owner, candidate).as_bool() }
}

/// Reads current HWND ownership instead of trusting focus notifications queued before a popup opens.
pub(super) fn owns_foreground_window(window: &Window) -> bool {
    contains_native_window(window, unsafe { GetForegroundWindow() })
}

/// Keeps the picker open across parent-to-WebView focus transfers and taskbar toggles.
pub(super) fn popup_focus_moved_elsewhere(popup: &Window, anchor: &Window) -> bool {
    let foreground = unsafe { GetForegroundWindow() };
    !foreground.is_invalid()
        && !contains_native_window(popup, foreground)
        && !contains_native_window(anchor, foreground)
}

/// Activates the top-level popup before WebView2 moves keyboard focus into its child.
pub(super) fn show_focused_popup(popup: &Window) {
    popup.set_visible(true);
    popup.set_focus();
}

/// Creates an owned shell window without adding another entry to the Windows taskbar.
pub(super) fn create_shell_window(
    target: &EventLoopWindowTarget<KioskEvent>,
    owner: &Window,
    title: &str,
    size: LogicalSize<f64>,
    capture_allowed: bool,
) -> Result<Window, String> {
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(size)
        .with_visible(false)
        .with_focused(false)
        .with_decorations(false)
        .with_resizable(false)
        .with_skip_taskbar(true)
        .with_undecorated_shadow(false)
        .with_always_on_top(true)
        .with_owner_window(owner.hwnd())
        .build(target)
        .map_err(|error| format!("Cannot create {title}: {error}"))?;
    if !capture_allowed {
        CaptureProtector::apply_protection(HWND(window.hwnd() as *mut _))?;
    }
    Ok(window)
}

/// Anchors the isolated taskbar to the monitor, regardless of the browser's restored size.
pub(super) fn position_taskbar(taskbar: &Window, browser: &Window) {
    let Some(monitor) = browser.current_monitor() else {
        return;
    };
    let origin = monitor.position();
    let size = monitor.size();
    let height = (TASKBAR_HEIGHT * monitor.scale_factor()).round() as u32;
    let position = PhysicalPosition::new(origin.x, origin.y + size.height as i32 - height as i32);
    set_geometry(taskbar, position, PhysicalSize::new(size.width, height));
}

/// Reserves only the area actually covered by the separate bottom bar.
pub(super) fn taskbar_overlap(browser: &Window, taskbar: &Window) -> f64 {
    let (Ok(browser_origin), Ok(bar_origin)) = (browser.inner_position(), taskbar.inner_position())
    else {
        return TASKBAR_HEIGHT;
    };
    bottom_overlap(
        browser_origin.y,
        browser.inner_size().height,
        bar_origin.y,
        taskbar.inner_size().height,
    ) as f64
        / browser.scale_factor()
}

/// Computes the footer overlap in physical pixels in O(1) time and space.
fn bottom_overlap(content_top: i32, content_height: u32, bar_top: i32, bar_height: u32) -> u32 {
    let content_bottom = i64::from(content_top) + i64::from(content_height);
    let bar_bottom = i64::from(bar_top) + i64::from(bar_height);
    (content_bottom.min(bar_bottom) - i64::from(content_top.max(bar_top))).max(0) as u32
}

/// Positions the picker above the taskbar, with bounds clamped to its current monitor.
pub(super) fn position_language_picker(picker: &Window, anchor: &Window) {
    let Some(monitor) = anchor.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    let monitor_origin = monitor.position();
    let monitor_size = monitor.size();
    let anchor_origin = anchor.inner_position().unwrap_or(monitor_origin);
    let anchor_size = anchor.inner_size();
    let margin = (PICKER_MARGIN * scale).round() as i32;
    let size = PhysicalSize::new(
        (LANGUAGE_PICKER_WIDTH * scale).round() as u32,
        (LANGUAGE_PICKER_HEIGHT * scale).round() as u32,
    );
    let right = (anchor_origin.x + anchor_size.width as i32 - margin)
        .min(monitor_origin.x + monitor_size.width as i32 - margin);
    let bottom = (anchor_origin.y + anchor_size.height as i32
        - (TASKBAR_HEIGHT * scale).round() as i32
        - margin)
        .min(monitor_origin.y + monitor_size.height as i32 - margin);
    let position = PhysicalPosition::new(
        (right - size.width as i32).max(monitor_origin.x),
        (bottom - size.height as i32).max(monitor_origin.y),
    );
    set_geometry(picker, position, size);
}

/// Avoids generating recursive resize events when a shell surface is already correctly placed.
fn set_geometry(window: &Window, position: PhysicalPosition<i32>, size: PhysicalSize<u32>) {
    if window.outer_position().ok() != Some(position) {
        window.set_outer_position(position);
    }
    if window.inner_size() != size {
        window.set_inner_size(size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tao::event::Event;
    use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
    use tao::platform::run_return::EventLoopExtRunReturn;
    use tao::platform::windows::EventLoopBuilderExtWindows;

    /// Flushes queued geometry notifications without showing either native test window.
    fn flush_window_events(event_loop: &mut EventLoop<KioskEvent>) {
        event_loop.run_return(|event, _, control_flow| {
            *control_flow = if matches!(event, Event::MainEventsCleared) {
                ControlFlow::Exit
            } else {
                ControlFlow::Poll
            };
        });
    }

    #[test]
    fn native_taskbar_stays_at_monitor_bottom_after_browser_restore_and_move() {
        let _native_test = crate::ui::NATIVE_WEBVIEW_TEST_LOCK.lock().unwrap();
        let mut event_loop = EventLoopBuilder::<KioskEvent>::with_user_event()
            .with_any_thread(true)
            .build();
        let browser = WindowBuilder::new()
            .with_title("SafeBrowse geometry test")
            .with_visible(false)
            .with_focused(false)
            .with_decorations(false)
            .build(&event_loop)
            .unwrap();
        let monitor = browser
            .current_monitor()
            .expect("native monitor unavailable");
        let monitor_origin = monitor.position();
        let monitor_size = monitor.size();
        let bar_height = (TASKBAR_HEIGHT * monitor.scale_factor()).round() as u32;
        let expected_position = PhysicalPosition::new(
            monitor_origin.x,
            monitor_origin.y + monitor_size.height as i32 - bar_height as i32,
        );
        let expected_size = PhysicalSize::new(monitor_size.width, bar_height);
        let taskbar = create_shell_window(
            &event_loop,
            &browser,
            "SafeBrowse taskbar geometry test",
            LogicalSize::new(400.0, TASKBAR_HEIGHT),
            true,
        )
        .unwrap();

        // Tao's fullscreen restoration may show a hidden window; equivalent native bounds keep this test invisible.
        browser.set_outer_position(monitor_origin);
        browser.set_inner_size(monitor_size);
        position_taskbar(&taskbar, &browser);
        flush_window_events(&mut event_loop);
        assert_eq!(taskbar.outer_position().unwrap(), expected_position);
        assert_eq!(taskbar.inner_size(), expected_size);
        assert!(taskbar_overlap(&browser, &taskbar) > 0.0);

        browser.set_inner_size(PhysicalSize::new(
            monitor_size.width / 2,
            monitor_size.height / 3,
        ));
        browser.set_outer_position(PhysicalPosition::new(
            monitor_origin.x + monitor_size.width as i32 / 8,
            monitor_origin.y + monitor_size.height as i32 / 8,
        ));
        position_taskbar(&taskbar, &browser);
        flush_window_events(&mut event_loop);
        assert_eq!(taskbar.outer_position().unwrap(), expected_position);
        assert_eq!(taskbar.inner_size(), expected_size);
        assert_eq!(taskbar_overlap(&browser, &taskbar), 0.0);

        browser.set_outer_position(PhysicalPosition::new(
            monitor_origin.x + monitor_size.width as i32 / 4,
            monitor_origin.y + monitor_size.height as i32 / 4,
        ));
        position_taskbar(&taskbar, &browser);
        flush_window_events(&mut event_loop);
        assert_eq!(taskbar.outer_position().unwrap(), expected_position);
        assert_eq!(taskbar.inner_size(), expected_size);
        assert!(!browser.is_visible());
        assert!(!taskbar.is_visible());
        drop(taskbar);
        drop(browser);
        drop(event_loop);
    }

    #[test]
    fn restored_browser_reserves_only_the_bottom_bar_intersection() {
        assert_eq!(bottom_overlap(0, 1080, 1034, 46), 46);
        assert_eq!(bottom_overlap(100, 600, 1034, 46), 0);
        assert_eq!(bottom_overlap(-1080, 1080, -46, 46), 46);
        assert_eq!(bottom_overlap(500, 550, 1034, 46), 16);
    }
}
