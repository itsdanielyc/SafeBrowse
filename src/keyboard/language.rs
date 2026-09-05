//! Session-scoped Windows input languages and physical-layout character maps.
//!
//! Layout identifiers are opaque installed HKL handles, valid for this Windows
//! session. Selecting a layout changes SafeBrowse and its embedded input windows;
//! it never loads layouts, changes the system default, or writes language settings.

use std::collections::HashMap;

use serde::Serialize;
use windows::core::{Error, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Globalization::{
    GetLocaleInfoEx, LCIDToLocaleName, LOCALE_SISO639LANGNAME2, LOCALE_SLOCALIZEDDISPLAYNAME,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::Ime::ImmIsIME;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ActivateKeyboardLayout, GetKeyboardLayout, GetKeyboardLayoutList, MapVirtualKeyExW,
    ToUnicodeEx, HKL, KLF_SETFORPROCESS, MAPVK_VSC_TO_VK_EX, VK_CAPITAL, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetGUIThreadInfo, GetWindowThreadProcessId, IsChild, SendMessageTimeoutW,
    GUITHREADINFO, SMTO_ABORTIFHUNG, SMTO_BLOCK, SMTO_ERRORONEXIT, WM_INPUTLANGCHANGEREQUEST,
};

const LOCALE_NAME_CAPACITY: usize = 85;
const LOCALE_TEXT_CAPACITY: usize = 256;
const CHARACTER_BUFFER_CAPACITY: usize = 16;
const KEY_PRESSED: u8 = 0x80;
const KEY_TOGGLED: u8 = 0x01;
const PRESERVE_KEYBOARD_STATE: u32 = 1 << 2;
const INPUT_LANGUAGE_TIMEOUT_MS: u32 = 250;
const PHYSICAL_KEY_ROWS: [&[u32]; 4] = [
    &[
        0x29, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
    ],
    &[
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x2b,
    ],
    &[
        0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28,
    ],
    &[
        0x56, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35,
    ],
];

/// An installed input language, including its regional Windows display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputLanguage {
    pub id: String,
    pub label: String,
    pub code: String,
    /// IME composition requires physical input; DOM insertion cannot implement it.
    pub ime: bool,
}

/// A fresh view of installed languages and the relevant window's actual layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputLanguageState {
    pub active_id: String,
    pub layouts: Vec<InputLanguage>,
}

impl InputLanguageState {
    /// Returns the active installed language, if Windows still exposes it.
    pub fn active(&self) -> Option<&InputLanguage> {
        self.layouts
            .iter()
            .find(|layout| layout.id == self.active_id)
    }
}

/// Printable values for one physical key under the native Shift/Caps combinations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayoutKey {
    pub value: String,
    pub shifted_value: String,
    pub caps_value: String,
    pub shifted_caps_value: String,
    /// Dead keys are exposed as spacing accents, not simulated composition.
    pub dead_key: bool,
    pub shifted_dead_key: bool,
}

/// Reads current state without modifying input languages or keyboard state.
///
/// Call again after focus or WM_INPUTLANGCHANGE events rather than caching a
/// global Windows language: individual embedded input threads can differ.
pub fn snapshot(parent_window: HWND) -> Result<InputLanguageState, String> {
    let parent_thread = validate_parent_window(parent_window)?;
    let focused_thread = focused_descendant(parent_window, 0)
        .or_else(|| focused_descendant(parent_window, parent_thread))
        .map(|window| unsafe { GetWindowThreadProcessId(window, None) })
        .unwrap_or_else(|| remembered_input_thread(parent_window, parent_thread));
    let active_layout = unsafe { GetKeyboardLayout(focused_thread) };
    state_for_layout(active_layout)
}

/// Activates an installed layout only for SafeBrowse and its embedded windows.
///
/// The supplied parent must belong to this process. WebView input threads can
/// belong to a child browser process, so descendant windows receive the normal
/// Windows language-change request too. Requests are bounded to avoid hanging
/// the browser shell if an embedded process stops responding.
pub fn select(parent_window: HWND, layout_id: &str) -> Result<InputLanguageState, String> {
    validate_parent_window(parent_window)?;
    let installed = installed_layouts()?;
    let layout = resolve_layout(&installed, layout_id)?;
    let input_windows = descendant_input_threads(parent_window);

    unsafe { ActivateKeyboardLayout(layout, KLF_SETFORPROCESS) }
        .map_err(|error| format!("Could not activate the selected input language: {error}"))?;

    for (thread_id, window) in input_windows {
        let target = focused_descendant(parent_window, thread_id).unwrap_or(window);
        let mut result = 0usize;
        let delivered = unsafe {
            SendMessageTimeoutW(
                target,
                WM_INPUTLANGCHANGEREQUEST,
                WPARAM(0),
                LPARAM(layout.0 as isize),
                SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
                INPUT_LANGUAGE_TIMEOUT_MS,
                Some(&mut result),
            )
        };
        if delivered.0 == 0 {
            return Err("The input language changed, but a page did not respond. Refocus the page and try again.".into());
        }
        if unsafe { GetKeyboardLayout(thread_id) } != layout {
            return Err("A page did not accept the selected input language. Refocus the page and try again.".into());
        }
    }

    // The picker may own focus, so report the actual calling-process layout,
    // after embedded input threads have also confirmed the requested layout.
    state_for_layout(unsafe { GetKeyboardLayout(0) })
}

