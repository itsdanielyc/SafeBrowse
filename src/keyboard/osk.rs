//! Secure Virtual On-Screen Keyboard Subsystem
//!
//! Provides a trusted, hook-immune virtual keyboard.
//! Crucial Security Invariant: Does NOT call `SendInput` or `keybd_event`.
//! Dispatches character values directly into the focused DOM input element via
//! internal IPC script evaluation, completely blinding system-level keyloggers.

use serde::{Deserialize, Serialize};

/// Type of key in the virtual keyboard layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyType {
    Character,
    Shift,
    Backspace,
    Space,
    Enter,
    Scramble,
}

/// Represents a single key rendered on the virtual keyboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualKey {
    pub label: String,
    pub value: String,
    pub key_type: KeyType,
}

impl VirtualKey {
    pub fn char_key(c: char) -> Self {
        Self {
            label: c.to_string(),
            value: c.to_string(),
            key_type: KeyType::Character,
        }
    }

    pub fn special(label: &str, value: &str, key_type: KeyType) -> Self {
        Self {
            label: label.to_string(),
            value: value.to_string(),
            key_type,
        }
    }
}

/// Manages keyboard layouts and JavaScript DOM injection script generation.
pub struct VirtualKeyboard {
    is_shifted: bool,
    is_scrambled: bool,
}

impl VirtualKeyboard {
    /// Initializes a new VirtualKeyboard instance.
    pub fn new() -> Self {
        Self {
            is_shifted: false,
            is_scrambled: false,
        }
    }

    /// Toggles the Shift state.
    pub fn toggle_shift(&mut self) -> bool {
        self.is_shifted = !self.is_shifted;
        self.is_shifted
    }

    /// Toggles the layout scrambling state.
    pub fn toggle_scramble(&mut self) -> bool {
        self.is_scrambled = !self.is_scrambled;
        self.is_scrambled
    }

    /// Generates the direct DOM input injection script for a given key value.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn generate_dom_injection_script(input_action: &str) -> String {
        let sanitized_action = input_action.replace('\\', "\\\\").replace('\'', "\\'");

        format!(
            r#"(function() {{
    const el = document.activeElement;
    if (!el) return;
    const action = '{sanitized_action}';
    
    if (action === 'BACKSPACE') {{
        if (typeof el.selectionStart === 'number' && typeof el.selectionEnd === 'number' && el.setRangeText) {{
            const start = Math.max(0, el.selectionStart === el.selectionEnd ? el.selectionStart - 1 : el.selectionStart);
            const end = el.selectionEnd;
            el.setRangeText('', start, end, 'end');
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
            el.dispatchEvent(new Event('change', {{ bubbles: true }}));
        }} else if (el.value && el.value.length > 0) {{
            el.value = el.value.slice(0, -1);
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
        }}
        return;
    }}

    if (action === 'ENTER') {{
        el.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', code: 'Enter', keyCode: 13, bubbles: true }}));
        el.dispatchEvent(new KeyboardEvent('keypress', {{ key: 'Enter', code: 'Enter', keyCode: 13, bubbles: true }}));
        el.dispatchEvent(new KeyboardEvent('keyup', {{ key: 'Enter', code: 'Enter', keyCode: 13, bubbles: true }}));
        if (el.form) {{
            el.form.dispatchEvent(new Event('submit', {{ bubbles: true, cancelable: true }}));
        }}
        return;
    }}

    if (typeof el.selectionStart === 'number' && typeof el.selectionEnd === 'number' && el.setRangeText) {{
        el.setRangeText(action, el.selectionStart, el.selectionEnd, 'end');
        el.dispatchEvent(new Event('input', {{ bubbles: true }}));
        el.dispatchEvent(new Event('change', {{ bubbles: true }}));
    }} else if ('value' in el) {{
        el.value = (el.value || '') + action;
        el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    }}
}})();"#,
            sanitized_action = sanitized_action
        )
    }

    /// Shuffles an array of keys using the Fisher-Yates algorithm.
    ///
    /// # Complexity
    /// - Time: O(N) where N is the number of keys
    /// - Space: O(1) in-place shuffle
    pub fn scramble_keys(keys: &mut [VirtualKey]) {
        use std::time::{SystemTime, UNIX_EPOCH};
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(12345);

        let mut rng_state = seed as u64;
        let len = keys.len();
        if len <= 1 {
            return;
        }

        for i in (1..len).rev() {
            // Xorshift64 pseudo-random generator
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            let j = (rng_state % ((i + 1) as u64)) as usize;
            keys.swap(i, j);
        }
    }
}
