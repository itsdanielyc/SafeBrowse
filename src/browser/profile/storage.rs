//! Exact-handle deletion with pinned ancestors and versioned ownership records.

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf, Prefix};
use std::time::{Duration, Instant};
use uuid::Uuid;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    FileDispositionInfo, GetFileInformationByHandle, SetFileInformationByHandle,
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

use crate::config::EPHEMERAL_DIR_PREFIX;

const OWNED_ROOT_NAME: &str = "SafeBrowse_EphemeralProfiles_v1";
pub(super) const ROOT_MARKER_NAME: &str = ".safebrowse-profile-root";
const ROOT_MARKER_CONTENT: &[u8] = b"SafeBrowse ephemeral profile root v1\n";
pub(super) const SESSION_MARKER_NAME: &str = ".safebrowse-session-lock";
const SESSION_MARKER_HEADER: &str = "SafeBrowse ephemeral session v1\n";
const MAX_MARKER_BYTES: u64 = 128;
const MAX_CLEANUP_NODES: usize = 50_000;
const MAX_CLEANUP_DEPTH: usize = 64;
const ROOT_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(1);
const ROOT_PUBLICATION_RETRY: Duration = Duration::from_millis(10);

/// Pins every ancestor to prevent directory replacement during child operations.
#[derive(Debug)]
pub(super) struct OwnedRoot {
    path: PathBuf,
    _ancestors: Vec<File>,
    _marker: File,
}

impl OwnedRoot {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Existing roots must already contain the expected regular ownership marker.
    pub(super) fn open(path: &Path, create: bool) -> io::Result<Option<Self>> {
        let parent = path
            .parent()
            .ok_or_else(|| invalid_storage("Profile root has no parent"))?;
        let mut ancestors = pin_directory_chain(parent)?;
        let created = if create {
            match fs::create_dir(path) {
                Ok(()) => true,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
                Err(error) => return Err(error),
            }
        } else {
            false
        };
        let directory = match open_pinned_object(path, false) {
            Ok(handle) => require_directory(handle)?,
            Err(error) if !create && error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        ancestors.push(directory);
        if created {
            let mut marker = marker_options(false)
                .create_new(true)
                .open(path.join(ROOT_MARKER_NAME))?;
            marker.write_all(ROOT_MARKER_CONTENT)?;
            marker.sync_all()?;
        }
        let marker = open_root_marker(path, create)?;
        Ok(Some(Self {
            path: path.to_owned(),
            _ancestors: ancestors,
            _marker: marker,
        }))
    }
}

/// A concurrent creator publishes the marker while holding an exclusive writer.
/// Existing unmarked roots are never adopted, even after this bounded grace period.
fn open_root_marker(path: &Path, wait_for_creation: bool) -> io::Result<File> {
    let deadline = Instant::now()
        + if wait_for_creation {
            ROOT_PUBLICATION_TIMEOUT
        } else {
            Duration::ZERO
        };
    loop {
        let opened = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path.join(ROOT_MARKER_NAME));
        let mut marker = match opened {
            Ok(marker) => marker,
            Err(error)
                if (error.kind() == io::ErrorKind::NotFound
                    || is_transient_cleanup_error(&error))
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(ROOT_PUBLICATION_RETRY);
                continue;
            }
            Err(error) => return Err(error),
        };
        if read_marker(&mut marker)? != ROOT_MARKER_CONTENT {
            return Err(invalid_storage(
                "Temporary profile root has no valid ownership marker",
            ));
        }
        return Ok(marker);
    }
}

/// The directory denies rename/delete sharing; the marker denies all sharing.
/// Closing these handles on process death permits later abandoned-session recovery.
#[derive(Debug)]
pub(super) struct SessionLease {
    _root: OwnedRoot,
    directory: File,
    marker: Option<File>,
    identifier: Uuid,
}

