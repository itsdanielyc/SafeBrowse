//! Read-only rejection of WebView2 loader configuration outside the application's control.
//!
//! Sources: Microsoft's CreateCoreWebView2EnvironmentWithOptions contract and WebView2 policy
//! reference. Check both registry views and all applicable selectors; do not attempt to reproduce
//! loader precedence or temporarily rewrite process-global environment values.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    APPMODEL_ERROR_NO_APPLICATION, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
    E_FAIL,
};
use windows::Win32::Storage::Packaging::Appx::GetCurrentApplicationUserModelId;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_QUERY_VALUE, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_SAM_FLAGS,
};
use windows::Win32::UI::Shell::GetCurrentProcessExplicitAppUserModelID;

const BLOCKED_ENVIRONMENT_VARIABLES: &[&str] = &[
    "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER",
    "WEBVIEW2_USER_DATA_FOLDER",
    "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
    "WEBVIEW2_CHANNEL_SEARCH_KIND",
    "WEBVIEW2_RELEASE_CHANNELS",
    "WEBVIEW2_RELEASE_CHANNEL_PREFERENCE",
    "WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER",
    "WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER",
];
const POLICY_ROOT: &str = r"Software\Policies\Microsoft\Edge\WebView2";
const LEGACY_POLICY_ROOT: &str =
    r"Software\Policies\Microsoft\EmbeddedBrowserWebView\LoaderOverride";
const LOADER_POLICIES: &[&str] = &[
    "BrowserExecutableFolder",
    "UserDataFolder",
    "AdditionalBrowserArguments",
    "ChannelSearchKind",
    "ReleaseChannels",
    "ReleaseChannelPreference",
    "DowngradeVersion",
];
const LEGACY_LOADER_POLICIES: &[&str] = &[
    "browserExecutableFolder",
    "userDataFolder",
    "additionalBrowserArguments",
    "releaseChannelPreference",
];
const ALL_APPLICATIONS: &str = "*";
const APPLICATION_ID_BUFFER_UNITS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistryHive {
    Machine,
    User,
}

