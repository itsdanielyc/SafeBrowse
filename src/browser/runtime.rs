//! Detects the browser runtime before startup creates session resources.

mod overrides;

use std::fmt;
use std::path::Path;

use webview2_com::Microsoft::Web::WebView2::Win32::{
    GetAvailableCoreWebView2BrowserVersionString, ICoreWebView2Environment7,
};
use webview2_com::{take_pwstr, CoTaskMemPWSTR};
use webview2_core::{Error, Interface, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use wry::{WebView, WebViewExtWindows};

const WEBVIEW2_DOWNLOAD_URL: &str =
    "https://developer.microsoft.com/microsoft-edge/webview2/#download-section";

/// Oldest runtime exercised by this project's recorded native checks (docs/VALIDATION.md).
/// This is a compatibility support floor, not an assertion that this version has no vulnerabilities.
pub const MINIMUM_SUPPORTED_RUNTIME: &str = "151.0.4129.107";
const MINIMUM_RUNTIME_COMPONENTS: [u32; 4] = [151, 0, 4129, 107];
const MAX_VERSION_BYTES: usize = 64;

/// Read-only preflight result suitable for startup diagnostics and support reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInfo {
    pub version: String,
    pub minimum_supported_version: &'static str,
}

/// Separates a missing/old runtime from policy or discovery failures an installer must not mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeInspectionError {
    InstallationRequired(String),
    Blocked(String),
}

impl fmt::Display for RuntimeInspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstallationRequired(message) | Self::Blocked(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for RuntimeInspectionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RuntimeVersion([u32; 4]);

impl fmt::Display for RuntimeVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, build, patch] = self.0;
        write!(formatter, "{major}.{minor}.{build}.{patch}")
    }
}

/// Requires a discoverable WebView2 runtime without creating a browser or user-data folder.
///
/// Rejects supported loader overrides before calling the loader. Call before any session mutation.
/// This is a configuration check, not a boundary against a process that can tamper with this host.
pub fn ensure_webview2_runtime_available() -> Result<(), String> {
    let runtime = inspect_webview2_runtime()?;
    eprintln!(
        "[SafeBrowse] WebView2 Runtime {}; minimum supported {}. Restart to use installed runtime updates.",
        runtime.version, runtime.minimum_supported_version
    );
    Ok(())
}

/// Inspects environment, policy and loader discovery without creating a browser or profile.
/// No process-global environment values, registry settings, or installed runtimes are modified.
pub fn inspect_webview2_runtime() -> Result<RuntimeInfo, String> {
    inspect_with(overrides::reject_runtime_overrides, probe_runtime_version)
}

/// Read-only installer preflight; only InstallationRequired permits automatic runtime setup.
pub fn inspect_webview2_runtime_for_installation() -> Result<RuntimeInfo, RuntimeInspectionError> {
    inspect_classified_with(overrides::reject_runtime_overrides, probe_runtime_version)
}

/// Checks the created runtime and resolved storage directory before loading any document.
///
/// Call for every trusted or website environment, including native popup children. Loader discovery
/// can differ from creation (for example after an update or when reusing an existing environment).
/// This check does not authenticate runtime binaries or prevent concurrent same-user tampering.
pub fn validate_created_environment(
    view: &WebView,
    expected_profile_path: &Path,
) -> Result<RuntimeInfo, String> {
    let environment = view.environment();
    let mut version = PWSTR::null();
    unsafe { environment.BrowserVersionString(&mut version) }
        .map_err(|error| format!("Cannot verify the active WebView2 Runtime version: {error}"))?;
    let runtime = validate_runtime_version(&take_pwstr(version))?;
    let storage = environment
        .cast::<ICoreWebView2Environment7>()
        .map_err(|error| format!("Cannot verify the active WebView2 profile directory: {error}"))?;
    let mut directory = PWSTR::null();
    unsafe { storage.UserDataFolder(&mut directory) }
        .map_err(|error| format!("Cannot read the active WebView2 profile directory: {error}"))?;
    let actual_profile_path = take_pwstr(directory);
    validate_profile_directory(expected_profile_path, Path::new(&actual_profile_path))?;
    Ok(runtime)
}

fn validate_profile_directory(expected: &Path, actual: &Path) -> Result<(), String> {
    if !expected.is_absolute() || !actual.is_absolute() {
        return Err("WebView2 profile verification requires absolute directories".into());
    }
    let expected = expected.canonicalize().map_err(|error| {
        format!("Cannot resolve the intended WebView2 profile directory: {error}")
    })?;
    let actual = actual.canonicalize().map_err(|error| {
        format!("Cannot resolve the active WebView2 profile directory: {error}")
    })?;
    if expected != actual {
        return Err("WebView2 selected a different profile directory from the one reserved by SafeBrowse. Close SafeBrowse and check WebView2 runtime policies before restarting.".into());
    }
    Ok(())
}

/// Keeps discovery behind the configuration gate, including when the loader itself is unavailable.
fn inspect_with(
    check_configuration: impl FnOnce() -> Result<(), String>,
    probe: impl FnOnce() -> Result<Option<String>, Error>,
) -> Result<RuntimeInfo, String> {
    inspect_classified_with(check_configuration, probe).map_err(|error| error.to_string())
}

