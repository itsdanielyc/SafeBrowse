//! Unit and Integration Tests for Tab Management

use safebrowse::browser::tabs::TabKind;
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

#[test]
fn rejected_close_preserves_the_last_tab_and_its_loading_state() {
    let mut manager = TabManager::new("https://bank.example/account");
    manager.update_title(1, "Account");
    manager.set_loading(1, true);
    assert!(!manager.close_tab(999));
    assert!(!manager.close_tab(1));
    let active = manager.active_tab().unwrap();
    assert_eq!(active.url, "https://bank.example/account");
    assert_eq!(active.title, "Account");
    assert!(active.is_loading);
}

#[test]
fn background_navigation_updates_only_its_own_tab() {
    let mut manager = TabManager::new("https://first.example");
    let second = manager.open_tab("https://second.example");
    manager.set_loading(1, true);
    manager.update_url(1, "http://first.example/redirect");
    manager.update_title(1, "Redirected");
    assert_eq!(manager.active_id(), second);
    assert_eq!(manager.active_tab().unwrap().url, "https://second.example");
    let first = manager.tab(1).unwrap();
    assert!(first.is_loading);
    assert!(!first.is_secure);
    assert_eq!(first.title, "Redirected");
    assert!(!manager.update_url(999, "https://missing.example"));
}

#[test]
fn closing_active_tab_selects_an_adjacent_tab_and_special_tabs_are_unique() {
    let mut manager = TabManager::new("https://first.example");
    let middle = manager.open_tab("https://middle.example");
    let last = manager.open_tab("https://last.example");
    manager.switch_to_tab(middle);
    assert!(manager.close_tab(middle));
    assert_eq!(manager.active_id(), last);
    assert!(manager.close_tab(last));
    assert_eq!(manager.active_id(), 1);
    let bookmarks = manager.open_or_switch_special("Bookmarks", TabKind::Bookmarks);
    manager.switch_to_tab(1);
    assert_eq!(
        manager.open_or_switch_special("Bookmarks", TabKind::Bookmarks),
        bookmarks
    );
    assert_eq!(manager.list().len(), 2);
    assert!(!manager.update_url(bookmarks, "https://example.com"));
    assert!(!manager.active_tab().unwrap().is_secure);
}