impl RegistryHive {
    fn handle(self) -> HKEY {
        match self {
            Self::Machine => HKEY_LOCAL_MACHINE,
            Self::User => HKEY_CURRENT_USER,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Machine => "HKLM",
            Self::User => "HKCU",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistryView {
    Bits64,
    Bits32,
}

impl RegistryView {
    fn flag(self) -> REG_SAM_FLAGS {
        match self {
            Self::Bits64 => KEY_WOW64_64KEY,
            Self::Bits32 => KEY_WOW64_32KEY,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Bits64 => "64-bit view",
            Self::Bits32 => "32-bit view",
        }
    }
}

/// Checks only documented loader/debugger inputs; values are never copied into diagnostics.
pub(super) fn reject_runtime_overrides() -> Result<(), String> {
    reject_environment_overrides(|name| std::env::var_os(name))?;
    let application_ids = application_policy_identifiers()?;
    reject_registry_overrides(&application_ids, registry_value_exists)
}

fn reject_environment_overrides(
    mut read_variable: impl FnMut(&str) -> Option<OsString>,
) -> Result<(), String> {
    for &name in BLOCKED_ENVIRONMENT_VARIABLES {
        if read_variable(name).is_some_and(|value| !value.is_empty()) {
            return Err(format!(
                "SafeBrowse cannot start while the WebView2 environment override {name} is set. Remove this override from the launch environment and reopen SafeBrowse. Runtime selection, profile redirection and browser debugging overrides are unsupported. No environment settings were changed."
            ));
        }
    }
    Ok(())
}

/// Enumerates a fixed policy set for this executable, its application identities, and wildcard.
/// Time: O(P * A). Space: O(L), for fixed P policies, A <= 4 selectors, and path length L.
fn reject_registry_overrides(
    application_ids: &[OsString],
    mut exists: impl FnMut(RegistryHive, RegistryView, &OsStr, &OsStr) -> Result<bool, String>,
) -> Result<(), String> {
    for hive in [RegistryHive::Machine, RegistryHive::User] {
        for view in [RegistryView::Bits64, RegistryView::Bits32] {
            for &policy in LOADER_POLICIES {
                let path = format!(r"{POLICY_ROOT}\{policy}");
                for application in application_ids {
                    // Microsoft's UserDataFolder and downgrade policies do not support '*'.
                    if application == ALL_APPLICATIONS
                        && matches!(policy, "UserDataFolder" | "DowngradeVersion")
                    {
                        continue;
                    }
                    reject_policy_value(hive, view, OsStr::new(&path), application, &mut exists)?;
                }
            }
            for application in application_ids {
                let mut path = OsString::from(LEGACY_POLICY_ROOT);
                path.push("\\");
                path.push(application);
                for &policy in LEGACY_LOADER_POLICIES {
                    reject_policy_value(hive, view, &path, OsStr::new(policy), &mut exists)?;
                }
            }
        }
    }
    Ok(())
}

fn reject_policy_value(
    hive: RegistryHive,
    view: RegistryView,
    path: &OsStr,
    name: &OsStr,
    exists: &mut impl FnMut(RegistryHive, RegistryView, &OsStr, &OsStr) -> Result<bool, String>,
) -> Result<(), String> {
    let location = format!(
        r"{}\{} [{}] ({})",
        hive.label(),
        path.to_string_lossy(),
        name.to_string_lossy(),
        view.label()
    );
    let present = exists(hive, view, path, name)
        .map_err(|error| format!("Cannot inspect WebView2 policy at {location}: {error}"))?;
    if present {
        return Err(format!(
            "SafeBrowse cannot start with a WebView2 loader override at {location}. This configuration can change browser arguments, runtime selection or profile storage and is unsupported, including empty or malformed policy values. Ask your administrator to remove the override for SafeBrowse. No registry settings were changed."
        ));
    }
    Ok(())
}

/// Includes packaged identity, any explicit Win32 identity, the actual filename, and '*'.
fn application_policy_identifiers() -> Result<Vec<OsString>, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Cannot identify this executable for WebView2 policy: {error}"))?;
    let file_name = executable
        .file_name()
        .ok_or("The executable has no filename for WebView2 policy checks")?
        .to_os_string();
    let mut identifiers = Vec::with_capacity(4);
    if let Some(identity) = packaged_application_id()? {
        identifiers.push(identity);
    }
    if let Some(identity) = explicit_application_id()? {
        if !identifiers.contains(&identity) {
            identifiers.push(identity);
        }
    }
    identifiers.push(file_name);
    identifiers.push(ALL_APPLICATIONS.into());
    Ok(identifiers)
}

fn packaged_application_id() -> Result<Option<OsString>, String> {
    let mut buffer = [0u16; APPLICATION_ID_BUFFER_UNITS];
    let mut length = buffer.len() as u32;
    let result =
        unsafe { GetCurrentApplicationUserModelId(&mut length, Some(PWSTR(buffer.as_mut_ptr()))) };
    if result == APPMODEL_ERROR_NO_APPLICATION {
        return Ok(None);
    }
    if result != ERROR_SUCCESS {
        return Err(format!(
            "Cannot inspect the process application identity (Windows error {})",
            result.0
        ));
    }
    let length = length as usize;
    if length < 2 || length > buffer.len() || buffer[length - 1] != 0 {
        return Err("Windows returned an invalid process application identity".into());
    }
    Ok(Some(OsString::from_wide(&buffer[..length - 1])))
}

fn explicit_application_id() -> Result<Option<OsString>, String> {
    let pointer = match unsafe { GetCurrentProcessExplicitAppUserModelID() } {
        Ok(pointer) => pointer,
        // Unpackaged processes without an explicitly assigned ID return E_FAIL.
        Err(error) if error.code() == E_FAIL => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Cannot inspect the explicit process application identity: {error}"
            ))
        }
    };
    if pointer.is_null() {
        return Err("Windows returned an empty explicit process application identity".into());
    }
    let identity = unsafe { OsString::from_wide(pointer.as_wide()) };
    unsafe { CoTaskMemFree(Some(pointer.0.cast())) };
    if identity.is_empty() {
        return Err("Windows returned an empty explicit process application identity".into());
    }
    Ok(Some(identity))
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// Queries value existence only; even malformed values fail closed without exposing their contents.
fn registry_value_exists(
    hive: RegistryHive,
    view: RegistryView,
    path: &OsStr,
    name: &OsStr,
) -> Result<bool, String> {
    let mut handle = HKEY::default();
    let path: Vec<u16> = path.encode_wide().chain(Some(0)).collect();
    let status = unsafe {
        RegOpenKeyExW(
            hive.handle(),
            PCWSTR(path.as_ptr()),
            None,
            KEY_QUERY_VALUE | view.flag(),
            &mut handle,
        )
    };
    if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
        return Ok(false);
    }
    if status != ERROR_SUCCESS {
        return Err(format!("Windows error {} opening policy key", status.0));
    }
    let key = RegistryKey(handle);
    let name: Vec<u16> = name.encode_wide().chain(Some(0)).collect();
    let status = unsafe { RegQueryValueExW(key.0, PCWSTR(name.as_ptr()), None, None, None, None) };
    match status {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        _ => Err(format!("Windows error {} reading policy value", status.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debugger_and_loader_environment_overrides_are_rejected_without_echoing_values() {
        for &blocked in BLOCKED_ENVIRONMENT_VARIABLES {
            let error = reject_environment_overrides(|name| {
                (name == blocked).then(|| OsString::from("private-secret-value"))
            })
            .unwrap_err();
            assert!(error.contains(blocked));
            assert!(!error.contains("private-secret-value"));
        }
        assert!(reject_environment_overrides(|_| None).is_ok());
        assert!(reject_environment_overrides(|_| Some(OsString::new())).is_ok());
        assert!(reject_environment_overrides(|name| {
            (name == "WEBVIEW2_LANGUAGE").then(|| "en-GB".into())
        })
        .is_ok());
    }

    #[test]
    fn applicable_overrides_are_checked_in_both_hives_and_registry_views() {
        let identifiers = vec![
            "SafeBrowse.Identity".into(),
            "renamed-browser.exe".into(),
            "*".into(),
        ];
        for target_hive in [RegistryHive::Machine, RegistryHive::User] {
            for target_view in [RegistryView::Bits64, RegistryView::Bits32] {
                for target_selector in &identifiers {
                    let error =
                        reject_registry_overrides(&identifiers, |hive, view, path, selector| {
                            Ok(hive == target_hive
                                && view == target_view
                                && path
                                    == OsStr::new(&format!(
                                        r"{POLICY_ROOT}\AdditionalBrowserArguments"
                                    ))
                                && selector == target_selector)
                        })
                        .unwrap_err();
                    assert!(error.contains(target_hive.label()));
                    assert!(error.contains(target_view.label()));
                    assert!(error.contains(target_selector.to_string_lossy().as_ref()));
                }
            }
        }
    }

    #[test]
    fn legacy_overrides_and_access_failures_are_not_silently_ignored() {
        let identifiers = vec!["safebrowse.exe".into(), "*".into()];
        let legacy = reject_registry_overrides(&identifiers, |_, _, path, value| {
            Ok(
                path == OsStr::new(&format!(r"{LEGACY_POLICY_ROOT}\safebrowse.exe"))
                    && value == "additionalBrowserArguments",
            )
        })
        .unwrap_err();
        assert!(legacy.contains("EmbeddedBrowserWebView"));
        let denied =
            reject_registry_overrides(&identifiers, |_, _, _, _| Err("access denied".into()))
                .unwrap_err();
        assert!(denied.contains("Cannot inspect"));
        assert!(denied.contains("access denied"));
        assert!(reject_registry_overrides(&identifiers, |_, _, _, _| Ok(false)).is_ok());
    }
}
