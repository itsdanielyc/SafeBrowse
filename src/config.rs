//! SafeBrowse Configuration & Architectural Constants
//!
//! Centralizes all system-level constants, Win32 desktop names, default URLs,
//! security flags, and profile storage configurations.

use std::time::Duration;

/// Namespace prefix for per-session desktop names; a fresh UUID is appended before creation.
pub const SAFE_DESKTOP_NAME: &str = "SafeBrowseDesktop";

/// Name of the standard interactive Windows desktop.
pub const DEFAULT_DESKTOP_NAME: &str = "Default";

/// Default home/landing page loaded upon starting a new session.
pub const DEFAULT_HOMEPAGE_URL: &str = "https://duckduckgo.com";

/// Application name used for `%APPDATA%` and `%TEMP%` directory scoping.
pub const APP_IDENTIFIER: &str = "SafeBrowse";

/// Prefix for ephemeral session directories generated in `%TEMP%`.
pub const EPHEMERAL_DIR_PREFIX: &str = "SafeBrowse_Session_";

/// Persistent profile directory name within the local app data folder.
pub const PERSISTENT_PROFILE_DIR_NAME: &str = "Profile_Persistent";

/// File name for the persistent bookmarks database.
pub const BOOKMARKS_FILE_NAME: &str = "bookmarks.json";

/// Win32 `SetWindowDisplayAffinity` constant for `WDA_EXCLUDEFROMCAPTURE`.
/// Available on Windows 10 Version 2004 (Build 19041) and newer.
/// Requests exclusion from supported DWM capture APIs; it is not a security boundary.
pub const WDA_EXCLUDEFROMCAPTURE: u32 = 0x0000_0011;

/// Fallback Win32 `WDA_MONITOR` affinity for legacy Windows 10 builds.
pub const WDA_MONITOR: u32 = 0x0000_0001;

/// Desktop Access Mask: Standard rights required to switch, create, and interact with a desktop.
pub const SAFE_DESKTOP_ACCESS_MASK: u32 = 0x01FF; // DESKTOP_ALL_ACCESS

/// Win32 desktop access right required strictly for desktop switching (`DESKTOP_SWITCHDESKTOP`).
pub const DESKTOP_SWITCHDESKTOP_ACCESS: u32 = 0x0100;

/// Maximum retry attempts when calling `SwitchDesktop` to absorb OS focus transition latency.
pub const DESKTOP_SWITCH_MAX_RETRIES: u32 = 10;

/// Delay between successive `SwitchDesktop` retry attempts.
pub const DESKTOP_SWITCH_RETRY_DELAY: Duration = Duration::from_millis(15);

/// Timeout interval for watchdog liveness polling across desktop boundaries.
pub const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Maximum wait for a worker after forced termination or failed authorization.
pub const WORKER_TERMINATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Lets released WebView2 processes unlock their temporary files before reporting a remnant.
pub const EPHEMERAL_PROFILE_CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Orderly worker exit includes profile cleanup before the final clipboard purge.
pub const WORKER_GRACEFUL_SHUTDOWN_TIMEOUT: Duration =
    EPHEMERAL_PROFILE_CLEANUP_TIMEOUT.saturating_add(WORKER_TERMINATION_TIMEOUT);

/// Browser behavior switches that preserve WebView2's sandbox and isolation defaults.
pub const CHROMIUM_ARGS_SECURITY: &[&str] =
    &["--no-default-browser-check", "--disable-print-preview"];
