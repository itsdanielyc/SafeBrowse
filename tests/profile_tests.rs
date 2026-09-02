//! Unit and Integration Tests for Profile Sandbox and Ephemeral Auto-Purge

use safebrowse::browser::{ProfileManager, ProfileMode};
use std::fs;

#[test]
fn test_ephemeral_profile_lifecycle() {
    let profile = ProfileManager::new(ProfileMode::Ephemeral).expect("Failed to create ephemeral profile");
    let dir = profile.data_directory().to_path_buf();

    // Verify directory was created in %TEMP%
    assert!(dir.exists(), "Ephemeral directory should exist on creation");
    let dir_name = dir.file_name().unwrap().to_string_lossy();
    assert!(
        dir_name.starts_with("SafeBrowse_Session_"),
        "Ephemeral directory must have prefix SafeBrowse_Session_"
    );

    // Simulate browser writing cache and cookie files
    let dummy_cookie_file = dir.join("Cookies.db");
    fs::write(&dummy_cookie_file, b"encrypted_cookie_mock_data").expect("Failed to write mock data");
    assert!(dummy_cookie_file.exists());

    // Explicitly purge storage
    let purge_res = profile.purge_ephemeral_storage();
    assert!(purge_res.is_ok(), "Purge should succeed");
    assert!(
        !dir.exists(),
        "Ephemeral directory must be completely wiped from disk after purge"
    );
}

#[test]
fn test_persistent_profile_creation() {
    let profile = ProfileManager::new(ProfileMode::Persistent).expect("Failed to create persistent profile");
    let dir = profile.data_directory();
    assert!(dir.exists(), "Persistent directory should exist");
    assert!(
        dir.to_string_lossy().contains("Profile_Persistent"),
        "Persistent profile path should include Profile_Persistent"
    );
}
