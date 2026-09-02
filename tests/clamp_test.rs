use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN, WINDOWPOS, WM_MOVING,
    WM_WINDOWPOSCHANGING, WNDPROC,
};

static PREV_WNDPROC: AtomicIsize = AtomicIsize::new(0);
const DESKTOP_TASKBAR_HEIGHT: i32 = 48;

unsafe extern "system" fn clamped_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_MOVING {
        let rect_ptr = lparam.0 as *mut RECT;
        if !rect_ptr.is_null() {
            let mut r = *rect_ptr;
            let width = r.right - r.left;
            let height = r.bottom - r.top;
            let screen_w = GetSystemMetrics(SM_CXSCREEN);
            let screen_h = GetSystemMetrics(SM_CYSCREEN);
            let bottom_limit = screen_h - DESKTOP_TASKBAR_HEIGHT;

            if r.left < 0 {
                r.left = 0;
                r.right = width;
            } else if r.right > screen_w {
                r.right = screen_w;
                r.left = screen_w - width;
            }

            if r.top < 0 {
                r.top = 0;
                r.bottom = height;
            } else if r.bottom > bottom_limit {
                r.bottom = bottom_limit;
                r.top = bottom_limit - height;
            }

            *rect_ptr = r;
            return LRESULT(1);
        }
    } else if msg == WM_WINDOWPOSCHANGING {
        let pos_ptr = lparam.0 as *mut WINDOWPOS;
        if !pos_ptr.is_null() {
            let pos = &mut *pos_ptr;
            const SWP_NOMOVE: u32 = 0x0002;
            if (pos.flags.0 & SWP_NOMOVE) == 0 {
                let screen_w = GetSystemMetrics(SM_CXSCREEN);
                let screen_h = GetSystemMetrics(SM_CYSCREEN);
                let bottom_limit = screen_h - DESKTOP_TASKBAR_HEIGHT;

                if pos.x < 0 {
                    pos.x = 0;
                } else if pos.x + pos.cx > screen_w {
                    pos.x = screen_w - pos.cx;
                }

                if pos.y < 0 {
                    pos.y = 0;
                } else if pos.y + pos.cy > bottom_limit {
                    pos.y = bottom_limit - pos.cy;
                }
            }
        }
    }

    let prev = PREV_WNDPROC.load(Ordering::SeqCst);
    if prev != 0 {
        let prev_fn: WNDPROC = std::mem::transmute(prev);
        CallWindowProcW(prev_fn, hwnd, msg, wparam, lparam)
    } else {
        windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

#[test]
fn test_clamping_logic() {
    let mut rect = RECT {
        left: -50,
        top: -30,
        right: 500,
        bottom: 400,
    };
    let lparam = LPARAM(&mut rect as *mut RECT as isize);
    unsafe {
        let res = clamped_wndproc(HWND(std::ptr::null_mut()), WM_MOVING, WPARAM(0), lparam);
        assert_eq!(res.0, 1);
        assert_eq!(rect.left, 0);
        assert_eq!(rect.top, 0);
        assert_eq!(rect.right, 550);
        assert_eq!(rect.bottom, 430);
    }
}