fn inspect_classified_with(
    check_configuration: impl FnOnce() -> Result<(), String>,
    probe: impl FnOnce() -> Result<Option<String>, Error>,
) -> Result<RuntimeInfo, RuntimeInspectionError> {
    check_configuration().map_err(RuntimeInspectionError::Blocked)?;
    let version = classify_runtime_probe_for_installation(probe())?;
    validate_runtime_version_for_installation(&version)
}

/// Requires four numeric components and the stable runtime channel, using numeric comparison.
/// Time: O(N). Space: O(1), bounded by MAX_VERSION_BYTES.
fn validate_runtime_version(version: &str) -> Result<RuntimeInfo, String> {
    validate_runtime_version_for_installation(version).map_err(|error| error.to_string())
}

fn validate_runtime_version_for_installation(
    version: &str,
) -> Result<RuntimeInfo, RuntimeInspectionError> {
    let parsed = parse_runtime_version(version).ok_or_else(|| {
        RuntimeInspectionError::Blocked(format!(
            "SafeBrowse requires the stable Microsoft Edge WebView2 Runtime with a valid four-part version. Preview channels and unrecognized version strings are not supported. Install or repair Evergreen WebView2 from:\n{WEBVIEW2_DOWNLOAD_URL}"
        ))
    })?;
    if parsed < RuntimeVersion(MINIMUM_RUNTIME_COMPONENTS) {
        return Err(RuntimeInspectionError::InstallationRequired(format!(
            "Microsoft Edge WebView2 Runtime {parsed} is older than SafeBrowse's supported minimum {MINIMUM_SUPPORTED_RUNTIME}. Update Evergreen WebView2 and restart SafeBrowse. This minimum is the project's tested compatibility baseline, not a guarantee of current security patches.\n\n{WEBVIEW2_DOWNLOAD_URL}"
        )));
    }
    Ok(RuntimeInfo {
        version: parsed.to_string(),
        minimum_supported_version: MINIMUM_SUPPORTED_RUNTIME,
    })
}

fn parse_runtime_version(version: &str) -> Option<RuntimeVersion> {
    if version.len() > MAX_VERSION_BYTES {
        return None;
    }
    let mut parts = version.split('.');
    let mut components = [0; 4];
    for component in &mut components {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        *component = part.parse().ok()?;
    }
    parts.next().is_none().then_some(RuntimeVersion(components))
}

/// Reads the loader's selected version and frees its COM allocation on every return path.
fn probe_runtime_version() -> Result<Option<String>, Error> {
    let mut version_pointer = PWSTR::null();
    let probe_result = unsafe {
        GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version_pointer)
    };
    let version_allocation = CoTaskMemPWSTR::from(version_pointer);
    probe_result?;
    Ok((!version_pointer.is_null()).then(|| version_allocation.to_string()))
}

/// Separates an absent runtime from failures that reinstall advice alone may not resolve.
#[cfg(test)]
fn classify_runtime_probe(probe_result: Result<Option<String>, Error>) -> Result<String, String> {
    classify_runtime_probe_for_installation(probe_result).map_err(|error| error.to_string())
}

fn classify_runtime_probe_for_installation(
    probe_result: Result<Option<String>, Error>,
) -> Result<String, RuntimeInspectionError> {
    match probe_result {
        Ok(Some(version)) if !version.trim().is_empty() => Ok(version),
        Ok(None) => Err(RuntimeInspectionError::InstallationRequired(missing_runtime_message())),
        Err(error) if error.code() == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) => {
            Err(RuntimeInspectionError::InstallationRequired(missing_runtime_message()))
        }
        Ok(Some(_)) => Err(RuntimeInspectionError::Blocked(format!(
            "Microsoft Edge WebView2 Runtime returned an empty version, so SafeBrowse could not verify that it is usable.\n\nRepair or reinstall the Evergreen WebView2 Runtime from:\n{WEBVIEW2_DOWNLOAD_URL}\n\nThen reopen SafeBrowse. See README.md for setup and troubleshooting."
        ))),
        Err(error) => Err(RuntimeInspectionError::Blocked(format!(
            "SafeBrowse could not check Microsoft Edge WebView2 Runtime. This may indicate a damaged installation or a Windows access restriction.\n\nDetails: {error} (HRESULT 0x{:08X})\n\nRepair or reinstall the Evergreen WebView2 Runtime from:\n{WEBVIEW2_DOWNLOAD_URL}\n\nIf this computer is managed, ask your administrator to check its WebView2 policies. See README.md for setup and troubleshooting.",
            error.code().0 as u32
        ))),
    }
}

