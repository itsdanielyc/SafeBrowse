//! Secure Virtual On-Screen Keyboard Subsystem
//!
//! Sends input directly into the focused DOM field without calling `SendInput`
//! or `keybd_event`. This avoids ordinary keyboard-hook input paths, but does not
//! protect against compromised pages, browser processes, or privileged malware.

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

impl Default for VirtualKeyboard {
    fn default() -> Self {
        Self::new()
    }
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

    /// Generates an input action for the focused editable element.
    ///
    /// Actions are serialized as JSON rather than interpolated into JavaScript
    /// string syntax. Native input setters keep controlled form state in sync.
    ///
    /// Time: O(N). Space: O(N), where N is action length plus script length.
    pub fn generate_dom_injection_script(input_action: &str) -> String {
        let serialized_action =
            serde_json::to_string(input_action).expect("Serializing a string to JSON cannot fail");
        format!("({})({});", include_str!("input.js"), serialized_action)
    }

    /// Maps a Win32 primary language ID to an unambiguous 3-letter UI display code.
    pub fn lang_id_to_code(lang_id: u16) -> String {
        let primary = lang_id & 0x03FF;
        match primary {
            0x09 => "ENG".to_string(), // English
            0x04 => "CHS".to_string(), // Chinese
            0x0C => "FRA".to_string(), // French
            0x07 => "DEU".to_string(), // German
            0x0A => "ESP".to_string(), // Spanish
            0x11 => "JPN".to_string(), // Japanese
            0x12 => "KOR".to_string(), // Korean
            0x19 => "RUS".to_string(), // Russian
            0x10 => "ITA".to_string(), // Italian
            0x16 => "POR".to_string(), // Portuguese
            0x13 => "NLD".to_string(), // Dutch
            0x1D => "SWE".to_string(), // Swedish
            0x1F => "TUR".to_string(), // Turkish
            0x01 => "ARA".to_string(), // Arabic
            0x0E => "HEB".to_string(), // Hebrew
            0x2A => "VIE".to_string(), // Vietnamese
            _ => "ENG".to_string(),
        }
    }

    /// Retrieves the current thread/process input layout code (e.g. "ENG", "CHS").
    pub fn get_system_input_language() -> String {
        unsafe {
            let hkl = windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout(0);
            let lang_id = (hkl.0 as usize & 0xFFFF) as u16;
            Self::lang_id_to_code(lang_id)
        }
    }

    /// Activates the next installed keyboard layout and reports the actual layout.
    /// A system with one installed layout keeps its current language indicator.
    pub fn cycle_system_input_language() -> String {
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::{
                ActivateKeyboardLayout, HKL, KLF_SETFORPROCESS,
            };
            // Win32 defines HKL_NEXT as the sentinel handle with integer value 1.
            let next_layout = HKL(std::ptr::without_provenance_mut(1));
            let _ = ActivateKeyboardLayout(next_layout, KLF_SETFORPROCESS);
        }
        Self::get_system_input_language()
    }

    /// Shuffles an array of keys using the Fisher-Yates algorithm.
    ///
    /// # Complexity
    /// - Time: O(N) where N is the number of keys
    /// - Space: O(1) in-place shuffle
    pub fn scramble_keys(keys: &mut [VirtualKey]) {
        // UUID v4 uses the OS random source, avoiding predictable timestamps.
        let mut rng_state = uuid::Uuid::new_v4().as_u128() as u64;
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

/// Helper exposing input language retrieval at module level.
pub fn get_system_input_language() -> String {
    VirtualKeyboard::get_system_input_language()
}

/// Helper exposing input language cycling at module level.
pub fn cycle_system_input_language() -> String {
    VirtualKeyboard::cycle_system_input_language()
}

/// Helper exposing language code conversion at module level.
pub fn lang_id_to_code(lang_id: u16) -> String {
    VirtualKeyboard::lang_id_to_code(lang_id)
}
