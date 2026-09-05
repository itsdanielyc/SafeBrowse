//! Deterministic races between directory enumeration and opening a cleanup child.

use super::*;
use crate::browser::profile::tests::Fixture;

const TEST_CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn vanished_enumerated_children_do_not_prevent_complete_profile_cleanup() {
    let fixture = Fixture::new();
    let (path, mut lease) = SessionLease::create(&fixture.root()).unwrap();
    fs::write(path.join("closing-cache-file"), b"disposable").unwrap();
    fs::create_dir(path.join("closing-cache-directory")).unwrap();
    fs::write(path.join("remaining-cookie"), b"must also be removed").unwrap();
    let vanished_entries: Vec<_> = fs::read_dir(&path)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("closing-"))
        .collect();
    assert_eq!(vanished_entries.len(), 2);
    let mut budget = CleanupBudget::new(Instant::now() + TEST_CLEANUP_TIMEOUT);
    for entry in vanished_entries {
        if entry.file_type().unwrap().is_dir() {
            fs::remove_dir(entry.path()).unwrap();
        } else {
            fs::remove_file(entry.path()).unwrap();
        }
        delete_child(&entry.path(), &mut budget, 0)
            .expect("a child removed after enumeration must not abort cleanup");
    }
    lease.purge(&path, &mut budget).unwrap();
    drop(lease);
    assert!(!path.exists(), "the full session directory must be removed");
}

#[test]
fn enumerated_child_replaced_by_a_hard_link_is_still_rejected() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let outside = root.parent().unwrap().join("outside-sentinel");
    fs::write(&outside, b"must survive").unwrap();
    let (path, mut lease) = SessionLease::create(&root).unwrap();
    let child_path = path.join("replaced-cache-file");
    fs::write(&child_path, b"disposable").unwrap();
    let entry = fs::read_dir(&path)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| entry.path() == child_path)
        .unwrap();
    fs::remove_file(entry.path()).unwrap();
    fs::hard_link(&outside, entry.path()).unwrap();
    let mut budget = CleanupBudget::new(Instant::now() + TEST_CLEANUP_TIMEOUT);
    let error = delete_child(&entry.path(), &mut budget, 0).unwrap_err();
    assert!(error.to_string().contains("multiply linked"));
    assert_eq!(fs::read(&outside).unwrap(), b"must survive");
    assert!(path.join(SESSION_MARKER_NAME).exists());
    fs::remove_file(entry.path()).unwrap();
    lease.purge(&path, &mut budget).unwrap();
    drop(lease);
    assert!(!path.exists());
    assert_eq!(fs::read(&outside).unwrap(), b"must survive");
}

#[test]
fn uninstall_removes_only_an_empty_verified_root() {
    let fixture = Fixture::new();
    let root = fixture.root();
    drop(OwnedRoot::open(&root, true).unwrap().unwrap());
    assert!(remove_empty_owned_root(&root).unwrap());
    assert!(!root.exists());
    assert!(!remove_empty_owned_root(&root).unwrap());

    drop(OwnedRoot::open(&root, true).unwrap().unwrap());
    fs::write(root.join("unexpected-child"), b"must survive").unwrap();
    assert!(!remove_empty_owned_root(&root).unwrap());
    assert_eq!(
        fs::read(root.join(ROOT_MARKER_NAME)).unwrap(),
        ROOT_MARKER_CONTENT
    );
    assert_eq!(
        fs::read(root.join("unexpected-child")).unwrap(),
        b"must survive"
    );
}

#[test]
fn uninstall_restores_root_ownership_after_failed_final_delete() {
    let fixture = Fixture::new();
    let root = fixture.root();
    drop(OwnedRoot::open(&root, true).unwrap().unwrap());
    let error = remove_empty_owned_root_with(&root, |_| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "injected directory lock",
        ))
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        fs::read(root.join(ROOT_MARKER_NAME)).unwrap(),
        ROOT_MARKER_CONTENT
    );
    assert!(remove_empty_owned_root(&root).unwrap());
}

#[test]
fn late_child_during_empty_root_removal_survives_with_restored_marker() {
    let fixture = Fixture::new();
    let root = fixture.root();
    drop(OwnedRoot::open(&root, true).unwrap().unwrap());
    remove_empty_owned_root_with(&root, |directory| {
        fs::write(root.join("late-child"), b"must survive")?;
        mark_for_deletion(directory)
    })
    .unwrap_err();
    assert_eq!(
        fs::read(root.join(ROOT_MARKER_NAME)).unwrap(),
        ROOT_MARKER_CONTENT
    );
    assert_eq!(fs::read(root.join("late-child")).unwrap(), b"must survive");
}

#[test]
fn uninstall_refuses_live_root_handles_and_multiply_linked_ownership_markers() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let lease = OwnedRoot::open(&root, true).unwrap().unwrap();
    assert!(remove_empty_owned_root(&root).is_err());
    drop(lease);
    let outside_marker = root.parent().unwrap().join("outside-marker");
    fs::hard_link(root.join(ROOT_MARKER_NAME), &outside_marker).unwrap();
    assert!(remove_empty_owned_root(&root).is_err());
    assert_eq!(
        fs::read(root.join(ROOT_MARKER_NAME)).unwrap(),
        ROOT_MARKER_CONTENT
    );
    assert_eq!(fs::read(outside_marker).unwrap(), ROOT_MARKER_CONTENT);
}
