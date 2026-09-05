//! Multi-Tab Management & Navigation State
//!
//! Tracks open tabs, navigation history states, loading indicators, and active view selection.

use serde::{Deserialize, Serialize};

use super::navigation::uses_https;

/// Type classification for a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabKind {
    Web,
    Bookmarks,
    Settings,
}

/// Represents an individual browser tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabItem {
    pub id: usize,
    pub title: String,
    pub url: String,
    pub is_loading: bool,
    pub is_secure: bool,
    pub kind: TabKind,
}

impl TabItem {
    /// Constructs a new tab with a given ID and initial URL.
    pub fn new(id: usize, url: impl Into<String>) -> Self {
        let url_str = url.into();
        let is_secure = uses_https(&url_str);
        Self {
            id,
            title: "New Tab".to_string(),
            url: url_str,
            is_loading: false,
            is_secure,
            kind: TabKind::Web,
        }
    }

    /// Constructs a special system tab (e.g. Bookmarks or Settings).
    pub fn new_special(id: usize, title: impl Into<String>, kind: TabKind) -> Self {
        let title_str = title.into();
        let url_scheme = match kind {
            TabKind::Bookmarks => "safebrowse://bookmarks",
            TabKind::Settings => "safebrowse://settings",
            TabKind::Web => "about:blank",
        };
        Self {
            id,
            title: title_str,
            url: url_scheme.to_string(),
            is_loading: false,
            is_secure: false,
            kind,
        }
    }
}

/// Coordinates tab lifecycle and active tab selection.
pub struct TabManager {
    tabs: Vec<TabItem>,
    next_id: usize,
    active_tab_id: usize,
}

impl TabManager {
    /// Initializes the tab manager with an initial blank or default homepage tab.
    pub fn new(initial_url: impl Into<String>) -> Self {
        let initial_tab = TabItem::new(1, initial_url);
        Self {
            tabs: vec![initial_tab],
            next_id: 2,
            active_tab_id: 1,
        }
    }

    /// Returns a slice of all currently open tabs.
    #[inline]
    pub fn list(&self) -> &[TabItem] {
        &self.tabs
    }

    /// Returns the active tab ID.
    #[inline]
    pub fn active_id(&self) -> usize {
        self.active_tab_id
    }

    /// Returns a reference to the active tab.
    pub fn active_tab(&self) -> Option<&TabItem> {
        self.tab(self.active_tab_id)
    }

    /// Returns a tab by its stable identifier.
    pub fn tab(&self, id: usize) -> Option<&TabItem> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    /// Creates and activates a new web tab with the target URL.
    ///
    /// # Complexity
    /// - Time: O(1)
    /// - Space: O(1)
    pub fn open_tab(&mut self, url: impl Into<String>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let tab = TabItem::new(id, url);
        self.tabs.push(tab);
        self.active_tab_id = id;
        id
    }

    /// Opens or switches to a special tab (Bookmarks or Settings).
    ///
    /// # Complexity
    /// - Time: O(N)
    /// - Space: O(1)
    pub fn open_or_switch_special(&mut self, title: &str, kind: TabKind) -> usize {
        if let Some(existing) = self.tabs.iter().find(|t| t.kind == kind) {
            let id = existing.id;
            self.active_tab_id = id;
            return id;
        }

        let id = self.next_id;
        self.next_id += 1;
        let tab = TabItem::new_special(id, title, kind);
        self.tabs.push(tab);
        self.active_tab_id = id;
        id
    }

    /// Switches the active tab to the specified ID.
    pub fn switch_to_tab(&mut self, id: usize) -> bool {
        if self.tabs.iter().any(|t| t.id == id) {
            self.active_tab_id = id;
            true
        } else {
            false
        }
    }

    /// Closes a tab by ID. If closing the active tab, activates an adjacent tab.
    ///
    /// # Complexity
    /// - Time: O(N)
    /// - Space: O(1)
    pub fn close_tab(&mut self, id: usize) -> bool {
        // The caller owns last-tab navigation; a rejected close must not silently
        // change metadata while the existing webview still shows its old page.
        if self.tabs.len() <= 1 {
            return false;
        }

        let current_index = self.tabs.iter().position(|t| t.id == id);
        if let Some(idx) = current_index {
            self.tabs.remove(idx);

            if self.active_tab_id == id {
                let new_idx = if idx >= self.tabs.len() {
                    self.tabs.len() - 1
                } else {
                    idx
                };
                self.active_tab_id = self.tabs[new_idx].id;
            }
            true
        } else {
            false
        }
    }

    /// Updates the URL, title, and security state for a tab, setting its kind to Web.
    pub fn update_tab(&mut self, id: usize, url: String, title: String) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.is_secure = uses_https(&url);
            tab.url = url;
            tab.title = title;
            tab.is_loading = false;
            tab.kind = TabKind::Web;
        }
    }

    /// Updates the loading indicator for one webview without changing its title.
    pub fn set_loading(&mut self, id: usize, is_loading: bool) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return false;
        };
        tab.is_loading = is_loading && tab.kind == TabKind::Web;
        true
    }

    /// Updates the current address and transport indicator after navigation.
    pub fn update_url(&mut self, id: usize, url: &str) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return false;
        };
        if tab.kind != TabKind::Web {
            return false;
        }
        tab.url = url.to_owned();
        tab.is_secure = uses_https(url);
        true
    }

    /// Updates a tab title independently of loading and address state.
    pub fn update_title(&mut self, id: usize, title: &str) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return false;
        };
        if !title.trim().is_empty() {
            tab.title = title.trim().to_owned();
        }
        true
    }
}
