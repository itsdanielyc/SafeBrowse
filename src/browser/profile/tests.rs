//! Native storage fixtures use only fresh injected roots and disposable data.

use super::*;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use uuid::Uuid;

pub(super) struct Fixture(PathBuf);

impl Fixture {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!("SafeBrowse_Profile_Test_{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub(super) fn root(&self) -> PathBuf {
        self.0.join("owned-profiles")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let path = fs::canonicalize(&self.0).unwrap();
        let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(path.parent() == Some(temp.as_path()));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("SafeBrowse_Profile_Test_"));
        fs::remove_dir_all(path).unwrap();
    }
}

/// Simulates OS handle closure without running ProfileManager's cleanup destructor.
fn abandon(mut profile: ProfileManager) -> PathBuf {
    let path = profile.data_directory.clone();
    let state = profile.cleanup.get_mut().unwrap();
    state.lease.take();
    state.last_result = Some(Ok(()));
    path
}

#[test]
fn abandoned_profiles_are_reclaimed_but_active_unknown_legacy_and_persistent_survive() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let active = ProfileManager::new_ephemeral_at(&root).unwrap();
    fs::write(active.data_directory().join("live-cookie"), b"live-fixture").unwrap();
    let abandoned = ProfileManager::new_ephemeral_at(&root).unwrap();
    fs::create_dir(abandoned.data_directory().join("cache")).unwrap();
    fs::write(
        abandoned.data_directory().join("cache/cookie"),
        b"discard-fixture",
    )
    .unwrap();
    let abandoned_path = abandon(abandoned);
    let unmarked = root.join(format!(
        "{}{}",
        crate::config::EPHEMERAL_DIR_PREFIX,
        Uuid::new_v4()
    ));
    let persistent = root.join(PERSISTENT_PROFILE_DIR_NAME);
    let legacy = fixture.0.join(format!(
        "{}{}",
        crate::config::EPHEMERAL_DIR_PREFIX,
        Uuid::new_v4()
    ));
    for path in [&unmarked, &persistent, &legacy] {
        fs::create_dir(path).unwrap();
        fs::write(path.join("keep"), b"unrelated-fixture").unwrap();
    }
    let report = reclaim_at(&root, Duration::from_secs(3)).unwrap();
    assert_eq!(report.reclaimed, 1, "{report:?}");
    assert!(report.skipped >= 3);
    assert!(report.failures.is_empty(), "{report:?}");
    assert!(!abandoned_path.exists());
    assert_eq!(
        fs::read(active.data_directory().join("live-cookie")).unwrap(),
        b"live-fixture"
    );
    for path in [&unmarked, &persistent, &legacy] {
        assert!(path.join("keep").exists());
    }
    drop(active);
}

#[test]
fn wrong_missing_and_oversized_markers_are_never_reclaimed() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let first = abandon(ProfileManager::new_ephemeral_at(&root).unwrap());
    let second = abandon(ProfileManager::new_ephemeral_at(&root).unwrap());
    let third = abandon(ProfileManager::new_ephemeral_at(&root).unwrap());
    let valid_marker = fs::read(first.join(storage::SESSION_MARKER_NAME)).unwrap();
    fs::write(second.join(storage::SESSION_MARKER_NAME), valid_marker).unwrap();
    fs::write(first.join(storage::SESSION_MARKER_NAME), vec![b'x'; 256]).unwrap();
    fs::remove_file(third.join(storage::SESSION_MARKER_NAME)).unwrap();
    let report = reclaim_at(&root, Duration::from_secs(3)).unwrap();
    assert_eq!(report.reclaimed, 0);
    assert_eq!(report.skipped, 3);
    assert!(first.exists() && second.exists() && third.exists());
}

#[test]
fn root_creation_is_safe_when_profile_constructors_race() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let root = root.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                ProfileManager::new_ephemeral_at(&root)
            })
        })
        .collect();
    let profiles: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap())
        .collect();
    assert_eq!(
        reclaim_at(&root, Duration::from_secs(3)).unwrap().skipped,
        4
    );
    drop(profiles);
}

#[test]
fn cleanup_errors_remain_retryable_without_repeating_the_delay_on_drop() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let profile = ProfileManager::new_ephemeral_at(&root).unwrap();
    let path = profile.data_directory().to_owned();
    let locked = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(path.join("locked-cookie"))
        .unwrap();
    let error = profile
        .purge_with_timeout(Duration::from_millis(30))
        .unwrap_err();
    assert!(error.contains("Temporary browser data remains"));
    assert!(profile
        .cleanup
        .lock()
        .unwrap()
        .last_result
        .as_ref()
        .unwrap()
        .is_err());
    let dropping = Instant::now();
    drop(profile);
    assert!(
        dropping.elapsed() < Duration::from_secs(1),
        "Drop repeated the cleanup wait"
    );
    assert!(path.join(storage::SESSION_MARKER_NAME).exists());
    drop(locked);
    let report = reclaim_at(&root, Duration::from_secs(3)).unwrap();
    assert_eq!(report.reclaimed, 1, "{report:?}");
    assert!(!path.exists());
}

