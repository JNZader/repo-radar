use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::infra::error::PipelineError;

/// Tracks which repo URLs have already been processed, persisted as JSON.
#[derive(Debug)]
pub struct SeenStore {
    seen: HashSet<String>,
    path: PathBuf,
}

impl SeenStore {
    /// Load from disk. Returns an empty store if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns `PipelineError::SeenStore` if the file exists but cannot be read or parsed.
    pub fn load(path: &Path) -> Result<Self, PipelineError> {
        let seen = if path.exists() {
            let content = std::fs::read_to_string(path)
                .map_err(|e| PipelineError::SeenStore(format!("reading {}: {e}", path.display())))?;
            serde_json::from_str::<Vec<String>>(&content)
                .map_err(|e| PipelineError::SeenStore(format!("parsing {}: {e}", path.display())))?
                .into_iter()
                .collect()
        } else {
            HashSet::new()
        };

        Ok(Self {
            seen,
            path: path.to_path_buf(),
        })
    }

    /// Check if a URL has already been processed.
    #[must_use]
    pub fn is_seen(&self, url: &str) -> bool {
        self.seen.contains(url)
    }

    /// Mark a URL as processed.
    pub fn mark_seen(&mut self, url: &str) {
        self.seen.insert(url.to_string());
    }

    /// Persist to disk using atomic write (temp file + rename).
    ///
    /// # Errors
    ///
    /// Returns `PipelineError::SeenStore` if writing or renaming fails.
    pub fn save(&self) -> Result<(), PipelineError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| PipelineError::SeenStore(format!("creating dir {}: {e}", parent.display())))?;
        }

        let urls: Vec<&String> = self.seen.iter().collect();
        let json = serde_json::to_string_pretty(&urls)
            .map_err(|e| PipelineError::SeenStore(format!("serializing: {e}")))?;

        // Atomic write: write to temp file then rename
        let tmp_path = self.path.with_extension("tmp");
        std::fs::write(&tmp_path, &json)
            .map_err(|e| PipelineError::SeenStore(format!("writing {}: {e}", tmp_path.display())))?;
        std::fs::rename(&tmp_path, &self.path)
            .map_err(|e| PipelineError::SeenStore(format!("renaming to {}: {e}", self.path.display())))?;

        Ok(())
    }

    /// Number of seen URLs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_nonexistent_file_creates_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        let store = SeenStore::load(&path).unwrap();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn mark_seen_then_is_seen_returns_true() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seen.json");
        let mut store = SeenStore::load(&path).unwrap();

        store.mark_seen("https://github.com/owner/repo");
        assert!(store.is_seen("https://github.com/owner/repo"));
    }

    #[test]
    fn is_seen_unknown_url_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seen.json");
        let store = SeenStore::load(&path).unwrap();

        assert!(!store.is_seen("https://github.com/unknown/repo"));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seen.json");

        // Create, mark, save
        let mut store = SeenStore::load(&path).unwrap();
        store.mark_seen("https://github.com/a/one");
        store.mark_seen("https://github.com/b/two");
        store.save().unwrap();

        // Load fresh and verify
        let loaded = SeenStore::load(&path).unwrap();
        assert!(loaded.is_seen("https://github.com/a/one"));
        assert!(loaded.is_seen("https://github.com/b/two"));
        assert!(!loaded.is_seen("https://github.com/c/three"));
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn len_accuracy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seen.json");
        let mut store = SeenStore::load(&path).unwrap();

        assert_eq!(store.len(), 0);
        store.mark_seen("https://github.com/a/one");
        assert_eq!(store.len(), 1);
        store.mark_seen("https://github.com/b/two");
        assert_eq!(store.len(), 2);
        // Duplicate should not increase count
        store.mark_seen("https://github.com/a/one");
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn atomic_write_produces_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seen.json");

        let mut store = SeenStore::load(&path).unwrap();
        store.mark_seen("https://github.com/x/y");
        store.save().unwrap();

        // Verify the file contains valid JSON
        let content = std::fs::read_to_string(&path).unwrap();
        let urls: Vec<String> = serde_json::from_str(&content).unwrap();
        assert_eq!(urls.len(), 1);
        assert!(urls.contains(&"https://github.com/x/y".to_string()));

        // Verify no temp file left behind
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("seen.json");

        let mut store = SeenStore::load(&path).unwrap();
        store.mark_seen("https://github.com/a/b");
        store.save().unwrap();

        assert!(path.exists());
    }
}
