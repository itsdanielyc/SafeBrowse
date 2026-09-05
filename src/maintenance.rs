//! Restricted per-user installer maintenance with no caller-selected deletion paths.
//!
//! The helper uses the browser's exact-handle deletion and session lock. Uninstall
//! removes files, not forensic remnants, and never sweeps other applications' data.

#[cfg(test)]
mod tests;

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use uuid::Uuid;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

use crate::browser::profile::{reclaim_for_uninstall_at, storage};
use crate::browser::runtime::{inspect_webview2_runtime_for_installation, RuntimeInspectionError};
use crate::config::{APP_IDENTIFIER, BOOKMARKS_FILE_NAME, PERSISTENT_PROFILE_DIR_NAME};
use crate::security::current_process_integrity;

const PERMISSION_FILE_NAME: &str = "permissions.json";
const KNOWN_CONFIGURATION_FILES: [&str; 2] = [BOOKMARKS_FILE_NAME, PERMISSION_FILE_NAME];
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_CONFIGURATION_ENTRIES: usize = 512;
const SESSION_MUTEX_NAME: windows::core::PCWSTR =
    windows::core::w!("Local\\SafeBrowse_Session_Mutex");
const USAGE: &str =
    "Usage: safebrowse-maintenance.exe check-runtime | cleanup [--remove-user-data]";

/// Stable installer exit status: runtime setup is permitted only for exit code 10.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceError {
    pub message: String,
    pub exit_code: u8,
}

impl From<String> for MaintenanceError {
    fn from(message: String) -> Self {
        Self {
            message,
            exit_code: 1,
        }
    }
}

impl From<RuntimeInspectionError> for MaintenanceError {
    fn from(error: RuntimeInspectionError) -> Self {
        let exit_code = match &error {
            RuntimeInspectionError::InstallationRequired(_) => 10,
            RuntimeInspectionError::Blocked(_) => 1,
        };
        Self {
            message: error.to_string(),
            exit_code,
        }
    }
}

/// The complete installer command surface; arbitrary paths are deliberately unsupported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceCommand {
    CheckRuntime,
    Cleanup { remove_user_data: bool },
}

impl MaintenanceCommand {
    /// Validates the whole command line before any filesystem or runtime operation.
    pub fn parse(arguments: &[String]) -> Result<Self, String> {
        match arguments {
            [command] if command == "check-runtime" => Ok(Self::CheckRuntime),
            [command] if command == "cleanup" => Ok(Self::Cleanup {
                remove_user_data: false,
            }),
            [command, option] if command == "cleanup" && option == "--remove-user-data" => {
                Ok(Self::Cleanup {
                    remove_user_data: true,
                })
            }
            _ => Err(USAGE.into()),
        }
    }

    /// Runs maintenance as the invoking standard user and returns human-readable results.
    /// Cleanup never creates a browser session or modifies the shared WebView2 installation.
    pub fn execute(self) -> Result<String, MaintenanceError> {
        refuse_elevated_maintenance()?;
        match self {
            Self::CheckRuntime => {
                let runtime = inspect_webview2_runtime_for_installation()?;
                Ok(format!(
                    "WebView2 Runtime {} is available (minimum supported {}).",
                    runtime.version, runtime.minimum_supported_version
                ))
            }
            Self::Cleanup { remove_user_data } => {
                let _session_lock = MaintenanceLock::acquire(SESSION_MUTEX_NAME)?;
                let paths = MaintenancePaths::for_current_user()?;
                cleanup_at(&paths, remove_user_data, CLEANUP_TIMEOUT).map_err(Into::into)
            }
        }
    }
}

/// Fails before selecting user paths when an elevated token could target another identity.
fn refuse_elevated_maintenance() -> Result<(), String> {
    let integrity = current_process_integrity().map_err(|error| {
        format!("Cannot verify installer maintenance privileges: {error}. No data was removed.")
    })?;
    if integrity.requires_browser_host_refusal() {
        return Err("Run SafeBrowse installation and removal without administrator privileges, using the Windows account that installed it. No data was removed.".into());
    }
    Ok(())
}

/// Object creation, rather than a later wait, is the launcher's atomic exclusion protocol.
struct MaintenanceLock(HANDLE);

impl MaintenanceLock {
    fn acquire(name: windows::core::PCWSTR) -> Result<Self, String> {
        let handle = unsafe { CreateMutexW(None, false, name) }.map_err(|error| {
            format!("Cannot acquire the SafeBrowse session lock: {error}. No data was removed.")
        })?;
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let lock = Self(handle);
        if already_exists {
            return Err("SafeBrowse is running or another maintenance operation is active. Close SafeBrowse and retry. No data was removed.".into());
        }
        Ok(lock)
    }
}

impl Drop for MaintenanceLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Test-only construction injects disposable paths; production paths are never CLI arguments.
struct MaintenancePaths {
    ephemeral_root: PathBuf,
    config_dir: PathBuf,
    data_dir: PathBuf,
}