#[test]
fn expired_scan_budget_and_unowned_root_do_not_delete_data() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let abandoned = abandon(ProfileManager::new_ephemeral_at(&root).unwrap());
    let report = reclaim_at(&root, Duration::ZERO).unwrap();
    assert!(report.limit_reached);
    assert_eq!(report.reclaimed, 0);
    assert!(abandoned.exists());
    let unowned = fixture.0.join("unowned");
    fs::create_dir(&unowned).unwrap();
    fs::write(unowned.join("keep"), b"outside").unwrap();
    assert!(reclaim_at(&unowned, Duration::from_secs(3)).is_err());
    assert!(unowned.join("keep").exists());
    assert_eq!(
        reclaim_at(&fixture.0.join("missing"), Duration::from_secs(3)).unwrap(),
        EphemeralCleanupReport::default()
    );
}

/// Creates a directory junction using the documented mount-point reparse format.
/// Both endpoints are disposable fixture paths; no privilege or shell is required.
fn create_junction(link: &Path, destination: &Path) {
    use windows::Win32::Foundation::{GENERIC_WRITE, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    use windows::Win32::System::IO::DeviceIoControl;
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
    let print_name: Vec<u16> = destination
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .collect();
    let substitute_name: Vec<u16> = format!("\\??\\{}", destination.display())
        .encode_utf16()
        .collect();
    let substitute_bytes = substitute_name.len() * 2;
    let print_bytes = print_name.len() * 2;
    let data_bytes = 8 + substitute_bytes + 2 + print_bytes + 2;
    let mut record = Vec::with_capacity(8 + data_bytes);
    record.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    for value in [
        data_bytes as u16,
        0,
        0,
        substitute_bytes as u16,
        (substitute_bytes + 2) as u16,
        print_bytes as u16,
    ] {
        record.extend_from_slice(&value.to_le_bytes());
    }
    for value in substitute_name
        .into_iter()
        .chain(Some(0))
        .chain(print_name)
        .chain(Some(0))
    {
        record.extend_from_slice(&value.to_le_bytes());
    }
    fs::create_dir(link).unwrap();
    let directory: File = OpenOptions::new()
        .access_mode(GENERIC_WRITE.0)
        .share_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(link)
        .unwrap();
    let mut bytes_returned = 0;
    unsafe {
        DeviceIoControl(
            HANDLE(directory.as_raw_handle()),
            FSCTL_SET_REPARSE_POINT,
            Some(record.as_ptr().cast()),
            record.len() as u32,
            None,
            0,
            Some(&mut bytes_returned),
            None,
        )
    }
    .unwrap();
}

#[test]
fn reparse_roots_sessions_and_nested_paths_never_reach_outside_data() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let outside = fixture.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"outside-fixture").unwrap();
    let nested = abandon(ProfileManager::new_ephemeral_at(&root).unwrap());
    create_junction(&nested.join("cache-junction"), &outside);
    let session_link = root.join(format!(
        "{}{}",
        crate::config::EPHEMERAL_DIR_PREFIX,
        Uuid::new_v4()
    ));
    create_junction(&session_link, &outside);
    let report = reclaim_at(&root, Duration::from_secs(3)).unwrap();
    assert_eq!(report.reclaimed, 0);
    assert_eq!(report.skipped, 1);
    assert_eq!(report.failures.len(), 1);
    assert!(nested.join(storage::SESSION_MARKER_NAME).exists());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"outside-fixture");
    let root_link = fixture.0.join("root-junction");
    create_junction(&root_link, &root);
    assert!(reclaim_at(&root_link, Duration::from_secs(3)).is_err());
    assert!(ProfileManager::new_ephemeral_at(&root_link.join("nested-root")).is_err());
    assert!(!root.join("nested-root").exists());
    for link in [&root_link, &session_link, &nested.join("cache-junction")] {
        fs::remove_dir(link).unwrap();
    }
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"outside-fixture");
}

#[test]
fn multiply_linked_files_are_left_for_manual_review() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let outside = fixture.0.join("outside-cookie");
    fs::write(&outside, b"outside-fixture").unwrap();
    let path = abandon(ProfileManager::new_ephemeral_at(&root).unwrap());
    fs::hard_link(&outside, path.join("hard-linked-cookie")).unwrap();
    let report = reclaim_at(&root, Duration::from_secs(3)).unwrap();
    assert_eq!(report.reclaimed, 0);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(fs::read(&outside).unwrap(), b"outside-fixture");
    assert!(path.join(storage::SESSION_MARKER_NAME).exists());
}

#[test]
fn uninstall_reports_active_and_unrecognized_profiles_and_keeps_legacy_data() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let active = ProfileManager::new_ephemeral_at(&root).unwrap();
    fs::write(active.data_directory().join("live-cookie"), b"live").unwrap();
    let abandoned = abandon(ProfileManager::new_ephemeral_at(&root).unwrap());
    fs::write(abandoned.join("cookie"), b"abandoned").unwrap();
    let unrecognized = root.join("unrecognized");
    fs::create_dir(&unrecognized).unwrap();
    fs::write(unrecognized.join("keep"), b"unknown").unwrap();
    let legacy = fixture.0.join(format!(
        "{}{}",
        crate::config::EPHEMERAL_DIR_PREFIX,
        Uuid::new_v4()
    ));
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("keep"), b"legacy").unwrap();
    let report = reclaim_for_uninstall_at(&root, Duration::from_secs(3)).unwrap();
    assert_eq!(report.reclaimed, 1);
    assert_eq!(report.failures.len(), 2, "{report:?}");
    assert!(!abandoned.exists());
    assert!(active.data_directory().join("live-cookie").exists());
    assert_eq!(fs::read(unrecognized.join("keep")).unwrap(), b"unknown");
    assert_eq!(fs::read(legacy.join("keep")).unwrap(), b"legacy");
}
