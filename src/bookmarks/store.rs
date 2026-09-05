//! Persistent Bookmark Store Subsystem
//!
//! Persists user bookmarks outside web content using atomic file replacement.
//! Callers must serialize writes through one store instance. Saved addresses are
//! user data; their presence does not establish that a website is trustworthy.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use uuid::Uuid;

use crate::browser::navigation::validate_web_url;
use crate::config::{APP_IDENTIFIER, BOOKMARKS_FILE_NAME};

/// Category classification for bookmarks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BookmarkCategory {
    Banking,
    Payment,
    Utility,
    General,
}

/// Represents an individual user bookmark entry.
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
    pub fn new(
        title: impl Into<String>,
        url: impl Into<String>,
        category: BookmarkCategory,
    ) -> Result<Self, String> {
        let url_str = validate_web_url(&url.into())?;
        let title = normalize_title(&title.into())?;

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            title,
            url: url_str,
            category,
            created_at: Utc::now(),
        })
    }
}

/// Owns one bookmark collection; callers provide synchronization when shared.
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
        if let Some(parent) = storage_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
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

        let mut parsed: Vec<Bookmark> = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse bookmarks JSON: {}", e))?;

        let mut identifiers = HashSet::with_capacity(parsed.len());
        for bookmark in &mut parsed {
            if bookmark.id.trim().is_empty() || !identifiers.insert(bookmark.id.clone()) {
                return Err("Bookmarks contain an empty or duplicate identifier".to_string());
            }
            bookmark.url = validate_web_url(&bookmark.url)
                .map_err(|error| format!("Invalid saved bookmark: {error}"))?;
            bookmark.title = normalize_title(&bookmark.title)?;
        }

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

        // Stage in the destination directory so rename stays on the same filesystem.
        let temp_filename = format!("{}.tmp.{}", BOOKMARKS_FILE_NAME, Uuid::new_v4());
        let temp_path = match self.storage_path.parent() {
            Some(parent) => parent.join(temp_filename),
            None => PathBuf::from(temp_filename),
        };

        let persist_result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|e| format!("Failed to create temporary bookmark file: {}", e))?;
            file.write_all(serialized.as_bytes())
                .map_err(|e| format!("Failed to write bookmark payload: {}", e))?;
            file.sync_all()
                .map_err(|e| format!("Failed to sync bookmark file to storage: {}", e))?;
            drop(file);
            fs::rename(&temp_path, &self.storage_path)
                .map_err(|e| format!("Failed to atomically replace bookmark file: {}", e))
        })();

        if persist_result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        persist_result
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
    pub fn add(
        &mut self,
        title: impl Into<String>,
        url: impl Into<String>,
        category: BookmarkCategory,
    ) -> Result<Bookmark, String> {
        let bookmark = Bookmark::new(title, url, category)?;
        self.bookmarks.push(bookmark.clone());
        if let Err(error) = self.persist_to_disk() {
            self.bookmarks.pop();
            return Err(error);
        }
        Ok(bookmark)
    }

    /// Deletes a bookmark by its unique ID.
    pub fn remove(&mut self, id: &str) -> Result<bool, String> {
        let Some(index) = self.bookmarks.iter().position(|bookmark| bookmark.id == id) else {
            return Ok(false);
        };
        let bookmark = self.bookmarks.remove(index);
        if let Err(error) = self.persist_to_disk() {
            self.bookmarks.insert(index, bookmark);
            return Err(error);
        }
        Ok(true)
    }
}

fn normalize_title(title: &str) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("A bookmark title is required".to_string());
    }
    Ok(title.to_owned())
}