/// Generates four rows of printable keys using the installed physical layout.
///
/// Shift mappings include local punctuation and non-Latin letters. Dead keys
/// produce their spacing accent; IME composition and AltGr are not synthesized.
/// Translation preserves native dead-key state (Windows 10 1607 and newer).
/// Time: O(L + K). Space: O(L + K), for L layouts and K physical keys.
pub fn virtual_key_rows(layout_id: &str) -> Result<Vec<Vec<LayoutKey>>, String> {
    let layout = resolve_layout(&installed_layouts()?, layout_id)?;
    Ok(PHYSICAL_KEY_ROWS
        .iter()
        .map(|row| {
            row.iter()
                .filter_map(|&scan_code| {
                    let (value, dead_key) = translate_character(layout, scan_code, false, false);
                    let (shifted_value, shifted_dead_key) =
                        translate_character(layout, scan_code, true, false);
                    let (caps_value, _) = translate_character(layout, scan_code, false, true);
                    let (shifted_caps_value, _) =
                        translate_character(layout, scan_code, true, true);
                    if value.is_empty()
                        && shifted_value.is_empty()
                        && caps_value.is_empty()
                        && shifted_caps_value.is_empty()
                    {
                        return None;
                    }
                    Some(LayoutKey {
                        value,
                        shifted_value,
                        caps_value,
                        shifted_caps_value,
                        dead_key,
                        shifted_dead_key,
                    })
                })
                .collect()
        })
        .collect())
}

fn validate_parent_window(parent_window: HWND) -> Result<u32, String> {
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(parent_window, Some(&mut process_id)) };
    if thread_id == 0 || process_id != unsafe { GetCurrentProcessId() } {
        return Err("Input languages can only be changed for a SafeBrowse window.".into());
    }
    Ok(thread_id)
}

fn focused_descendant(parent_window: HWND, thread_id: u32) -> Option<HWND> {
    let mut state = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetGUIThreadInfo(thread_id, &mut state) }.is_err() {
        return None;
    }
    let focus = state.hwndFocus;
    (focus == parent_window || unsafe { IsChild(parent_window, focus) }.as_bool()).then_some(focus)
}

/// Retains the page's layout while a shell control temporarily owns global focus.
/// Time: O(W + T). Space: O(T), for W descendant windows and T input threads.
fn remembered_input_thread(parent_window: HWND, parent_thread: u32) -> u32 {
    let descendants = descendant_input_threads(parent_window);
    choose_input_thread(
        parent_thread,
        descendants.into_keys().map(|thread_id| {
            let has_local_focus = focused_descendant(parent_window, thread_id).is_some();
            (thread_id, has_local_focus)
        }),
    )
}

/// Uses an unambiguous embedded input queue rather than an arbitrary HashMap entry.
/// Time: O(T). Space: O(1), for T distinct descendant input threads.
fn choose_input_thread(
    parent_thread: u32,
    candidates: impl IntoIterator<Item = (u32, bool)>,
) -> u32 {
    let mut descendant_count = 0;
    let mut focused_count = 0;
    let mut last_descendant = parent_thread;
    let mut last_focused = parent_thread;
    for (thread_id, has_local_focus) in candidates {
        descendant_count += 1;
        last_descendant = thread_id;
        if has_local_focus {
            focused_count += 1;
            last_focused = thread_id;
        }
    }
    match (focused_count, descendant_count) {
        (1, _) => last_focused,
        (0, 1) => last_descendant,
        _ => parent_thread,
    }
}

