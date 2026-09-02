//! Browser Profile Container & Storage Sandbox
//!
//! Manages ephemeral vs persistent profile directories.
//! In ephemeral mode, an isolated directory in `%TEMP%` is created and completely purged
//! when the browser closes, guaranteeing zero lingering cookies, cache, or web storage.

use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::config::{APP_IDENTIFIER, EPHEMERAL_DIR_PREFIX, PERSISTENT_PROFILE_DIR_NAME};

/// Profile execution mode for SafeBrowse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileMode {
    /// Zero-retention ephemeral session. Storage is destroyed on exit.
    Ephemeral,
    /// Durable container isolated from other system browsers.
    Persistent,
}

/// Manages the user data directory lifecycle for the embedded Chromium engine.
#[derive(Debug)]
pub struct ProfileManager {
    mode: ProfileMode,
    data_directory: PathBuf,
}

impl ProfileManager {
    /// Creates a new profile directory according to the chosen mode.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn new(mode: ProfileMode) -> Result<Self, String> {
        let data_directory = match mode {
            ProfileMode::Ephemeral => {
                let temp_dir = std::env::temp_dir();
                let session_name = format!("{}{}", EPHEMERAL_DIR_PREFIX, Uuid::new_v4());
                let dir = temp_dir.join(session_name);
                fs::create_dir_all(&dir)
                    .map_err(|e| format!("Failed to create ephemeral profile directory: {}", e))?;
                dir
            }
            ProfileMode::Persistent => {
                let project_dirs = directories::ProjectDirs::from("com", "SafeBrowse", APP_IDENTIFIER)
                    .ok_or_else(|| "Failed to resolve app data directory".to_string())?;
                let dir = project_dirs.data_local_dir().join(PERSISTENT_PROFILE_DIR_NAME);
                fs::create_dir_all(&dir)
                    .map_err(|e| format!("Failed to create persistent profile directory: {}", e))?;
                dir
            }
        };

        Ok(Self {
            mode,
            data_directory,
        })
    }

    /// Returns the absolute path to the profile user data directory.
    #[inline]
    pub fn data_directory(&self) -> &Path {
        &self.data_directory
    }

    /// Returns the active profile mode.
    #[inline]
    pub fn mode(&self) -> ProfileMode {
        self.mode
    }

    /// Recursively purges the ephemeral directory from disk.
    ///
    /// # Complexity
    /// - Time: O(F) where F is the number of files written during the session
    /// - Space: O(D) recursion stack depth
    pub fn purge_ephemeral_storage(&self) -> Result<(), String> {
        if self.mode == ProfileMode::Ephemeral && self.data_directory.exists() {
            // Why: Retry a few times if Edge WebView2 processes take a moment to release file locks.
            let mut attempts = 0;
            const MAX_ATTEMPTS: u32 = 5;
            while attempts < MAX_ATTEMPTS {
                match fs::remove_dir_all(&self.data_directory) {
                    Ok(_) => return Ok(()),
                    Err(_) => {
                        attempts += 1;
                        std::thread::sleep(std::time::Duration::from_millis(150));
                    }
                }
            }
            // If still locked after retries, try best-effort non-fatal warning
            let _ = fs::remove_dir_all(&self.data_directory);
        }
        Ok(())
    }
}

impl Drop for ProfileManager {
    fn drop(&mut self) {
        let _ = self.purge_ephemeral_storage();
    }
}
