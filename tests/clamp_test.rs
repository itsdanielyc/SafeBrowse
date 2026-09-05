//! Unit & Integration Tests for Window Clamping Logic
//!
//! Validates:
//! 1. Top of window must remain in frame (top >= screen_top and top <= screen_bottom - MIN_VISIBLE_TOP_HEIGHT).
//! 2. Left and right sides can extend out of frame (leaving at least MIN_VISIBLE_SIDE_WIDTH for titlebar access).
//! 3. Bottom of window can freely extend out of frame below monitor / taskbar boundaries.
//! 4. Multi-monitor environments with negative virtual screen coordinates (e.g. left = -1920).
//! 5. Minimized iconic coordinates (-32000, -32000) are NOT clamped, preserving Win32 minimize.

use safebrowse::ui::{
    clamp_window_pos, clamp_window_rect, MIN_VISIBLE_SIDE_WIDTH, MIN_VISIBLE_TOP_HEIGHT,
};
use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::{SET_WINDOW_POS_FLAGS, WINDOWPOS};

#[test]
fn test_clamp_drag_left_and_top_edges() {
    let mut rect = RECT {
        left: -120,
        top: -80,
        right: 880,
        bottom: 620,
    };
    clamp_window_rect(&mut rect, 0, 0, 1920, 1034);

    // Top must clamp to 0 so the user never loses the grab bar
    assert_eq!(rect.top, 0, "Top edge must clamp to 0");
    // Left side is allowed out of frame
    assert_eq!(rect.left, -120, "Left edge is allowed out of frame (< 0)");
    assert_eq!(rect.right, 880, "Right edge preserved");
    assert_eq!(
        rect.bottom, 700,
        "Height must be preserved: 620 - (-80) = 700"
    );
}

#[test]
fn test_clamp_drag_right_and_bottom_edges() {
    let mut rect = RECT {
        left: 1200,
        top: 600,
        right: 2200,  // 2200 > 1920
        bottom: 1100, // 1100 > 1034
    };
    clamp_window_rect(&mut rect, 0, 0, 1920, 1034);

    // Right and bottom are allowed out of frame
    assert_eq!(rect.left, 1200, "Left edge preserved");
    assert_eq!(rect.right, 2200, "Right edge allowed out of frame (> 1920)");
    assert_eq!(rect.top, 600, "Top edge remains in frame");
    assert_eq!(
        rect.bottom, 1100,
        "Bottom edge allowed out of frame (> 1034)"
    );
}

#[test]
fn test_clamp_drag_down_only_top_in_frame() {
    let mut rect = RECT {
        left: 100,
        top: 1050, // Pushed past screen_bottom (1034)
        right: 1100,
        bottom: 1750,
    };
    clamp_window_rect(&mut rect, 0, 0, 1920, 1034);

    let expected_top = 1034 - MIN_VISIBLE_TOP_HEIGHT;
    assert_eq!(
        rect.top, expected_top,
        "Top edge clamped so only top bar remains in frame"
    );
    assert_eq!(
        rect.bottom,
        expected_top + 700,
        "Bottom edge extends far out of frame"
    );
    assert_eq!(rect.left, 100);
    assert_eq!(rect.right, 1100);
}

#[test]
fn test_clamp_drag_excessive_side_offset() {
    // Window pushed almost completely off to the left
    let mut rect = RECT {
        left: -1200,
        top: 100,
        right: -200, // Entire window off screen
        bottom: 700,
    };
    clamp_window_rect(&mut rect, 0, 0, 1920, 1034);

    assert_eq!(
        rect.right, MIN_VISIBLE_SIDE_WIDTH,
        "Right edge clamped so titlebar remains grab-able"
    );
    assert_eq!(
        rect.left,
        MIN_VISIBLE_SIDE_WIDTH - 1000,
        "Width 1000 preserved"
    );
    assert_eq!(rect.top, 100);
}

