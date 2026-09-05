//! Temporary profile ownership, sharing locks, and conservative crash reclamation.
//!
//! Cleanup deletes files, not forensic remnants. It does not isolate this user
//! from same-user malware. Legacy unmarked temporary profiles are never swept.

pub(crate) mod storage;
#[cfg(test)]
mod tests;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::{
    APP_IDENTIFIER, EPHEMERAL_PROFILE_CLEANUP_TIMEOUT, PERSISTENT_PROFILE_DIR_NAME,
};
use storage::{CleanupBudget, SessionLease};

const PURGE_RETRY_DELAY: Duration = Duration::from_millis(200);
const RECLAMATION_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RECLAMATION_ENTRIES: usize = 128;

/// Profile execution mode for SafeBrowse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileMode {
    /// Temporary storage removed after shutdown or reclaimed at a later startup.
    Ephemeral,
    /// Durable container isolated from other system browsers.
    Persistent,
}

/// Outcomes of a bounded scan of the application-owned temporary root.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EphemeralCleanupReport {
    pub reclaimed: usize,
    pub skipped: usize,
    pub failures: Vec<String>,
    pub limit_reached: bool,
}

/// Manages the browser profile and its exclusive temporary-session lease.
#[derive(Debug)]
pub struct ProfileManager {
    mode: ProfileMode,
    data_directory: PathBuf,
    cleanup: Mutex<CleanupState>,
}

#[derive(Debug, Default)]
struct CleanupState {
    lease: Option<SessionLease>,
    last_result: Option<Result<(), String>>,
}

impl ProfileManager {
    /// Creates one profile without scanning or deleting any other profile.
    pub fn new(mode: ProfileMode) -> Result<Self, String> {
        if mode == ProfileMode::Ephemeral {
            return Self::new_ephemeral_at(&storage::owned_root_path());
        }
        let project_dirs = directories::ProjectDirs::from("com", "SafeBrowse", APP_IDENTIFIER)
            .ok_or_else(|| "Failed to resolve app data directory".to_string())?;
        let data_directory = project_dirs
            .data_local_dir()
            .join(PERSISTENT_PROFILE_DIR_NAME);
        fs::create_dir_all(&data_directory)
            .map_err(|error| format!("Failed to create persistent profile directory: {error}"))?;
        Ok(Self {
            mode,
            data_directory,
            cleanup: Mutex::new(CleanupState::default()),
        })
    }

    /// An injected root keeps native fixtures separate from actual browsing data.
    fn new_ephemeral_at(root: &Path) -> Result<Self, String> {
        let (data_directory, lease) = SessionLease::create(root).map_err(|error| {
            format!(
                "Cannot create temporary browser profile at {}: {error}",
                root.display()
            )
        })?;
        Ok(Self {
            mode: ProfileMode::Ephemeral,
            data_directory,
            cleanup: Mutex::new(CleanupState {
                lease: Some(lease),
                last_result: None,
            }),
        })
    }

    /// Returns the absolute browser user-data directory.
    pub fn data_directory(&self) -> &Path {
        &self.data_directory
    }

    /// Returns the active profile mode.
    pub fn mode(&self) -> ProfileMode {
        self.mode
    }

    /// Deletes temporary data after callers release all browser views and contexts.
    ///
    /// Transient locks are retried for ten seconds. Explicit callers can retry a
    /// previous error; Drop never repeats an already attempted cleanup delay.
    pub fn purge_ephemeral_storage(&self) -> Result<(), String> {
        self.purge_with_timeout(EPHEMERAL_PROFILE_CLEANUP_TIMEOUT)
    }

