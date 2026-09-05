//! Persistent, exact-origin website permissions owned by the browser shell.
//!
//! Callers serialize mutations through one store. A saved grant describes a user
//! choice, not whether the native runtime supports that capability.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::config::APP_IDENTIFIER;

const FILE_NAME: &str = "permissions.json";
const SCHEMA_VERSION: u32 = 1;
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_RULES: usize = 1024;
const MAX_ORIGIN_BYTES: usize = 2048;
const MAX_URL_BYTES: usize = 64 * 1024;

/// Website capabilities understood by the policy schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SitePermission {
    Popups,
    Camera,
    Microphone,
    Location,
    Notifications,
    ClipboardRead,
    OtherSensors,
    AutomaticDownloads,
    FileReadWrite,
    Autoplay,
    LocalFonts,
    MidiSystemExclusive,
    WindowManagement,
}

/// Ask leaves each new request for the user to decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Ask,
    Allow,
    Block,
}

/// One explicit choice for one capability at an exact HTTP(S) origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteRule {
    pub origin: String,
    pub permission: SitePermission,
    pub decision: PermissionDecision,
}

/// Stable, versioned representation used by settings and disk persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionSnapshot {
    pub version: u32,
    pub popup_default: PermissionDecision,
    #[serde(default = "default_download_policy")]
    pub downloads_default: PermissionDecision,
    #[serde(default)]
    pub printing_enabled: bool,
    pub site_rules: Vec<SiteRule>,
}

/// Older policy documents must not enable downloads without a per-download choice.
fn default_download_policy() -> PermissionDecision {
    PermissionDecision::Ask
}

#[derive(Clone)]
struct PolicyState {
    popup_default: PermissionDecision,
    downloads_default: PermissionDecision,
    printing_enabled: bool,
    rules: BTreeMap<String, BTreeMap<SitePermission, PermissionDecision>>,
}

impl Default for PolicyState {
    fn default() -> Self {
        Self {
            popup_default: PermissionDecision::Ask,
            downloads_default: default_download_policy(),
            printing_enabled: false,
            rules: BTreeMap::new(),
        }
    }
}

/// Holds validated policy and commits changes to disk before changing memory.
pub struct PermissionStore {
    storage_path: PathBuf,
    state: PolicyState,
}

impl PermissionStore {
    /// Opens the application configuration store, creating safe defaults if absent.
    pub fn initialize() -> Result<Self, String> {
        let directories = directories::ProjectDirs::from("com", "SafeBrowse", APP_IDENTIFIER)
            .ok_or("Cannot resolve the application configuration directory")?;
        Self::with_storage_path(directories.config_dir().join(FILE_NAME))
    }