#[test]
fn test_clamp_drag_excessive_right_side_offset() {
    // Window pushed almost completely off to the right
    let mut rect = RECT {
        left: 2500,
        top: 100,
        right: 3500, // Entire window off screen to the right
        bottom: 700,
    };
    clamp_window_rect(&mut rect, 0, 0, 1920, 1034);

    let expected_left = 1920 - MIN_VISIBLE_SIDE_WIDTH;
    assert_eq!(
        rect.left, expected_left,
        "Left edge clamped so at least MIN_VISIBLE_SIDE_WIDTH remains on screen"
    );
    assert_eq!(rect.right, expected_left + 1000, "Width 1000 preserved");
    assert_eq!(rect.top, 100);
}

#[test]
fn test_clamp_multi_monitor_negative_coordinates() {
    // Secondary monitor positioned to the left of the main monitor: [-1920, 0]
    let screen_left = -1920;
    let screen_top = 0;
    let screen_right = 0;
    let screen_bottom = 1034;

    let mut rect = RECT {
        left: -2100, // partially off screen to the left of secondary monitor
        top: -50,
        right: -1100,
        bottom: 650,
    };
    clamp_window_rect(
        &mut rect,
        screen_left,
        screen_top,
        screen_right,
        screen_bottom,
    );

    assert_eq!(rect.left, -2100, "Left edge allowed out of frame");
    assert_eq!(rect.top, 0, "Must clamp to top edge");
    assert_eq!(rect.right, -1100, "Right edge preserved");
    assert_eq!(rect.bottom, 700, "Height 700 preserved");
}

#[test]
fn test_clamp_window_pos_normal() {
    let mut pos = WINDOWPOS {
        hwnd: windows::Win32::Foundation::HWND(std::ptr::null_mut()),
        hwndInsertAfter: windows::Win32::Foundation::HWND(std::ptr::null_mut()),
        x: -50,
        y: -30,
        cx: 800,
        cy: 600,
        flags: SET_WINDOW_POS_FLAGS(0),
    };
    clamp_window_pos(&mut pos, 0, 0, 1920, 1034);

    assert_eq!(pos.y, 0, "Top clamped to 0");
    assert_eq!(pos.x, -50, "Side allowed out of frame");
    assert_eq!(pos.cx, 800);
    assert_eq!(pos.cy, 600);
}

#[test]
fn test_clamp_window_pos_minimized_iconic_preservation() {
    // When Win32 minimizes a window, coordinates are iconic (-32000, -32000).
    // Our clamping logic must NOT clamp -32000 to 0, or minimize would be broken.
    let mut pos = WINDOWPOS {
        hwnd: windows::Win32::Foundation::HWND(std::ptr::null_mut()),
        hwndInsertAfter: windows::Win32::Foundation::HWND(std::ptr::null_mut()),
        x: -32000,
        y: -32000,
        cx: 160,
        cy: 24,
        flags: SET_WINDOW_POS_FLAGS(0),
    };
    clamp_window_pos(&mut pos, 0, 0, 1920, 1034);

    assert_eq!(pos.x, -32000, "Iconic X must be preserved");
    assert_eq!(pos.y, -32000, "Iconic Y must be preserved");
}

#[test]
fn test_clamp_window_pos_zero_width_nosize() {
    // When SWP_NOSIZE is present, pos.cx can be 0.
    // Clamping must not jump the window to unexpected offsets.
    let mut pos = WINDOWPOS {
        hwnd: windows::Win32::Foundation::HWND(std::ptr::null_mut()),
        hwndInsertAfter: windows::Win32::Foundation::HWND(std::ptr::null_mut()),
        x: 100,
        y: 200,
        cx: 0,
        cy: 0,
        flags: SET_WINDOW_POS_FLAGS(0x0001), // SWP_NOSIZE
    };
    clamp_window_pos(&mut pos, 0, 0, 1920, 1034);

    assert_eq!(
        pos.x, 100,
        "Position X should be preserved when within screen boundaries"
    );
    assert_eq!(
        pos.y, 200,
        "Position Y should be preserved when within screen boundaries"
    );
}
