//! Persistent Bookmark Store Subsystem
//!
//! Provides durable, renderer-isolated storage for trusted banking and payment URLs.
//! Uses atomic file replacement with unique temporary staging paths to ensure
//! corruption-free persistence even under high concurrency or unexpected power loss.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use url::Url;
use uuid::Uuid;

use crate::config::{APP_IDENTIFIER, BOOKMARKS_FILE_NAME};

/// Category classification for bookmarks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BookmarkCategory {
    Banking,
    Payment,
    Utility,
    General,
}

/// Represents an individual trusted bookmark entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: String,
    pub title: String,
    pub url: String,
    pub category: BookmarkCategory,
    pub created_at: DateTime<Utc>,
}

impl Bookmark {
    /// Constructs a new bookmark with a generated UUID and current timestamp.
    pub fn new(title: impl Into<String>, url: impl Into<String>, category: BookmarkCategory) -> Result<Self, String> {
        let url_str = url.into();
        // Why: Validate URL format strictly to prevent malformed or script injection URLs.
        let parsed = Url::parse(&url_str).map_err(|e| format!("Invalid bookmark URL: {}", e))?;
        if parsed.scheme() != "https" && parsed.scheme() != "http" {
            return Err("Only HTTP/HTTPS URLs are allowed for bookmarks".to_string());
        }

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            url: url_str,
            category,
            created_at: Utc::now(),
        })
    }
}

/// Thread-safe and crash-resilient manager for user bookmarks.
pub struct BookmarkStore {
    storage_path: PathBuf,
    bookmarks: Vec<Bookmark>,
}

impl BookmarkStore {
    /// Initializes the bookmark store with default app data configuration.
    pub fn initialize() -> Result<Self, String> {
        let app_dirs = directories::ProjectDirs::from("com", "SafeBrowse", APP_IDENTIFIER)
            .ok_or_else(|| "Failed to resolve local application data directory".to_string())?;

        let config_dir = app_dirs.config_dir();
        fs::create_dir_all(config_dir)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;

        let storage_path = config_dir.join(BOOKMARKS_FILE_NAME);
        Self::with_storage_path(storage_path)
    }

    /// Initializes a bookmark store with a custom target storage path (Dependency Injection).
    ///
    /// # Complexity
    /// - Time: O(N) where N is number of bookmarks on disk
    /// - Space: O(N)
    pub fn with_storage_path(storage_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = storage_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent directory for bookmarks: {}", e))?;
        }

        let mut store = Self {
            storage_path,
            bookmarks: Vec::new(),
        };

        if store.storage_path.exists() {
            store.load_from_disk()?;
        } else {
            store.populate_default_banking_bookmarks()?;
            store.persist_to_disk()?;
        }

        Ok(store)
    }

    /// Loads bookmarks from the JSON file on disk.
    fn load_from_disk(&mut self) -> Result<(), String> {
        let content = fs::read_to_string(&self.storage_path)
            .map_err(|e| format!("Failed to read bookmarks file: {}", e))?;

        let parsed: Vec<Bookmark> = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse bookmarks JSON: {}", e))?;

        self.bookmarks = parsed;
        Ok(())
    }

    /// Atomically persists the bookmark collection to disk using a collision-free temporary file.
    ///
    /// # Complexity
    /// - Time: O(N)
    /// - Space: O(N)
    pub fn persist_to_disk(&self) -> Result<(), String> {
        let serialized = serde_json::to_string_pretty(&self.bookmarks)
            .map_err(|e| format!("Failed to serialize bookmarks: {}", e))?;

        // Why: Generate a unique staging temporary file to prevent race conditions during concurrent execution.
        let temp_filename = format!("{}.tmp.{}", BOOKMARKS_FILE_NAME, Uuid::new_v4());
        let temp_path = match self.storage_path.parent() {
            Some(parent) => parent.join(temp_filename),
            None => PathBuf::from(temp_filename),
        };

        {
            let mut file = File::create(&temp_path)
                .map_err(|e| format!("Failed to create temporary bookmark file: {}", e))?;
            file.write_all(serialized.as_bytes())
                .map_err(|e| format!("Failed to write bookmark payload: {}", e))?;
            file.sync_all()
                .map_err(|e| format!("Failed to sync bookmark file to storage: {}", e))?;
        }

        fs::rename(&temp_path, &self.storage_path)
            .map_err(|e| format!("Failed to atomically replace bookmark file: {}", e))?;

        Ok(())
    }

    /// Populates curated default financial and secure search bookmarks.
    fn populate_default_banking_bookmarks(&mut self) -> Result<(), String> {
        self.bookmarks.clear();
        self.bookmarks.push(Bookmark::new(
            "DuckDuckGo (Private Search)",
            "https://duckduckgo.com",
            BookmarkCategory::Utility,
        )?);
        self.bookmarks.push(Bookmark::new(
            "PayPal",
            "https://www.paypal.com",
            BookmarkCategory::Payment,
        )?);
        self.bookmarks.push(Bookmark::new(
            "Stripe Dashboard",
            "https://dashboard.stripe.com",
            BookmarkCategory::Payment,
        )?);
        self.bookmarks.push(Bookmark::new(
            "Chase Online",
            "https://www.chase.com",
            BookmarkCategory::Banking,
        )?);
        self.bookmarks.push(Bookmark::new(
            "Bank of America",
            "https://www.bankofamerica.com",
            BookmarkCategory::Banking,
        )?);
        self.bookmarks.push(Bookmark::new(
            "Fidelity Investments",
            "https://www.fidelity.com",
            BookmarkCategory::Banking,
        )?);
        Ok(())
    }

    /// Returns a slice of all stored bookmarks.
    #[inline]
    pub fn list(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    /// Adds a new bookmark and immediately writes changes to disk.
    pub fn add(&mut self, title: impl Into<String>, url: impl Into<String>, category: BookmarkCategory) -> Result<Bookmark, String> {
        let bookmark = Bookmark::new(title, url, category)?;
        self.bookmarks.push(bookmark.clone());
        self.persist_to_disk()?;
        Ok(bookmark)
    }

    /// Deletes a bookmark by its unique ID.
    pub fn remove(&mut self, id: &str) -> Result<bool, String> {
        let initial_len = self.bookmarks.len();
        self.bookmarks.retain(|b| b.id != id);
        let removed = self.bookmarks.len() < initial_len;
        if removed {
            self.persist_to_disk()?;
        }
        Ok(removed)
    }
}