    /// Opens an injected storage path. Invalid saved policy fails without replacing it.
    pub fn with_storage_path(storage_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = storage_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Cannot create permission directory: {error}"))?;
        }
        let state = match File::open(&storage_path) {
            Ok(file) => read_policy(file)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => PolicyState::default(),
            Err(error) => return Err(format!("Cannot open website permissions: {error}")),
        };
        let store = Self {
            storage_path,
            state,
        };
        if !store.storage_path.exists() {
            store.persist(&store.state)?;
        }
        Ok(store)
    }

    /// Returns deterministic settings data ordered by origin and capability.
    /// Time: O(R). Space: O(R), for R rules.
    pub fn snapshot(&self) -> PermissionSnapshot {
        snapshot_from_state(&self.state)
    }

    /// Resolves a rule at the requesting URL's exact origin; other capabilities ask by default.
    pub fn decision(
        &self,
        url: &str,
        permission: SitePermission,
    ) -> Result<PermissionDecision, String> {
        let origin = normalize_origin(url)?;
        Ok(self
            .state
            .rules
            .get(&origin)
            .and_then(|rules| rules.get(&permission))
            .copied()
            .unwrap_or(if permission == SitePermission::Popups {
                self.state.popup_default
            } else {
                PermissionDecision::Ask
            }))
    }

    /// Changes the global popup default while retaining explicit site overrides.
    pub fn set_popup_default(&mut self, decision: PermissionDecision) -> Result<(), String> {
        if self.state.popup_default == decision {
            return Ok(());
        }
        let mut next = self.state.clone();
        next.popup_default = decision;
        self.commit(next)
    }

    /// Returns the global policy for new file downloads without allocating a settings snapshot.
    pub fn download_policy(&self) -> PermissionDecision {
        self.state.downloads_default
    }

    /// Persists the policy for subsequent downloads before making the choice active in memory.
    pub fn set_download_policy(&mut self, decision: PermissionDecision) -> Result<(), String> {
        if self.state.downloads_default == decision {
            return Ok(());
        }
        let mut next = self.state.clone();
        next.downloads_default = decision;
        self.commit(next)
    }

    /// Returns whether native printing requests may be offered by the browser.
    pub fn printing_enabled(&self) -> bool {
        self.state.printing_enabled
    }

    /// Persists printing consent before making the choice active in memory.
    pub fn set_printing_enabled(&mut self, enabled: bool) -> Result<(), String> {
        if self.state.printing_enabled == enabled {
            return Ok(());
        }
        let mut next = self.state.clone();
        next.printing_enabled = enabled;
        self.commit(next)
    }

    /// Persists an explicit site rule, including Ask overrides of a permissive popup default.
    pub fn set_site_rule(
        &mut self,
        url: &str,
        permission: SitePermission,
        decision: PermissionDecision,
    ) -> Result<(), String> {
        let origin = normalize_origin(url)?;
        if self
            .state
            .rules
            .get(&origin)
            .and_then(|rules| rules.get(&permission))
            == Some(&decision)
        {
            return Ok(());
        }
        let mut next = self.state.clone();
        next.rules
            .entry(origin)
            .or_default()
            .insert(permission, decision);
        self.commit(next)
    }

    /// Revokes an explicit override, restoring its default. Returns false for an absent rule.
    pub fn remove_site_rule(
        &mut self,
        url: &str,
        permission: SitePermission,
    ) -> Result<bool, String> {
        let origin = normalize_origin(url)?;
        let Some(rules) = self.state.rules.get(&origin) else {
            return Ok(false);
        };
        if !rules.contains_key(&permission) {
            return Ok(false);
        }
        let mut next = self.state.clone();
        let rules = next
            .rules
            .get_mut(&origin)
            .expect("The cloned policy contains this origin");
        rules.remove(&permission);
        if rules.is_empty() {
            next.rules.remove(&origin);
        }
        self.commit(next)?;
        Ok(true)
    }

    /// Revokes all grants, restores Ask defaults and disables printing atomically.
    pub fn reset(&mut self) -> Result<(), String> {
        self.commit(PolicyState::default())
    }

    fn commit(&mut self, next: PolicyState) -> Result<(), String> {
        self.persist(&next)?;
        self.state = next;
        Ok(())
    }

    /// Stages on the same filesystem and atomically replaces the previous valid file.
    /// Time: O(R + B). Space: O(R + B), for R rules and B serialized bytes.
    fn persist(&self, state: &PolicyState) -> Result<(), String> {
        let snapshot = snapshot_from_state(state);
        if snapshot.site_rules.len() > MAX_RULES {
            return Err(format!(
                "Website permission limit reached ({MAX_RULES} rules)"
            ));
        }
        let bytes = serde_json::to_vec_pretty(&snapshot)
            .map_err(|error| format!("Cannot serialize website permissions: {error}"))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("Website permissions exceed the storage limit".into());
        }
        let temporary_path = self
            .storage_path
            .with_file_name(format!("{FILE_NAME}.tmp.{}", Uuid::new_v4()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)
                .map_err(|error| format!("Cannot stage website permissions: {error}"))?;
            file.write_all(&bytes)
                .map_err(|error| format!("Cannot write website permissions: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("Cannot flush website permissions: {error}"))?;
            drop(file);
            fs::rename(&temporary_path, &self.storage_path)
                .map_err(|error| format!("Cannot replace website permissions: {error}"))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }
}

/// Converts an HTTP(S) request URL to a canonical scheme/host/port origin.
/// Credentials, wildcards, opaque schemes and control characters are rejected.
pub fn normalize_origin(input: &str) -> Result<String, String> {
    if input.len() > MAX_URL_BYTES || input.chars().any(char::is_control) || input.contains('\\') {
        return Err("Invalid website permission address".into());
    }
    let url = Url::parse(input.trim())
        .map_err(|error| format!("Invalid website permission address: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Website permissions require an HTTP or HTTPS origin".into());
    }
    let (_, authority_and_path) = input
        .trim()
        .split_once("://")
        .ok_or("Website permission addresses must contain an explicit HTTP(S) authority")?;
    let authority = authority_and_path
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(authority_and_path);
    if !url.username().is_empty() || url.password().is_some() || authority.contains('@') {
        return Err("Website permission addresses must not contain credentials".into());
    }
    if url.host_str().is_some_and(|host| host.contains('*')) {
        return Err("Website permissions cannot use wildcard hosts".into());
    }
    let origin = url.origin().ascii_serialization();
    if origin.len() > MAX_ORIGIN_BYTES {
        return Err("Website permission origin is too long".into());
    }
    Ok(origin)
}

fn snapshot_from_state(state: &PolicyState) -> PermissionSnapshot {
    PermissionSnapshot {
        version: SCHEMA_VERSION,
        popup_default: state.popup_default,
        downloads_default: state.downloads_default,
        printing_enabled: state.printing_enabled,
        site_rules: state
            .rules
            .iter()
            .flat_map(|(origin, rules)| {
                rules.iter().map(|(&permission, &decision)| SiteRule {
                    origin: origin.clone(),
                    permission,
                    decision,
                })
            })
            .collect(),
    }
}

/// Validates the complete document before accepting any saved grants.
/// Time: O(B + R log R). Space: O(B + R), for B bytes and R rules.
fn read_policy(file: File) -> Result<PolicyState, String> {
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read website permissions: {error}"))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("Saved website permissions exceed the storage limit".into());
    }
    let saved: PermissionSnapshot = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid saved website permissions: {error}"))?;
    if saved.version != SCHEMA_VERSION {
        return Err(format!(
            "Unsupported website permissions schema version: {}",
            saved.version
        ));
    }
    if saved.site_rules.len() > MAX_RULES {
        return Err("Saved website permissions contain too many rules".into());
    }
    let mut state = PolicyState {
        popup_default: saved.popup_default,
        downloads_default: saved.downloads_default,
        printing_enabled: saved.printing_enabled,
        rules: BTreeMap::new(),
    };
    for rule in saved.site_rules {
        let origin = normalize_origin(&rule.origin)?;
        if origin != rule.origin {
            return Err("Saved website permission rules must contain canonical origins without paths or credentials".into());
        }
        if state
            .rules
            .entry(origin)
            .or_default()
            .insert(rule.permission, rule.decision)
            .is_some()
        {
            return Err(
                "Saved website permissions contain duplicate origin/capability rules".into(),
            );
        }
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::windows::fs::OpenOptionsExt;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("safebrowse-permissions-test-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> PathBuf {
            self.0.join(FILE_NAME)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn normalizes_exact_origins_without_accepting_credentials_or_wildcards() {
        assert_eq!(
            normalize_origin("HTTPS://ExAmPlE.com:443/account?token=abc#section").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            normalize_origin("http://[::1]:8080/page").unwrap(),
            "http://[::1]:8080"
        );
        assert_eq!(
            normalize_origin("https://example.com:8443/page").unwrap(),
            "https://example.com:8443"
        );
        for invalid in [
            "https://user@example.com",
            "https://@example.com",
            "https:@example.com",
            "https://example.com\\evil",
            "https://*.example.com",
            "https://%2A.example.com",
            "file:///tmp/a",
            "about:blank",
            "https://example.com\n",
        ] {
            assert!(normalize_origin(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn grants_do_not_cross_scheme_port_subdomain_or_capability() {
        let directory = TestDirectory::new();
        let mut store = PermissionStore::with_storage_path(directory.path()).unwrap();
        store
            .set_site_rule(
                "https://bank.example/authorize",
                SitePermission::Camera,
                PermissionDecision::Allow,
            )
            .unwrap();
        assert_eq!(
            store
                .decision("https://bank.example:443/other", SitePermission::Camera)
                .unwrap(),
            PermissionDecision::Allow
        );
        for isolated in [
            "http://bank.example",
            "https://bank.example:8443",
            "https://sub.bank.example",
            "https://bank.example.evil",
        ] {
            assert_eq!(
                store.decision(isolated, SitePermission::Camera).unwrap(),
                PermissionDecision::Ask
            );
        }
        assert_eq!(
            store
                .decision("https://bank.example", SitePermission::Microphone)
                .unwrap(),
            PermissionDecision::Ask
        );
    }

    #[test]
    fn popup_defaults_and_explicit_revocation_survive_reopening() {
        let directory = TestDirectory::new();
        let mut store = PermissionStore::with_storage_path(directory.path()).unwrap();
        store.set_popup_default(PermissionDecision::Allow).unwrap();
        store
            .set_site_rule(
                "https://example.com",
                SitePermission::Popups,
                PermissionDecision::Ask,
            )
            .unwrap();
        store
            .set_site_rule(
                "https://example.com",
                SitePermission::Camera,
                PermissionDecision::Allow,
            )
            .unwrap();
        let mut reopened = PermissionStore::with_storage_path(directory.path()).unwrap();
        assert_eq!(
            reopened
                .decision("https://example.com", SitePermission::Popups)
                .unwrap(),
            PermissionDecision::Ask
        );
        assert_eq!(
            reopened
                .decision("https://elsewhere.example", SitePermission::Popups)
                .unwrap(),
            PermissionDecision::Allow
        );
        assert!(reopened
            .remove_site_rule("https://example.com", SitePermission::Camera)
            .unwrap());
        assert!(!reopened
            .remove_site_rule("https://example.com", SitePermission::Camera)
            .unwrap());
        let mut revoked = PermissionStore::with_storage_path(directory.path()).unwrap();
        assert_eq!(
            revoked
                .decision("https://example.com", SitePermission::Camera)
                .unwrap(),
            PermissionDecision::Ask
        );
        revoked.reset().unwrap();
        let reset = PermissionStore::with_storage_path(directory.path())
            .unwrap()
            .snapshot();
        assert_eq!(reset.popup_default, PermissionDecision::Ask);
        assert!(reset.site_rules.is_empty());
    }

    #[test]
    fn failed_replacement_preserves_memory_disk_and_cleans_staging_files() {
        const EXCLUSIVE_FILE_ACCESS: u32 = 0;
        let directory = TestDirectory::new();
        let mut store = PermissionStore::with_storage_path(directory.path()).unwrap();
        store
            .set_site_rule(
                "https://example.com",
                SitePermission::Camera,
                PermissionDecision::Allow,
            )
            .unwrap();
        let expected = store.snapshot();
        let original_bytes = fs::read(directory.path()).unwrap();
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(EXCLUSIVE_FILE_ACCESS)
            .open(directory.path())
            .unwrap();
        assert!(store.set_popup_default(PermissionDecision::Allow).is_err());
        assert!(store
            .set_download_policy(PermissionDecision::Allow)
            .is_err());
        assert!(store.set_printing_enabled(true).is_err());
        assert!(store
            .set_site_rule(
                "https://example.com",
                SitePermission::Microphone,
                PermissionDecision::Allow
            )
            .is_err());
        assert!(store
            .remove_site_rule("https://example.com", SitePermission::Camera)
            .is_err());
        assert!(store.reset().is_err());
        assert_eq!(store.snapshot(), expected);
        drop(locked);
        assert_eq!(fs::read(directory.path()).unwrap(), original_bytes);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[test]
    fn invalid_saved_records_fail_without_silently_discarding_grants() {
        let directory = TestDirectory::new();
        let valid_rule =
            json!({"origin":"https://example.com", "permission":"camera", "decision":"allow"});
        let invalid_records = [
            json!({"version":2,"popup_default":"ask","site_rules":[]}),
            json!({"version":1,"popup_default":"trust_all","site_rules":[]}),
            json!({"version":1,"popup_default":"ask","downloads_default":"trust_all","site_rules":[]}),
            json!({"version":1,"popup_default":"ask","printing_enabled":"true","site_rules":[]}),
            json!({"version":1,"popup_default":"ask","site_rules":[],"unknown":true}),
            json!({"version":1,"popup_default":"ask","site_rules":[valid_rule.clone(),valid_rule]}),
            json!({"version":1,"popup_default":"ask","site_rules":[{"origin":"https://example.com/path","permission":"camera","decision":"allow"}]}),
            json!({"version":1,"popup_default":"ask","site_rules":[{"origin":"https://example.com","permission":"unknown_native_kind","decision":"allow"}]}),
            json!({"version":1,"popup_default":"ask","site_rules":[{"origin":"https://@example.com","permission":"camera","decision":"allow"}]}),
        ];
        for record in invalid_records {
            let bytes = serde_json::to_vec(&record).unwrap();
            fs::write(directory.path(), &bytes).unwrap();
            assert!(
                PermissionStore::with_storage_path(directory.path()).is_err(),
                "{record}"
            );
            assert_eq!(fs::read(directory.path()).unwrap(), bytes);
        }
        fs::write(directory.path(), vec![b' '; MAX_FILE_BYTES + 1]).unwrap();
        assert!(PermissionStore::with_storage_path(directory.path()).is_err());
    }

    #[test]
    fn legacy_permissions_preserve_grants_with_safe_download_and_printing_defaults() {
        let directory = TestDirectory::new();
        let legacy = json!({
            "version": 1,
            "popup_default": "allow",
            "site_rules": [{"origin":"https://example.com", "permission":"camera", "decision":"allow"}]
        });
        let original_bytes = serde_json::to_vec(&legacy).unwrap();
        fs::write(directory.path(), &original_bytes).unwrap();
        let mut store = PermissionStore::with_storage_path(directory.path()).unwrap();
        assert_eq!(store.download_policy(), PermissionDecision::Ask);
        assert!(!store.printing_enabled());
        assert_eq!(store.snapshot().popup_default, PermissionDecision::Allow);
        assert_eq!(
            store
                .decision("https://example.com", SitePermission::Camera)
                .unwrap(),
            PermissionDecision::Allow
        );
        assert_eq!(
            fs::read(directory.path()).unwrap(),
            original_bytes,
            "opening legacy settings must not rewrite existing choices"
        );
        store.set_printing_enabled(true).unwrap();
        let migrated = PermissionStore::with_storage_path(directory.path()).unwrap();
        assert!(migrated.printing_enabled());
        assert_eq!(migrated.download_policy(), PermissionDecision::Ask);
        assert_eq!(migrated.snapshot().site_rules, store.snapshot().site_rules);
    }

    #[test]
    fn download_and_printing_choices_persist_independently_and_reset_to_defaults() {
        let directory = TestDirectory::new();
        let mut store = PermissionStore::with_storage_path(directory.path()).unwrap();
        assert_eq!(store.download_policy(), PermissionDecision::Ask);
        assert!(!store.printing_enabled());
        for decision in [
            PermissionDecision::Block,
            PermissionDecision::Allow,
            PermissionDecision::Ask,
        ] {
            store.set_download_policy(decision).unwrap();
            let reopened = PermissionStore::with_storage_path(directory.path()).unwrap();
            assert_eq!(reopened.download_policy(), decision);
            assert!(!reopened.printing_enabled());
        }
        store
            .set_download_policy(PermissionDecision::Allow)
            .unwrap();
        store.set_printing_enabled(true).unwrap();
        let reopened = PermissionStore::with_storage_path(directory.path()).unwrap();
        assert!(reopened.printing_enabled());
        assert_eq!(reopened.download_policy(), PermissionDecision::Allow);
        store.set_printing_enabled(false).unwrap();
        assert!(!PermissionStore::with_storage_path(directory.path())
            .unwrap()
            .printing_enabled());
        store.set_printing_enabled(true).unwrap();
        store.reset().unwrap();
        let reset = PermissionStore::with_storage_path(directory.path()).unwrap();
        assert_eq!(reset.download_policy(), PermissionDecision::Ask);
        assert!(!reset.printing_enabled());
    }
}
