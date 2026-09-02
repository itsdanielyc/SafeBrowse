//! Multi-Tab Management & Navigation State
//!
//! Tracks open tabs, navigation history states, loading indicators, and active view selection.

use serde::{Deserialize, Serialize};

/// Represents an individual browser tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabItem {
    pub id: usize,
    pub title: String,
    pub url: String,
    pub is_loading: bool,
    pub is_secure: bool,
}

impl TabItem {
    /// Constructs a new tab with a given ID and initial URL.
    pub fn new(id: usize, url: impl Into<String>) -> Self {
        let url_str = url.into();
        let is_secure = url_str.starts_with("https://");
        Self {
            id,
            title: "New Tab".to_string(),
            url: url_str,
            is_loading: false,
            is_secure,
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
        self.tabs.iter().find(|t| t.id == self.active_tab_id)
    }

    /// Creates and activates a new tab with the target URL.
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
        // Why: Preserve at least one tab open at all times.
        if self.tabs.len() <= 1 {
            if let Some(first) = self.tabs.first_mut() {
                first.url = "https://duckduckgo.com".to_string();
                first.title = "New Tab".to_string();
                first.is_secure = true;
            }
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

    /// Updates the URL, title, and security state for a tab.
    pub fn update_tab(&mut self, id: usize, url: String, title: String) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.is_secure = url.starts_with("https://");
            tab.url = url;
            tab.title = title;
            tab.is_loading = false;
        }
    }
}
