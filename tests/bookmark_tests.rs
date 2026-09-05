//! Unit and Integration Tests for Persistent Bookmarks Store

use safebrowse::bookmarks::{BookmarkCategory, BookmarkStore};
use uuid::Uuid;

#[test]
fn test_bookmark_store_initialization_and_defaults() {
    let test_dir = std::env::temp_dir().join(format!("SafeBrowse_Test_BM_{}", Uuid::new_v4()));
    let test_path = test_dir.join("bookmarks.json");

    let store =
        BookmarkStore::with_storage_path(test_path).expect("Failed to initialize bookmark store");
    let bookmarks = store.list();

    // Default bookmarks should be populated on first run
    assert!(!bookmarks.is_empty(), "Store should have default bookmarks");
    assert!(
        bookmarks.iter().any(|b| b.title.contains("DuckDuckGo")),
        "DuckDuckGo bookmark should be present"
    );
    assert!(
        bookmarks.iter().any(|b| b.title.contains("PayPal")),
        "PayPal banking bookmark should be present"
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn failed_bookmark_writes_preserve_memory_and_clean_staging_files() {
    let directory = std::env::temp_dir().join(format!("SafeBrowse_Test_BM_{}", Uuid::new_v4()));
    let path = directory.join("bookmarks.json");
    let mut store = BookmarkStore::with_storage_path(path.clone()).unwrap();
    let original_ids: Vec<_> = store
        .list()
        .iter()
        .map(|bookmark| bookmark.id.clone())
        .collect();

    // A directory at the destination forces atomic replacement to fail reliably.
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(store
        .add("Unsaved", "https://example.com", BookmarkCategory::General)
        .is_err());
    assert!(store.remove(&original_ids[0]).is_err());
    assert_eq!(
        store
            .list()
            .iter()
            .map(|bookmark| bookmark.id.clone())
            .collect::<Vec<_>>(),
        original_ids
    );
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn invalid_saved_urls_and_duplicate_ids_are_rejected() {
    let directory = std::env::temp_dir().join(format!("SafeBrowse_Test_BM_{}", Uuid::new_v4()));
    let path = directory.join("bookmarks.json");
    let store = BookmarkStore::with_storage_path(path.clone()).unwrap();
    let mut records = store.list().to_vec();
    records[0].url = "javascript:alert(1)".to_string();
    std::fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
    assert!(BookmarkStore::with_storage_path(path.clone()).is_err());

    records[0].url = "https://example.com".to_string();
    records[1].id = records[0].id.clone();
    std::fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
    assert!(BookmarkStore::with_storage_path(path).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn test_bookmark_url_validation() {
    let test_dir = std::env::temp_dir().join(format!("SafeBrowse_Test_BM_{}", Uuid::new_v4()));
    let test_path = test_dir.join("bookmarks.json");

    let mut store =
        BookmarkStore::with_storage_path(test_path).expect("Failed to initialize bookmark store");

    // HTTPS URL should succeed
    let add_ok = store.add(
        "Test Secure Bank",
        "https://bank.example.com",
        BookmarkCategory::Banking,
    );
    assert!(add_ok.is_ok());
    let added = add_ok.unwrap();

    // Dangerous JavaScript scheme must be rejected
    let add_bad = store.add(
        "Malicious Script",
        "javascript:alert(1)",
        BookmarkCategory::General,
    );
    assert!(add_bad.is_err(), "JavaScript URLs must be rejected");

    // Clean up added bookmark
    let remove_res = store.remove(&added.id);
    assert!(remove_res.is_ok());
    assert!(remove_res.unwrap(), "Bookmark should have been removed");

    let _ = std::fs::remove_dir_all(&test_dir);
}