impl SessionLease {
    pub(super) fn create(root_path: &Path) -> io::Result<(PathBuf, Self)> {
        let root = OwnedRoot::open(root_path, true)?
            .ok_or_else(|| invalid_storage("Profile root was not created"))?;
        let identifier = Uuid::new_v4();
        let path = root
            .path
            .join(format!("{EPHEMERAL_DIR_PREFIX}{identifier}"));
        fs::create_dir(&path)?;
        let directory = require_directory(open_pinned_object(&path, true)?)?;
        let marker = create_session_marker(&path, identifier)?;
        Ok((
            path,
            Self {
                _root: root,
                directory,
                marker: Some(marker),
                identifier,
            },
        ))
    }

    pub(super) fn reopen(root_path: &Path, path: &Path, identifier: Uuid) -> io::Result<Self> {
        let root = OwnedRoot::open(root_path, false)?
            .ok_or_else(|| invalid_storage("Profile root disappeared"))?;
        if path.parent() != Some(root.path())
            || path.file_name().and_then(session_identifier) != Some(identifier)
        {
            return Err(invalid_storage(
                "Session path does not match the owned root",
            ));
        }
        let directory = require_directory(open_pinned_object(path, true)?)?;
        let mut marker = marker_options(true).open(path.join(SESSION_MARKER_NAME))?;
        if read_marker(&mut marker)? != session_marker_content(identifier) {
            return Err(invalid_storage(
                "Session ownership marker does not match its directory",
            ));
        }
        Ok(Self {
            _root: root,
            directory,
            marker: Some(marker),
            identifier,
        })
    }

    pub(super) fn purge(&mut self, path: &Path, budget: &mut CleanupBudget) -> io::Result<()> {
        delete_children(path, budget, 0)?;
        if let Some(marker) = self.marker.as_ref() {
            mark_for_deletion(marker)?;
        }
        self.marker.take();
        if let Err(error) = mark_for_deletion(&self.directory) {
            // All payload data is gone. Restore identity if a remaining browser
            // handle prevents removing the empty directory, allowing later retry.
            self.marker = Some(create_session_marker(path, self.identifier)?);
            return Err(error);
        }
        Ok(())
    }
}

pub(crate) fn owned_root_path() -> PathBuf {
    std::env::temp_dir().join(OWNED_ROOT_NAME)
}

pub(super) fn session_identifier(name: &std::ffi::OsStr) -> Option<Uuid> {
    let text = name.to_str()?.strip_prefix(EPHEMERAL_DIR_PREFIX)?;
    let identifier = Uuid::parse_str(text).ok()?;
    (identifier.get_version_num() == 4 && identifier.to_string() == text).then_some(identifier)
}

/// Pins each ancestor before descending and rejects junctions or symlinks.
/// Time/space O(D), where D is the absolute path's component count.
fn pin_directory_chain(path: &Path) -> io::Result<Vec<File>> {
    let mut current = PathBuf::new();
    let mut handles = Vec::new();
    let mut components = path.components();
    match components.next() {
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
        {
            current.push(prefix.as_os_str())
        }
        _ => {
            return Err(invalid_storage(
                "Profile storage requires an absolute local drive path",
            ))
        }
    }
    if components.next() != Some(Component::RootDir) {
        return Err(invalid_storage(
            "Profile storage must not use drive-relative paths",
        ));
    }
    current.push(std::path::MAIN_SEPARATOR.to_string());
    handles.push(require_directory(open_pinned_object(&current, false)?)?);
    for component in components {
        if !matches!(component, Component::Normal(_)) {
            return Err(invalid_storage(
                "Profile storage cannot contain parent traversal",
            ));
        }
        current.push(component.as_os_str());
        handles.push(require_directory(open_pinned_object(&current, false)?)?);
    }
    Ok(handles)
}

fn marker_options(delete: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .access_mode(GENERIC_READ.0 | GENERIC_WRITE.0 | if delete { DELETE.0 } else { 0 })
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
    options
}

fn session_marker_content(identifier: Uuid) -> Vec<u8> {
    format!("{SESSION_MARKER_HEADER}{identifier}\n").into_bytes()
}

fn create_session_marker(directory: &Path, identifier: Uuid) -> io::Result<File> {
    let mut marker = marker_options(true)
        .create_new(true)
        .open(directory.join(SESSION_MARKER_NAME))?;
    marker.write_all(&session_marker_content(identifier))?;
    marker.sync_all()?;
    Ok(marker)
}