impl MaintenancePaths {
    fn for_current_user() -> Result<Self, String> {
        let project = directories::ProjectDirs::from("com", "SafeBrowse", APP_IDENTIFIER).ok_or(
            "Cannot resolve this user's SafeBrowse data directories. No data was removed.",
        )?;
        Ok(Self {
            ephemeral_root: storage::owned_root_path(),
            config_dir: project.config_dir().to_owned(),
            data_dir: project.data_local_dir().to_owned(),
        })
    }
}

/// Time O(E + F), space O(D + E), bounded entries E, files F and recursion depth D.
fn cleanup_at(
    paths: &MaintenancePaths,
    remove_user_data: bool,
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let mut failures = Vec::new();
    let mut notes = Vec::new();
    match reclaim_for_uninstall_at(&paths.ephemeral_root, timeout) {
        Ok(report) => {
            notes.push(format!(
                "Removed {} inactive temporary browser profile(s).",
                report.reclaimed
            ));
            failures.extend(report.failures);
            if report.limit_reached {
                failures.push("Temporary-profile cleanup reached its work limit. Run removal again to continue.".into());
            }
            if report.skipped == 0 && !report.limit_reached && failures.is_empty() {
                match storage::remove_empty_owned_root(&paths.ephemeral_root) {
                    Ok(true) => notes.push("Removed the empty temporary profile ownership folder.".into()),
                    Ok(false) if paths.ephemeral_root.exists() => failures.push(format!(
                        "The temporary profile ownership folder at {} changed during cleanup or is not empty. It was retained for inspection and retry.", paths.ephemeral_root.display()
                    )),
                    Ok(false) => {},
                    Err(error) => failures.push(format!("Cannot remove the temporary profile ownership folder at {}: {error}", paths.ephemeral_root.display())),
                }
            }
            if paths.ephemeral_root.exists() {
                notes.push(format!("The temporary profile ownership folder remains at {}. Review the reported cleanup failure before retrying.", paths.ephemeral_root.display()));
            }
        }
        Err(error) => failures.push(error),
    }
    if remove_user_data {
        clean_configuration(&paths.config_dir, deadline, &mut failures);
        remove_known(
            &paths.data_dir.join(PERSISTENT_PROFILE_DIR_NAME),
            true,
            deadline,
            &mut failures,
        );
        for directory in [&paths.config_dir, &paths.data_dir] {
            if let Err(error) = storage::remove_empty_known_directory(directory) {
                failures.push(format!(
                    "Cannot remove empty SafeBrowse folder {}: {error}",
                    directory.display()
                ));
            }
        }
        notes.push("Requested removal of SafeBrowse bookmarks, preferences, saved permissions and its persistent browser profile.".into());
    } else {
        notes.push("SafeBrowse bookmarks, preferences, saved permissions and persistent browsing data were kept.".into());
    }
    notes.push("Downloaded files, shared WebView2, and legacy or unrelated temporary directories were not removed.".into());
    if failures.is_empty() {
        return Ok(notes.join("\n"));
    }
    Err(format!("SafeBrowse cleanup is incomplete. Close any remaining SafeBrowse processes and retry.\n\n{}\n\n{}", failures.join("\n"), notes.join("\n")))
}

/// Visits one pinned configuration directory and deletes only exact known file names.
/// Time O(E), space O(D), bounded by MAX_CONFIGURATION_ENTRIES and the deadline.
fn clean_configuration(directory: &Path, deadline: Instant, failures: &mut Vec<String>) {
    let result = (|| -> std::io::Result<()> {
        let Some(_ancestors) = storage::pin_existing_directory(directory)? else {
            return Ok(());
        };
        for (index, entry) in fs::read_dir(directory)?.enumerate() {
            if index >= MAX_CONFIGURATION_ENTRIES || Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Configuration cleanup reached its work limit",
                ));
            }
            let entry = entry?;
            if is_known_configuration_name(&entry.file_name()) {
                remove_known(&entry.path(), false, deadline, failures);
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        failures.push(format!(
            "SafeBrowse configuration remains at {}: {error}",
            directory.display()
        ));
    }
}

/// Accepts the exact stores and their canonical UUID-v4 atomic-write staging files.
fn is_known_configuration_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    KNOWN_CONFIGURATION_FILES.iter().any(|known| {
        if name == *known {
            return true;
        }
        let Some(suffix) = name
            .strip_prefix(known)
            .and_then(|rest| rest.strip_prefix(".tmp."))
        else {
            return false;
        };
        Uuid::parse_str(suffix).is_ok_and(|identifier| {
            identifier.get_version_num() == 4 && identifier.to_string() == suffix
        })
    })
}

fn remove_known(path: &Path, directory: bool, deadline: Instant, failures: &mut Vec<String>) {
    if Instant::now() >= deadline {
        failures.push(format!(
            "Cleanup reached its work limit before removing {}.",
            path.display()
        ));
        return;
    }
    if let Err(error) = storage::remove_known_path(path, directory, deadline) {
        failures.push(format!(
            "SafeBrowse data remains at {}: {error}",
            path.display()
        ));
    }
}