    fn purge_with_timeout(&self, timeout: Duration) -> Result<(), String> {
        if self.mode != ProfileMode::Ephemeral {
            return Ok(());
        }
        let mut state = self
            .cleanup
            .lock()
            .map_err(|_| "Profile cleanup state was poisoned")?;
        let Some(lease) = state.lease.as_mut() else {
            return Ok(());
        };
        let deadline = Instant::now() + timeout;
        let result = loop {
            let mut budget = CleanupBudget::new(deadline);
            match lease.purge(&self.data_directory, &mut budget) {
                Ok(()) => break Ok(()),
                Err(error) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if storage::is_transient_cleanup_error(&error) && !remaining.is_zero() {
                        std::thread::sleep(PURGE_RETRY_DELAY.min(remaining));
                        continue;
                    }
                    break Err(format!(
                        "Temporary browser data remains at {}: {error}",
                        self.data_directory.display()
                    ));
                }
            }
        };
        if result.is_ok() {
            state.lease.take();
        }
        state.last_result = Some(result.clone());
        result
    }
}

impl Drop for ProfileManager {
    fn drop(&mut self) {
        let already_attempted = self
            .cleanup
            .get_mut()
            .map(|state| state.last_result.is_some())
            .unwrap_or(true);
        if !already_attempted {
            if let Err(error) = self.purge_ephemeral_storage() {
                eprintln!("SafeBrowse profile cleanup: {error}");
            }
        }
    }
}

/// Reclaims marked, unlocked profiles only in the new application-owned root.
///
/// Call once after acquiring the launcher session mutex. Missing roots are a
/// no-op. Legacy, persistent, active and unrecognized directories are untouched.
/// Work is bounded; an individual slow filesystem call can exceed the time limit.
pub fn reclaim_abandoned_ephemeral_profiles() -> Result<EphemeralCleanupReport, String> {
    reclaim_at(&storage::owned_root_path(), RECLAMATION_TIMEOUT)
}

/// Time O(E + F), space O(D), bounded entries E, files F and directory depth D.
fn reclaim_at(root_path: &Path, timeout: Duration) -> Result<EphemeralCleanupReport, String> {
    reclaim_with_policy(root_path, timeout, false)
}

/// Uninstall must disclose every item it cannot safely reclaim; startup may skip them.
pub(crate) fn reclaim_for_uninstall_at(
    root_path: &Path,
    timeout: Duration,
) -> Result<EphemeralCleanupReport, String> {
    reclaim_with_policy(root_path, timeout, true)
}

/// Time O(E + F), space O(D), bounded entries E, files F and directory depth D.
fn reclaim_with_policy(
    root_path: &Path,
    timeout: Duration,
    report_skipped: bool,
) -> Result<EphemeralCleanupReport, String> {
    let Some(root) = storage::OwnedRoot::open(root_path, false).map_err(|error| {
        format!(
            "Cannot inspect temporary profile storage at {}: {error}",
            root_path.display()
        )
    })?
    else {
        return Ok(EphemeralCleanupReport::default());
    };
    let mut report = EphemeralCleanupReport::default();
    let mut budget = CleanupBudget::new(Instant::now() + timeout);
    let entries = fs::read_dir(root.path()).map_err(|error| error.to_string())?;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_RECLAMATION_ENTRIES || budget.exhausted() {
            report.limit_reached = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.failures.push(error.to_string());
                continue;
            }
        };
        if entry.file_name() == storage::ROOT_MARKER_NAME {
            continue;
        }
        let Some(identifier) = storage::session_identifier(&entry.file_name()) else {
            report.skipped += 1;
            if report_skipped {
                report.failures.push(format!(
                    "Unrecognized item remains at {}; it was not removed.",
                    entry.path().display()
                ));
            }
            continue;
        };
        let path = entry.path();
        let mut lease = match SessionLease::reopen(root_path, &path, identifier) {
            Ok(lease) => lease,
            Err(error) => {
                report.skipped += 1;
                if report_skipped {
                    report.failures.push(format!(
                        "Temporary browser data remains at {}: {error}",
                        path.display()
                    ));
                }
                continue;
            }
        };
        match lease.purge(&path, &mut budget) {
            Ok(()) => report.reclaimed += 1,
            Err(error) => report.failures.push(format!(
                "Temporary browser data remains at {}: {error}",
                path.display()
            )),
        }
    }
    report.limit_reached |= budget.exhausted();
    Ok(report)
}