/// Gives an installation path that remains readable in both console and native error dialogs.
fn missing_runtime_message() -> String {
    format!(
        "Microsoft Edge WebView2 Runtime was not found. SafeBrowse needs this browser component to start.\n\nInstall Microsoft's Evergreen WebView2 Runtime from:\n{WEBVIEW2_DOWNLOAD_URL}\n\nChoose the Evergreen Bootstrapper, or the Evergreen Standalone Installer for an offline computer. Then reopen SafeBrowse. See README.md for setup instructions."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_MOD_NOT_FOUND};

    #[test]
    fn absent_runtime_explains_installation_without_starting_a_browser() {
        for result in [
            Ok(None),
            Err(Error::from(HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0))),
        ] {
            let message = classify_runtime_probe(result).unwrap_err();
            assert!(message.contains("Microsoft Edge WebView2 Runtime was not found"));
            assert!(message.contains("Evergreen Bootstrapper"));
            assert!(message.contains(WEBVIEW2_DOWNLOAD_URL));
        }
    }

    #[test]
    fn discovery_failures_preserve_the_error_instead_of_claiming_absence() {
        for error_code in [ERROR_ACCESS_DENIED, ERROR_MOD_NOT_FOUND] {
            let hresult = HRESULT::from_win32(error_code.0);
            let message = classify_runtime_probe(Err(Error::from(hresult))).unwrap_err();
            assert!(message.contains("could not check Microsoft Edge WebView2 Runtime"));
            assert!(!message.contains("Runtime was not found"));
            assert!(message.contains(&format!("0x{:08X}", hresult.0 as u32)));
            assert!(message.contains(WEBVIEW2_DOWNLOAD_URL));
        }
        for version in ["", " \t\n"] {
            let message = classify_runtime_probe(Ok(Some(version.into()))).unwrap_err();
            assert!(message.contains("returned an empty version"));
            assert!(!message.contains("Runtime was not found"));
        }
    }

    #[test]
    fn support_policy_compares_numeric_versions_and_rejects_preview_channels() {
        for version in [
            MINIMUM_SUPPORTED_RUNTIME,
            "151.0.4129.108",
            "152.0.0.0",
            "1000.0.0.0",
        ] {
            assert_eq!(validate_runtime_version(version).unwrap().version, version);
        }
        for version in [
            "151.0.4129.106",
            "151.0.4128.999",
            "150.999.9999.9999",
            "99.0.9999.9999",
        ] {
            assert!(validate_runtime_version(version)
                .unwrap_err()
                .contains("older than"));
        }
        for version in [
            "151.0.4129.107 dev",
            "151.0.4129.107 beta",
            "151.0.4129.107 canary",
            "151.0.4129",
            "151.0.4129.107.1",
            "151..4129.107",
            "151.0.4129.-1",
            "151.0.4129.+107",
            "151.0.4129.4294967296",
            " 151.0.4129.107",
            "151.0.4129.107\n",
            "१५१.0.4129.107",
        ] {
            assert!(validate_runtime_version(version).is_err(), "{version}");
        }
    }

    #[test]
    fn rejected_configuration_never_reaches_loader_discovery() {
        let result = inspect_with(
            || Err("Blocked override".into()),
            || panic!("The runtime loader must not run under rejected configuration"),
        );
        assert_eq!(result.unwrap_err(), "Blocked override");
    }

    #[test]
    fn installer_classification_only_allows_setup_for_missing_or_old_runtime() {
        for probe in [
            Ok(None),
            Err(Error::from(HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0))),
            Ok(Some("150.0.1.0".into())),
        ] {
            assert!(matches!(
                inspect_classified_with(|| Ok(()), || probe),
                Err(RuntimeInspectionError::InstallationRequired(_))
            ));
        }
        for probe in [
            Ok(Some(String::new())),
            Ok(Some("151.0.4129.107 beta".into())),
            Err(Error::from(HRESULT::from_win32(ERROR_ACCESS_DENIED.0))),
        ] {
            assert!(matches!(
                inspect_classified_with(|| Ok(()), || probe),
                Err(RuntimeInspectionError::Blocked(_))
            ));
        }
        assert!(matches!(
            inspect_classified_with(
                || Err("blocked override".into()),
                || panic!("must not probe")
            ),
            Err(RuntimeInspectionError::Blocked(_))
        ));
        assert!(
            inspect_classified_with(|| Ok(()), || Ok(Some(MINIMUM_SUPPORTED_RUNTIME.into())))
                .is_ok()
        );
    }

    #[test]
    fn created_profile_must_resolve_to_the_intended_directory() {
        let directory =
            std::env::temp_dir().join(format!("safebrowse-runtime-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let other = directory.join("other");
        std::fs::create_dir(&other).unwrap();
        assert!(validate_profile_directory(&directory, &directory.join(".")).is_ok());
        assert!(validate_profile_directory(&directory, &other).is_err());
        assert!(validate_profile_directory(&directory, Path::new("relative")).is_err());
        assert!(validate_profile_directory(&directory, &directory.join("missing")).is_err());
        std::fs::remove_dir(&other).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }

    #[test]
    #[ignore = "requires a locally installed WebView2 Runtime; run explicitly on Windows"]
    fn installed_runtime_is_discoverable_without_creating_a_session() {
        let runtime = inspect_webview2_runtime()
            .expect("this native fixture requires an installed WebView2 Runtime");
        println!("WebView2 loader selected runtime {}", runtime.version);
    }
}