fn read_marker(marker: &mut File) -> io::Result<Vec<u8>> {
    let information = object_information(marker)?;
    if information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0
        || information.nNumberOfLinks != 1
    {
        return Err(invalid_storage(
            "Ownership marker must be a regular, singly linked file",
        ));
    }
    let mut bytes = Vec::new();
    marker.take(MAX_MARKER_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err(invalid_storage("Ownership marker is too large"));
    }
    Ok(bytes)
}

fn open_pinned_object(path: &Path, delete: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0 | if delete { DELETE.0 } else { 0 })
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    object_information(&file)?;
    Ok(file)
}

fn object_information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information) }
        .map_err(|_| io::Error::last_os_error())?;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(invalid_storage(
            "Refusing a reparse point in temporary profile storage",
        ));
    }
    Ok(information)
}

fn require_directory(file: File) -> io::Result<File> {
    if object_information(&file)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
        return Err(invalid_storage("Expected a temporary profile directory"));
    }
    Ok(file)
}

fn mark_for_deletion(file: &File) -> io::Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    }
    .map_err(|_| io::Error::last_os_error())
}

pub(super) struct CleanupBudget {
    deadline: Instant,
    visited: usize,
}

impl CleanupBudget {
    pub(super) fn new(deadline: Instant) -> Self {
        Self {
            deadline,
            visited: 0,
        }
    }
    pub(super) fn exhausted(&self) -> bool {
        Instant::now() >= self.deadline || self.visited >= MAX_CLEANUP_NODES
    }
    fn visit(&mut self, depth: usize) -> io::Result<()> {
        if self.exhausted() || depth > MAX_CLEANUP_DEPTH {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Temporary profile cleanup reached its work limit",
            ));
        }
        self.visited += 1;
        Ok(())
    }
}

/// Deletes exact opened objects while their ancestors stay pinned.
/// Time O(F), space O(D); files F and recursion depth D are both bounded.
fn delete_children(path: &Path, budget: &mut CleanupBudget, depth: usize) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        budget.visit(depth)?;
        let entry = entry?;
        if depth == 0 && entry.file_name() == SESSION_MARKER_NAME {
            continue;
        }
        let child_path = entry.path();
        if child_path.parent() != Some(path) {
            return Err(invalid_storage(
                "Temporary profile child escaped its parent",
            ));
        }
        delete_child(&child_path, budget, depth)?;
    }
    Ok(())
}

/// Removes one enumerated child while its parent remains pinned by the caller.
/// Time O(F), space O(D) for the child's bounded subtree.
fn delete_child(path: &Path, budget: &mut CleanupBudget, depth: usize) -> io::Result<()> {
    let child = match open_pinned_object(path, true) {
        Ok(child) => child,
        // WebView2 may remove a cache entry after enumeration during shutdown.
        // The pinned parent still has to be deleted successfully before cleanup succeeds.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let information = object_information(&child)?;
    if information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
        delete_children(path, budget, depth + 1)?;
    } else if information.nNumberOfLinks != 1 {
        return Err(invalid_storage(
            "Refusing a multiply linked temporary profile file",
        ));
    }
    mark_for_deletion(&child)
}

pub(super) fn is_transient_cleanup_error(error: &io::Error) -> bool {
    use windows::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_DIR_NOT_EMPTY, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION,
    };
    let Some(code) = error.raw_os_error() else {
        return false;
    };
    [
        ERROR_ACCESS_DENIED,
        ERROR_DIR_NOT_EMPTY,
        ERROR_LOCK_VIOLATION,
        ERROR_SHARING_VIOLATION,
    ]
    .iter()
    .any(|transient_error| transient_error.0 == code as u32)
}