/// Enumerates at most one target per foreign input thread under this app window.
/// Time: O(W). Space: O(T), for W descendant windows and T browser input threads.
fn descendant_input_threads(parent_window: HWND) -> HashMap<u32, HWND> {
    let mut windows = HashMap::<u32, HWND>::new();
    unsafe extern "system" fn collect(window: HWND, context: LPARAM) -> BOOL {
        let targets = unsafe { &mut *(context.0 as *mut HashMap<u32, HWND>) };
        let mut process_id = 0;
        let thread_id = unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
        if thread_id != 0 && process_id != unsafe { GetCurrentProcessId() } {
            targets.entry(thread_id).or_insert(window);
        }
        BOOL(1)
    }
    unsafe {
        let _ = EnumChildWindows(
            Some(parent_window),
            Some(collect),
            LPARAM(&mut windows as *mut _ as isize),
        );
    }
    windows
}

fn installed_layouts() -> Result<Vec<HKL>, String> {
    let count = unsafe { GetKeyboardLayoutList(None) };
    if count <= 0 {
        return Err(format!(
            "Could not read installed input languages: {}",
            Error::from_thread()
        ));
    }
    let mut layouts = vec![HKL::default(); count as usize];
    let copied = unsafe { GetKeyboardLayoutList(Some(&mut layouts)) };
    if copied <= 0 {
        return Err(format!(
            "Could not read installed input languages: {}",
            Error::from_thread()
        ));
    }
    layouts.truncate(copied as usize);
    Ok(layouts)
}

fn layout_identifier(layout: HKL) -> String {
    format!("{:016X}", layout.0 as usize)
}

fn resolve_layout(installed: &[HKL], layout_id: &str) -> Result<HKL, String> {
    installed
        .iter()
        .copied()
        .find(|&layout| layout_identifier(layout) == layout_id)
        .ok_or_else(|| {
            "The selected input language is no longer installed. Reopen the language picker.".into()
        })
}

fn state_for_layout(active_layout: HKL) -> Result<InputLanguageState, String> {
    let mut layouts: Vec<_> = installed_layouts()?
        .into_iter()
        .map(describe_layout)
        .collect();
    let mut label_counts = HashMap::new();
    for layout in &layouts {
        *label_counts.entry(layout.label.clone()).or_insert(0usize) += 1;
    }
    for layout in &mut layouts {
        if label_counts[&layout.label] > 1 {
            layout.label = format!("{} · {}", layout.label, layout.id);
        }
    }
    Ok(InputLanguageState {
        active_id: layout_identifier(active_layout),
        layouts,
    })
}

fn describe_layout(layout: HKL) -> InputLanguage {
    let language_id = (layout.0 as usize & 0xffff) as u16;
    let id = layout_identifier(layout);
    let mut locale_name = [0u16; LOCALE_NAME_CAPACITY];
    let name_length = unsafe { LCIDToLocaleName(language_id.into(), Some(&mut locale_name), 0) };
    let (label, code) = if name_length > 0 {
        (
            locale_text(&locale_name, LOCALE_SLOCALIZEDDISPLAYNAME),
            locale_text(&locale_name, LOCALE_SISO639LANGNAME2),
        )
    } else {
        (None, None)
    };
    // Modern Text Services Framework IMEs do not always report through ImmIsIME.
    // CJK input languages therefore also expose the composition limitation.
    let primary_language = language_id & 0x03ff;
    let ime =
        unsafe { ImmIsIME(layout) }.as_bool() || matches!(primary_language, 0x04 | 0x11 | 0x12);
    InputLanguage {
        label: label.unwrap_or_else(|| format!("Input language · {id}")),
        code: code
            .map(|value| value.to_uppercase())
            .unwrap_or_else(|| "LANG".into()),
        id,
        ime,
    }
}

fn locale_text(locale_name: &[u16], information: u32) -> Option<String> {
    let mut buffer = [0u16; LOCALE_TEXT_CAPACITY];
    let length =
        unsafe { GetLocaleInfoEx(PCWSTR(locale_name.as_ptr()), information, Some(&mut buffer)) };
    (length > 1).then(|| String::from_utf16_lossy(&buffer[..length as usize - 1]))
}

