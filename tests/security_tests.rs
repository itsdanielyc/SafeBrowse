//! Unit and Integration Tests for Security Subsystem

use safebrowse::keyboard::osk::{VirtualKey, VirtualKeyboard};
use safebrowse::security::{CaptureProtector, ClipboardBroker};
use windows::Win32::Foundation::HWND;

#[test]
fn test_clipboard_purging() {
    let res = ClipboardBroker::purge_clipboard(None);
    assert!(res.is_ok(), "Clipboard purge failed: {:?}", res);
}

#[test]
fn test_capture_protector_invalid_handle() {
    let null_hwnd = HWND(std::ptr::null_mut());
    let res = CaptureProtector::apply_protection(null_hwnd);
    assert!(res.is_err(), "Null HWND should be rejected by CaptureProtector");
}

#[test]
fn test_dom_injection_script_generation() {
    let script_char = VirtualKeyboard::generate_dom_injection_script("A");
    assert!(script_char.contains("const action = 'A';"));
    assert!(script_char.contains("document.activeElement"));

    let script_backspace = VirtualKeyboard::generate_dom_injection_script("BACKSPACE");
    assert!(script_backspace.contains("action === 'BACKSPACE'"));

    let script_enter = VirtualKeyboard::generate_dom_injection_script("ENTER");
    assert!(script_enter.contains("action === 'ENTER'"));

    // Verify sanitization against single-quote escape
    let script_escape = VirtualKeyboard::generate_dom_injection_script("'; alert(1); '");
    assert!(script_escape.contains(r#"const action = '\'; alert(1); \'';"#));
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
