//! Unit and Integration Tests for Tab Management

use safebrowse::browser::tabs::TabManager;

#[test]
fn test_tab_manager_lifecycle() {
    let mut tm = TabManager::new("https://duckduckgo.com");
    assert_eq!(tm.list().len(), 1);
    assert_eq!(tm.active_id(), 1);

    // Open a second tab
    let id2 = tm.open_tab("https://www.paypal.com");
    assert_eq!(tm.list().len(), 2);
    assert_eq!(tm.active_id(), id2);

    // Switch back to first tab
    let switched = tm.switch_to_tab(1);
    assert!(switched);
    assert_eq!(tm.active_id(), 1);

    // Close second tab
    let closed = tm.close_tab(id2);
    assert!(closed);
    assert_eq!(tm.list().len(), 1);

    // Closing the only remaining tab must preserve a clamped default tab
    let closed_last = tm.close_tab(1);
    assert!(!closed_last);
    assert_eq!(tm.list().len(), 1);
}
