//! Native deletion fixtures use only fresh, injected directories and disposable sentinels.

use super::*;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("SafeBrowse_Maintenance_Test_{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn paths(&self) -> MaintenancePaths {
        MaintenancePaths {
            ephemeral_root: self.0.join("ephemeral"),
            config_dir: self.0.join("config"),
            data_dir: self.0.join("data"),
        }
    }

    fn populate(&self) -> MaintenancePaths {
        let paths = self.paths();
        fs::create_dir(&paths.config_dir).unwrap();
        fs::create_dir(&paths.data_dir).unwrap();
        for name in KNOWN_CONFIGURATION_FILES {
            fs::write(paths.config_dir.join(name), b"disposable configuration").unwrap();
            fs::write(
                paths
                    .config_dir
                    .join(format!("{name}.tmp.{}", Uuid::new_v4())),
                b"disposable staging file",
            )
            .unwrap();
        }
        let profile = paths.data_dir.join(PERSISTENT_PROFILE_DIR_NAME);
        fs::create_dir(&profile).unwrap();
        fs::write(profile.join("cookie"), b"disposable cookie").unwrap();
        // This name is not reserved inside a persistent profile and must also be removed.
        fs::write(
            profile.join(".safebrowse-session-lock"),
            b"disposable payload",
        )
        .unwrap();
        paths
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let root = self.0.canonicalize().unwrap();
        let temporary = std::env::temp_dir().canonicalize().unwrap();
        assert_eq!(root.parent(), Some(temporary.as_path()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("SafeBrowse_Maintenance_Test_"));
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn parser_rejects_arbitrary_paths_and_extra_arguments() {
    for arguments in [
        vec![],
        vec!["cleanup", "C:\\Users"],
        vec!["cleanup", "--remove-user-data", "extra"],
        vec!["check-runtime", "--remove-user-data"],
    ] {
        let arguments = arguments.into_iter().map(String::from).collect::<Vec<_>>();
        assert!(MaintenanceCommand::parse(&arguments).is_err());
    }
    assert_eq!(
        MaintenanceCommand::parse(&["cleanup".into()]).unwrap(),
        MaintenanceCommand::Cleanup {
            remove_user_data: false
        }
    );
    assert_eq!(
        MaintenanceCommand::parse(&["cleanup".into(), "--remove-user-data".into()]).unwrap(),
        MaintenanceCommand::Cleanup {
            remove_user_data: true
        }
    );
}

#[test]
fn absent_data_is_a_noop_without_creating_directories() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    cleanup_at(&paths, true, CLEANUP_TIMEOUT).unwrap();
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
}

#[test]
fn ordinary_uninstall_keeps_all_persistent_user_data() {
    let fixture = Fixture::new();
    let paths = fixture.populate();
    cleanup_at(&paths, false, CLEANUP_TIMEOUT).unwrap();
    assert_eq!(fs::read_dir(&paths.config_dir).unwrap().count(), 4);
    assert!(paths
        .data_dir
        .join(PERSISTENT_PROFILE_DIR_NAME)
        .join("cookie")
        .exists());
}

#[test]
fn opt_in_removes_known_data_and_empty_folders_but_keeps_downloads() {
    let fixture = Fixture::new();
    let paths = fixture.populate();
    let downloads = fixture.0.join("Downloads");
    fs::create_dir(&downloads).unwrap();
    fs::write(downloads.join("statement.pdf"), b"keep download").unwrap();
    cleanup_at(&paths, true, CLEANUP_TIMEOUT).unwrap();
    assert!(!paths.config_dir.exists());
    assert!(!paths.data_dir.exists());
    assert_eq!(
        fs::read(downloads.join("statement.pdf")).unwrap(),
        b"keep download"
    );
}

#[test]
fn unknown_config_files_and_malformed_staging_names_survive() {
    let fixture = Fixture::new();
    let paths = fixture.populate();
    for name in [
        "personal.txt",
        "bookmarks.json.tmp.invalid",
        "bookmarks.json.backup",
        "permissions.json.tmp.00000000-0000-0000-0000-000000000000",
    ] {
        fs::write(paths.config_dir.join(name), b"keep unrelated file").unwrap();
    }
    cleanup_at(&paths, true, CLEANUP_TIMEOUT).unwrap();
    assert_eq!(fs::read_dir(&paths.config_dir).unwrap().count(), 4);
    assert_eq!(
        fs::read(paths.config_dir.join("personal.txt")).unwrap(),
        b"keep unrelated file"
    );
}

#[test]
fn multiply_linked_configuration_and_profile_files_are_reported_and_preserved() {
    let fixture = Fixture::new();
    let paths = fixture.populate();
    let sentinel = fixture.0.join("outside-sentinel");
    fs::write(&sentinel, b"must survive").unwrap();
    let config = paths.config_dir.join(BOOKMARKS_FILE_NAME);
    fs::remove_file(&config).unwrap();
    fs::hard_link(&sentinel, &config).unwrap();
    let profile = paths.data_dir.join(PERSISTENT_PROFILE_DIR_NAME);
    fs::hard_link(&sentinel, profile.join("hard-link")).unwrap();
    let message = cleanup_at(&paths, true, CLEANUP_TIMEOUT).unwrap_err();
    assert!(message.contains("multiply linked"));
    assert!(config.exists());
    assert!(profile.join("hard-link").exists());
    assert_eq!(fs::read(sentinel).unwrap(), b"must survive");
}

#[test]
fn wrong_type_and_locked_known_files_fail_without_claiming_complete_removal() {
    let fixture = Fixture::new();
    let paths = fixture.populate();
    let config = paths.config_dir.join(BOOKMARKS_FILE_NAME);
    fs::remove_file(&config).unwrap();
    fs::create_dir(&config).unwrap();
    fs::write(config.join("sentinel"), b"keep unexpected directory").unwrap();
    let locked = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(paths.config_dir.join(PERMISSION_FILE_NAME))
        .unwrap();
    let message = cleanup_at(&paths, true, CLEANUP_TIMEOUT).unwrap_err();
    assert!(message.contains("unexpected file type"));
    assert!(message.contains(PERMISSION_FILE_NAME));
    assert!(config.join("sentinel").exists());
    drop(locked);
}

#[test]
fn expired_budget_preserves_known_data_and_reports_incomplete_cleanup() {
    let fixture = Fixture::new();
    let paths = fixture.populate();
    let message = cleanup_at(&paths, true, Duration::ZERO).unwrap_err();
    assert!(message.contains("work limit"));
    assert!(paths.config_dir.join(BOOKMARKS_FILE_NAME).exists());
    assert!(paths
        .data_dir
        .join(PERSISTENT_PROFILE_DIR_NAME)
        .join("cookie")
        .exists());
}

#[test]
fn a_live_session_lock_blocks_cleanup_and_releases_after_drop() {
    let name: Vec<u16> = format!("Local\\SafeBrowse_Maintenance_Test_{}", Uuid::new_v4())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let name = windows::core::PCWSTR(name.as_ptr());
    let first = MaintenanceLock::acquire(name).unwrap();
    assert!(MaintenanceLock::acquire(name).is_err());
    drop(first);
    assert!(MaintenanceLock::acquire(name).is_ok());
}

#[test]
fn runtime_exit_code_is_typed_and_never_inferred_from_error_wording() {
    let required = MaintenanceError::from(RuntimeInspectionError::InstallationRequired(
        "missing".into(),
    ));
    assert_eq!(required.exit_code, 10);
    let blocked = MaintenanceError::from(RuntimeInspectionError::Blocked(
        "Microsoft Edge WebView2 Runtime was not found".into(),
    ));
    assert_eq!(blocked.exit_code, 1);
}

/// Creates a disposable junction without elevation; the format is the Windows mount-point record.
fn create_junction(link: &Path, destination: &Path) {
    use windows::Win32::Foundation::GENERIC_WRITE;
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    use windows::Win32::System::IO::DeviceIoControl;
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
    let printable = destination
        .to_string_lossy()
        .encode_utf16()
        .collect::<Vec<_>>();
    let substitute = format!("\\??\\{}", destination.display())
        .encode_utf16()
        .collect::<Vec<_>>();
    let substitute_bytes = substitute.len() * 2;
    let print_bytes = printable.len() * 2;
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
    for value in substitute
        .into_iter()
        .chain(Some(0))
        .chain(printable)
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
fn reparse_ancestors_and_profile_children_never_reach_outside_sentinels() {
    let fixture = Fixture::new();
    let mut paths = fixture.populate();
    let outside = fixture.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join(BOOKMARKS_FILE_NAME), b"outside sentinel").unwrap();
    let config_link = fixture.0.join("config-junction");
    create_junction(&config_link, &outside);
    paths.config_dir = config_link.clone();
    let nested = paths
        .data_dir
        .join(PERSISTENT_PROFILE_DIR_NAME)
        .join("cache-junction");
    create_junction(&nested, &outside);
    let message = cleanup_at(&paths, true, CLEANUP_TIMEOUT).unwrap_err();
    assert!(message.contains("reparse point"));
    assert_eq!(
        fs::read(outside.join(BOOKMARKS_FILE_NAME)).unwrap(),
        b"outside sentinel"
    );
    fs::remove_dir(&config_link).unwrap();
    fs::remove_dir(&nested).unwrap();
}
