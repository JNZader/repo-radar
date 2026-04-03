use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::infra::error::PipelineError;

/// Cached GitHub repo metadata with a timestamp for TTL checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedRepoMetadata {
    pub owner: String,
    pub repo_name: String,
    pub stars: u64,
    pub language: Option<String>,
    pub topics: Vec<String>,
    pub fork: bool,
    pub archived: bool,
    pub cached_at: DateTime<Utc>,
}

/// JSON-file-backed cache for GitHub repo metadata with configurable TTL.
#[derive(Debug)]
pub struct RepoCache {
    entries: HashMap<String, CachedRepoMetadata>,
    path: PathBuf,
    ttl: Duration,
}

impl RepoCache {
    /// Load cache from disk. Returns an empty cache if the file does not exist.
    /// Logs a warning and returns empty cache if the file is corrupt.
    ///
    /// # Errors
    ///
    /// Returns `PipelineError::Cache` if the file exists but cannot be read
    /// (permission denied, etc). Corrupt JSON is treated as empty cache.
    pub fn load(path: &Path, ttl: Duration) -> Result<Self, PipelineError> {
        let entries = if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    match serde_json::from_str::<HashMap<String, CachedRepoMetadata>>(&content) {
                        Ok(map) => map,
                        Err(e) => {
                            warn!(
                                path = %path.display(),
                                error = %e,
                                "corrupt cache file, starting with empty cache"
                            );
                            HashMap::new()
                        }
                    }
                }
                Err(e) => {
                    return Err(PipelineError::Cache(format!(
                        "reading {}: {e}",
                        path.display()
                    )));
                }
            }
        } else {
            HashMap::new()
        };

        Ok(Self {
            entries,
            path: path.to_path_buf(),
            ttl,
        })
    }

    /// Get a cached entry by key ("owner/repo").
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&CachedRepoMetadata> {
        self.entries.get(key)
    }

    /// Check if a cached entry exists and is within TTL.
    #[must_use]
    pub fn is_fresh(&self, key: &str) -> bool {
        self.entries.get(key).is_some_and(|entry| {
            let age = Utc::now()
                .signed_duration_since(entry.cached_at)
                .to_std()
                .unwrap_or(Duration::MAX);
            age < self.ttl
        })
    }

    /// Insert or update a cache entry.
    pub fn insert(&mut self, key: String, meta: CachedRepoMetadata) {
        self.entries.insert(key, meta);
    }

    /// Persist cache to disk using atomic write (temp file + rename).
    ///
    /// # Errors
    ///
    /// Returns `PipelineError::Cache` if writing or renaming fails.
    pub fn save(&self) -> Result<(), PipelineError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                PipelineError::Cache(format!("creating dir {}: {e}", parent.display()))
            })?;
        }

        let json = serde_json::to_string_pretty(&self.entries)
            .map_err(|e| PipelineError::Cache(format!("serializing cache: {e}")))?;

        let tmp_path = self.path.with_extension("tmp");
        std::fs::write(&tmp_path, &json)
            .map_err(|e| PipelineError::Cache(format!("writing {}: {e}", tmp_path.display())))?;
        std::fs::rename(&tmp_path, &self.path).map_err(|e| {
            PipelineError::Cache(format!("renaming to {}: {e}", self.path.display()))
        })?;

        Ok(())
    }

    /// Number of cached entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metadata(owner: &str, repo: &str) -> CachedRepoMetadata {
        CachedRepoMetadata {
            owner: owner.to_string(),
            repo_name: repo.to_string(),
            stars: 100,
            language: Some("Rust".to_string()),
            topics: vec!["cli".to_string()],
            fork: false,
            archived: false,
            cached_at: Utc::now(),
        }
    }

    #[test]
    fn load_nonexistent_file_creates_empty_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let cache = RepoCache::load(&path, Duration::from_secs(3600)).unwrap();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn insert_and_get() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = RepoCache::load(&path, Duration::from_secs(3600)).unwrap();

        cache.insert("owner/repo".to_string(), sample_metadata("owner", "repo"));
        assert_eq!(cache.len(), 1);

        let entry = cache.get("owner/repo").unwrap();
        assert_eq!(entry.stars, 100);
        assert_eq!(entry.language.as_deref(), Some("Rust"));
    }

    #[test]
    fn is_fresh_within_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = RepoCache::load(&path, Duration::from_secs(3600)).unwrap();

        cache.insert("owner/repo".to_string(), sample_metadata("owner", "repo"));
        assert!(cache.is_fresh("owner/repo"));
    }

    #[test]
    fn is_fresh_expired_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = RepoCache::load(&path, Duration::from_secs(0)).unwrap();

        let mut meta = sample_metadata("owner", "repo");
        meta.cached_at = Utc::now() - chrono::Duration::hours(1);
        cache.insert("owner/repo".to_string(), meta);

        assert!(!cache.is_fresh("owner/repo"));
    }

    #[test]
    fn is_fresh_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let cache = RepoCache::load(&path, Duration::from_secs(3600)).unwrap();
        assert!(!cache.is_fresh("nonexistent/repo"));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");

        let mut cache = RepoCache::load(&path, Duration::from_secs(3600)).unwrap();
        cache.insert("a/one".to_string(), sample_metadata("a", "one"));
        cache.insert("b/two".to_string(), sample_metadata("b", "two"));
        cache.save().unwrap();

        let loaded = RepoCache::load(&path, Duration::from_secs(3600)).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.get("a/one").is_some());
        assert!(loaded.get("b/two").is_some());
    }

    #[test]
    fn corrupt_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        std::fs::write(&path, "not valid json {{{{").unwrap();

        let cache = RepoCache::load(&path, Duration::from_secs(3600)).unwrap();
        assert!(cache.is_empty());
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("cache.json");

        let mut cache = RepoCache::load(&path, Duration::from_secs(3600)).unwrap();
        cache.insert("owner/repo".to_string(), sample_metadata("owner", "repo"));
        cache.save().unwrap();

        assert!(path.exists());
    }

    #[test]
    fn atomic_write_no_temp_file_left() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");

        let mut cache = RepoCache::load(&path, Duration::from_secs(3600)).unwrap();
        cache.insert("x/y".to_string(), sample_metadata("x", "y"));
        cache.save().unwrap();

        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn insert_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = RepoCache::load(&path, Duration::from_secs(3600)).unwrap();

        cache.insert("owner/repo".to_string(), sample_metadata("owner", "repo"));
        assert_eq!(cache.get("owner/repo").unwrap().stars, 100);

        let mut updated = sample_metadata("owner", "repo");
        updated.stars = 999;
        cache.insert("owner/repo".to_string(), updated);
        assert_eq!(cache.get("owner/repo").unwrap().stars, 999);
        assert_eq!(cache.len(), 1);
    }
}
