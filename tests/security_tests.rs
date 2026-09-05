//! Unit and Integration Tests for Security Subsystem

use safebrowse::keyboard::osk::{VirtualKey, VirtualKeyboard};
use safebrowse::security::{CaptureProtector, ClipboardBroker};
use windows::Win32::Foundation::HWND;

#[test]
#[ignore = "Destructively clears the user's clipboard; run explicitly with disposable clipboard content"]
fn test_clipboard_purging() {
    let res = ClipboardBroker::purge_clipboard(None);
    assert!(res.is_ok(), "Clipboard purge failed: {:?}", res);
}

#[test]
fn test_capture_protector_invalid_handle() {
    let null_hwnd = HWND(std::ptr::null_mut());
    let res = CaptureProtector::apply_protection(null_hwnd);
    assert!(
        res.is_err(),
        "Null HWND should be rejected by CaptureProtector"
    );
}

#[test]
fn test_dom_injection_script_generation() {
    let script_char = VirtualKeyboard::generate_dom_injection_script("A");
    assert!(script_char.ends_with("(\"A\");"));
    assert!(script_char.contains("document.activeElement"));

    let script_backspace = VirtualKeyboard::generate_dom_injection_script("BACKSPACE");
    assert!(script_backspace.contains("action === 'BACKSPACE'"));

    let script_enter = VirtualKeyboard::generate_dom_injection_script("ENTER");
    assert!(script_enter.contains("action === 'ENTER'"));

    // JSON encoding preserves control characters and quotes as one data argument.
    let action = "'; alert(1); '\n\"\\";
    let script_escape = VirtualKeyboard::generate_dom_injection_script(action);
    let expected_argument = format!("({});", serde_json::to_string(action).unwrap());
    assert!(script_escape.ends_with(&expected_argument));
}

#[test]
fn test_virtual_keyboard_scramble_keys() {
    let mut keys = vec![
        VirtualKey::char_key('a'),
        VirtualKey::char_key('b'),
        VirtualKey::char_key('c'),
        VirtualKey::char_key('d'),
        VirtualKey::char_key('e'),
    ];

    let original_labels: Vec<String> = keys.iter().map(|k| k.label.clone()).collect();
    VirtualKeyboard::scramble_keys(&mut keys);
    let scrambled_labels: Vec<String> = keys.iter().map(|k| k.label.clone()).collect();

    // Verify length and multiset equality (all keys preserved, none dropped or duplicated)
    assert_eq!(keys.len(), 5);
    for original in &original_labels {
        assert!(scrambled_labels.contains(original));
    }
}

#[test]
fn test_virtual_keyboard_default() {
    let mut osk = VirtualKeyboard::default();
    assert!(osk.toggle_shift());
    assert!(!osk.toggle_shift());
    assert!(osk.toggle_scramble());
    assert!(!osk.toggle_scramble());
}

#[test]
fn test_system_input_language_mapping() {
    use safebrowse::keyboard::osk::lang_id_to_code;

    assert_eq!(lang_id_to_code(0x0409), "ENG", "0x0409 (US English) -> ENG");
    assert_eq!(lang_id_to_code(0x0809), "ENG", "0x0809 (UK English) -> ENG");
    assert_eq!(
        lang_id_to_code(0x0804),
        "CHS",
        "0x0804 (Simplified Chinese) -> CHS"
    );
    assert_eq!(lang_id_to_code(0x040C), "FRA", "0x040C (French) -> FRA");
    assert_eq!(lang_id_to_code(0x0407), "DEU", "0x0407 (German) -> DEU");
    assert_eq!(lang_id_to_code(0x040A), "ESP", "0x040A (Spanish) -> ESP");
    assert_eq!(lang_id_to_code(0x0411), "JPN", "0x0411 (Japanese) -> JPN");
    assert_eq!(lang_id_to_code(0x0419), "RUS", "0x0419 (Russian) -> RUS");
}

#[test]
fn test_system_input_language_live() {
    use safebrowse::keyboard::osk::get_system_input_language;

    let lang = get_system_input_language();
    assert_eq!(
        lang.len(),
        3,
        "Language code must be a 3-character identifier"
    );
}

#[test]
fn test_system_battery_status_query() {
    use safebrowse::ui::assets::get_system_battery_status;

    let (icon, pct, _charging) = get_system_battery_status();
    assert!(pct <= 100, "Battery percentage must be <= 100");
    assert!(!icon.is_empty(), "Battery icon must not be empty");
}

#[test]
fn test_system_input_language_cycling() {
    use safebrowse::keyboard::osk::cycle_system_input_language;

    let l1 = cycle_system_input_language();
    let l2 = cycle_system_input_language();
    assert_eq!(l1.len(), 3, "Language code must be 3 characters");
    assert_eq!(l2.len(), 3, "Language code must be 3 characters");
    // One installed layout may not change; the indicator must match the OS.
    assert_eq!(l2, safebrowse::keyboard::osk::get_system_input_language());
}
