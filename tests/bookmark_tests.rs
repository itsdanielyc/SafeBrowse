//! Unit and Integration Tests for Persistent Bookmarks Store

use safebrowse::bookmarks::{BookmarkCategory, BookmarkStore};
use uuid::Uuid;

#[test]
fn test_bookmark_store_initialization_and_defaults() {
    let test_dir = std::env::temp_dir().join(format!("SafeBrowse_Test_BM_{}", Uuid::new_v4()));
    let test_path = test_dir.join("bookmarks.json");

    let store = BookmarkStore::with_storage_path(test_path).expect("Failed to initialize bookmark store");
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
fn test_bookmark_url_validation() {
    let test_dir = std::env::temp_dir().join(format!("SafeBrowse_Test_BM_{}", Uuid::new_v4()));
    let test_path = test_dir.join("bookmarks.json");

    let mut store = BookmarkStore::with_storage_path(test_path).expect("Failed to initialize bookmark store");

    // HTTPS URL should succeed
    let add_ok = store.add("Test Secure Bank", "https://bank.example.com", BookmarkCategory::Banking);
    assert!(add_ok.is_ok());
    let added = add_ok.unwrap();

    // Dangerous JavaScript scheme must be rejected
    let add_bad = store.add("Malicious Script", "javascript:alert(1)", BookmarkCategory::General);
    assert!(add_bad.is_err(), "JavaScript URLs must be rejected");

    // Clean up added bookmark
    let remove_res = store.remove(&added.id);
    assert!(remove_res.is_ok());
    assert!(remove_res.unwrap(), "Bookmark should have been removed");

    let _ = std::fs::remove_dir_all(&test_dir);
}
