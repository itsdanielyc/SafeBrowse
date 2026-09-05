//! Bounded Windows filenames and exclusive per-transfer output directories.

use std::path::{Path, PathBuf};

const MAX_FILE_NAME_UNITS: usize = 120;
const MAX_EXTENSION_UNITS: usize = 20;
const OUTPUT_DIRECTORY: &str = "SafeBrowse";
const FALLBACK_FILE_NAME: &str = "download.bin";

/// A fresh UUID directory avoids WebView2's documented overwrite behavior for existing paths.
pub(super) struct DownloadDestination {
    directory: PathBuf,
    path: PathBuf,
    retained: bool,
}

impl DownloadDestination {
    /// Allocates no file before approval and never reuses a preexisting transfer directory.
    pub(super) fn new(root_override: Option<&Path>, file_name: &str) -> Result<Self, String> {
        let root = match root_override {
            Some(path) => path.to_owned(),
            None => directories::UserDirs::new()
                .and_then(|directories| {
                    directories
                        .download_dir()
                        .map(|path| path.join(OUTPUT_DIRECTORY))
                })
                .ok_or("Windows did not provide a Downloads folder")?,
        };
        if !root.is_absolute() {
            return Err("The Downloads folder must be an absolute path".into());
        }
        std::fs::create_dir_all(&root)
            .map_err(|error| format!("Cannot create the download folder: {error}"))?;
        let directory = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&directory)
            .map_err(|error| format!("Cannot reserve a fresh download destination: {error}"))?;
        let path = directory.join(safe_file_name(file_name));
        Ok(Self {
            directory,
            path,
            retained: false,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn keep(&mut self) {
        self.retained = true;
    }
}

impl Drop for DownloadDestination {
    fn drop(&mut self) {
        if !self.retained {
            // Chromium may release file handles asynchronously. Cleanup is bounded and best effort.
            let _ = std::fs::remove_file(&self.path);
            if let Some(file_name) = self.path.file_name().and_then(|name| name.to_str()) {
                let _ =
                    std::fs::remove_file(self.directory.join(format!("{file_name}.crdownload")));
            }
            let _ = std::fs::remove_dir(&self.directory);
        }
    }
}

/// Removes path traversal, stream syntax, device names, bidi controls and trailing Win32 aliases.
/// Time/space: O(n), where n is the native suggested name length; output is at most 120 UTF-16 units.
pub(super) fn safe_file_name(suggested: &str) -> String {
    let basename = suggested.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut cleaned: String = basename.chars().map(|character| {
        if character.is_control()
            || matches!(character, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            || matches!(character as u32, 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFEFF)
        { '_' } else { character }
    }).collect();
    cleaned = cleaned.trim_matches([' ', '.']).to_owned();
    if cleaned.is_empty() {
        return FALLBACK_FILE_NAME.into();
    }
    let base = cleaned
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_uppercase();
    if matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        base.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    }) {
        cleaned.insert(0, '_');
    }
    if cleaned.encode_utf16().count() <= MAX_FILE_NAME_UNITS {
        return cleaned;
    }
    let suffix = cleaned
        .rfind('.')
        .map(|index| &cleaned[index..])
        .filter(|extension| extension.encode_utf16().count() <= MAX_EXTENSION_UNITS)
        .unwrap_or_default();
    let prefix_budget = MAX_FILE_NAME_UNITS - suffix.encode_utf16().count();
    let mut used = 0;
    let prefix: String = cleaned
        .chars()
        .take_while(|character| {
            used += character.len_utf16();
            used <= prefix_budget
        })
        .collect();
    format!("{}{suffix}", prefix.trim_end_matches([' ', '.']))
}