fn translate_character(
    layout: HKL,
    scan_code: u32,
    shifted: bool,
    caps_locked: bool,
) -> (String, bool) {
    let virtual_key = unsafe { MapVirtualKeyExW(scan_code, MAPVK_VSC_TO_VK_EX, Some(layout)) };
    if virtual_key == 0 {
        return (String::new(), false);
    }
    let mut state = [0u8; 256];
    if shifted {
        state[VK_SHIFT.0 as usize] = KEY_PRESSED;
    }
    if caps_locked {
        state[VK_CAPITAL.0 as usize] = KEY_TOGGLED;
    }
    let mut buffer = [0u16; CHARACTER_BUFFER_CAPACITY];
    let count = unsafe {
        ToUnicodeEx(
            virtual_key,
            scan_code,
            &state,
            &mut buffer,
            PRESERVE_KEYBOARD_STATE,
            Some(layout),
        )
    };
    let length = (count.unsigned_abs() as usize).min(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..length]);
    if value.chars().any(char::is_control) {
        return (String::new(), false);
    }
    (value, count < 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_layout_sentinels_and_uninstalled_identifiers() {
        let installed = [HKL(std::ptr::without_provenance_mut(0x08090809))];
        for invalid in [
            "",
            "1",
            "0000000000000001",
            "0000000000000000",
            "0000000004090409",
        ] {
            assert!(resolve_layout(&installed, invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            resolve_layout(&installed, "0000000008090809").unwrap(),
            installed[0]
        );
    }

    #[test]
    fn rejects_windows_outside_this_process() {
        assert!(snapshot(HWND::default()).is_err());
        assert!(select(HWND::default(), "0000000008090809").is_err());
    }

    #[test]
    fn temporary_shell_focus_keeps_an_unambiguous_renderer_input_thread() {
        const SHELL_THREAD: u32 = 10;
        const PAGE_THREAD: u32 = 20;
        const OTHER_INPUT_THREAD: u32 = 30;
        assert_eq!(choose_input_thread(SHELL_THREAD, []), SHELL_THREAD);
        assert_eq!(
            choose_input_thread(SHELL_THREAD, [(PAGE_THREAD, false)]),
            PAGE_THREAD
        );
        assert_eq!(
            choose_input_thread(
                SHELL_THREAD,
                [(PAGE_THREAD, true), (OTHER_INPUT_THREAD, false)]
            ),
            PAGE_THREAD
        );
        for candidates in [
            [(PAGE_THREAD, false), (OTHER_INPUT_THREAD, false)],
            [(OTHER_INPUT_THREAD, false), (PAGE_THREAD, false)],
            [(PAGE_THREAD, true), (OTHER_INPUT_THREAD, true)],
            [(OTHER_INPUT_THREAD, true), (PAGE_THREAD, true)],
        ] {
            assert_eq!(choose_input_thread(SHELL_THREAD, candidates), SHELL_THREAD);
        }
    }

    #[test]
    fn installed_languages_and_character_maps_do_not_change_active_layout() {
        let before = unsafe { GetKeyboardLayout(0) };
        let state = state_for_layout(before).unwrap();
        assert!(!state.layouts.is_empty());
        for layout in &state.layouts {
            assert!(!layout.label.is_empty());
            assert!(!layout.code.is_empty());
            assert_eq!(layout.id.len(), 16);
            let rows = virtual_key_rows(&layout.id).unwrap();
            assert_eq!(rows.len(), PHYSICAL_KEY_ROWS.len());
            assert!(rows.iter().flatten().all(|key| [
                &key.value,
                &key.shifted_value,
                &key.caps_value,
                &key.shifted_caps_value
            ]
            .iter()
            .all(|value| !value.chars().any(char::is_control))));
        }
        assert_eq!(unsafe { GetKeyboardLayout(0) }, before);
    }

    #[test]
    fn locale_labels_distinguish_english_regions() {
        let united_states = describe_layout(HKL(std::ptr::without_provenance_mut(0x04090409)));
        let united_kingdom = describe_layout(HKL(std::ptr::without_provenance_mut(0x08090809)));
        assert_ne!(united_states.label, united_kingdom.label);
        assert_eq!(united_states.code, "ENG");
        assert_eq!(united_kingdom.code, "ENG");
    }

    #[test]
    fn installed_english_layouts_distinguish_caps_lock_from_shift() {
        const ENGLISH_US_LAYOUT: u32 = 0x04090409;
        const ENGLISH_UK_LAYOUT: u32 = 0x08090809;
        let before = unsafe { GetKeyboardLayout(0) };
        for layout in installed_layouts().unwrap() {
            let native_id = layout.0 as usize as u32;
            if !matches!(native_id, ENGLISH_US_LAYOUT | ENGLISH_UK_LAYOUT) {
                continue;
            }
            let rows = virtual_key_rows(&layout_identifier(layout)).unwrap();
            let letter = rows.iter().flatten().find(|key| key.value == "a").unwrap();
            assert_eq!(letter.caps_value, "A");
            assert_eq!(letter.shifted_caps_value, "a");
            let number = rows.iter().flatten().find(|key| key.value == "2").unwrap();
            assert_eq!(number.caps_value, "2");
            assert_eq!(
                number.shifted_caps_value,
                if native_id == ENGLISH_US_LAYOUT {
                    "@"
                } else {
                    "\""
                }
            );
        }
        assert_eq!(unsafe { GetKeyboardLayout(0) }, before);
    }
}