/// Pins the full ancestor chain for a maintenance operation without creating directories.
/// Missing ancestors mean that the known target cannot contain any remaining data.
pub(crate) fn pin_existing_directory(path: &Path) -> io::Result<Option<Vec<File>>> {
    match pin_directory_chain(path) {
        Ok(handles) => Ok(Some(handles)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Deletes a known regular file or bounded directory tree using the browser's safety checks.
/// Callers supply only allowlisted application paths and hold the session mutex.
/// Time O(F), space O(D); both file count and depth are bounded by CleanupBudget.
pub(crate) fn remove_known_path(
    path: &Path,
    directory: bool,
    deadline: Instant,
) -> io::Result<bool> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid_storage("Cleanup target has no parent"))?;
    let Some(_ancestors) = pin_existing_directory(parent)? else {
        return Ok(false);
    };
    let file = match open_pinned_object(path, true) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let information = object_information(&file)?;
    let is_directory = information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
    if directory != is_directory {
        return Err(invalid_storage(
            "Cleanup target has an unexpected file type",
        ));
    }
    if is_directory {
        let mut budget = CleanupBudget::new(deadline);
        budget.visit(0)?;
        // The persistent profile has no reserved session marker: every child is payload.
        delete_children(path, &mut budget, 1)?;
    } else if information.nNumberOfLinks != 1 {
        return Err(invalid_storage(
            "Refusing a multiply linked application data file",
        ));
    }
    mark_for_deletion(&file)?;
    Ok(true)
}

/// Removes only an empty known application directory; unrelated children are never traversed.
pub(crate) fn remove_empty_known_directory(path: &Path) -> io::Result<bool> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid_storage("Cleanup directory has no parent"))?;
    let Some(_ancestors) = pin_existing_directory(parent)? else {
        return Ok(false);
    };
    let directory = match open_pinned_object(path, true) {
        Ok(file) => require_directory(file)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if fs::read_dir(path)?.next().transpose()?.is_some() {
        return Ok(false);
    }
    mark_for_deletion(&directory)?;
    Ok(true)
}

/// Removes only a verified, otherwise empty temporary root during uninstall.
/// Live leases prevent the delete-capable directory open; callers also hold the session mutex.
/// Time O(D), space O(D), for the pinned absolute path depth; at most two entries are examined.
pub(crate) fn remove_empty_owned_root(path: &Path) -> io::Result<bool> {
    remove_empty_owned_root_with(path, mark_for_deletion)
}

/// The final-delete seam permits deterministic lock/race fixtures without browser user data.
fn remove_empty_owned_root_with(
    path: &Path,
    delete_directory: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<bool> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid_storage("Temporary root has no parent"))?;
    let Some(_ancestors) = pin_existing_directory(parent)? else {
        return Ok(false);
    };
    let directory = match open_pinned_object(path, true) {
        Ok(file) => require_directory(file)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut marker = marker_options(true).open(path.join(ROOT_MARKER_NAME))?;
    if read_marker(&mut marker)? != ROOT_MARKER_CONTENT {
        return Err(invalid_storage(
            "Temporary profile root has no valid ownership marker",
        ));
    }
    let mut entries = fs::read_dir(path)?;
    let only_marker = entries
        .next()
        .transpose()?
        .is_some_and(|entry| entry.file_name() == ROOT_MARKER_NAME);
    if !only_marker || entries.next().transpose()?.is_some() {
        return Ok(false);
    }
    drop(entries);
    mark_for_deletion(&marker)?;
    drop(marker);
    if let Err(delete_error) = delete_directory(&directory) {
        // Keep crash-recovery ownership intact when a late child or native lock
        // prevents deleting the directory after its marker has been removed.
        let restore_result = (|| -> io::Result<()> {
            let mut restored = marker_options(false)
                .create_new(true)
                .open(path.join(ROOT_MARKER_NAME))?;
            restored.write_all(ROOT_MARKER_CONTENT)?;
            restored.sync_all()
        })();
        if let Err(restore_error) = restore_result {
            return Err(io::Error::new(
                delete_error.kind(),
                format!("Cannot remove temporary root: {delete_error}. Cannot restore its ownership marker: {restore_error}. The remaining directory needs manual inspection."),
            ));
        }
        return Err(delete_error);
    }
    Ok(true)
}

fn invalid_storage(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
