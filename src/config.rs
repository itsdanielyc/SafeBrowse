//! SafeBrowse Configuration & Architectural Constants
//!
//! Centralizes all system-level constants, Win32 desktop names, default URLs,
//! security flags, and profile storage configurations.

use std::time::Duration;

/// Name of the isolated Win32 desktop created inside the interactive window station (`WinSta0`).
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
/// Instructs the Desktop Window Manager (DWM) to render this window completely black
/// to any screen capture APIs (GDI `BitBlt`, Snipping Tool, OBS, Teams/Discord screenshare).
pub const WDA_EXCLUDEFROMCAPTURE: u32 = 0x0000_0011;

/// Fallback Win32 `WDA_MONITOR` affinity for legacy Windows 10 builds.
pub const WDA_MONITOR: u32 = 0x0000_0001;

/// Desktop Access Mask: Standard rights required to switch, create, and interact with a desktop.
pub const SAFE_DESKTOP_ACCESS_MASK: u32 = 0x01FF; // DESKTOP_ALL_ACCESS

/// Timeout interval for watchdog liveness polling across desktop boundaries.
pub const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Maximum duration to wait for graceful worker process termination before forceful kill.
pub const WORKER_TERMINATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Browser automation suppression switches passed to Chromium / WebView2.
/// Ensures the browser does not expose `navigator.webdriver = true` or automation telemetry.
pub const CHROMIUM_ARGS_SECURITY: &[&str] = &[
    "--disable-blink-features=AutomationControlled",
    "--disable-features=IsolateOrigins,site-per-process",
    "--no-default-browser-check",
    "--disable-component-update",
];
