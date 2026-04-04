use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::domain::model::CrossRefResult;
use crate::infra::error::PipelineError;

/// Metadata about a stored scan (without the full results payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanMeta {
    pub id: String,
    pub scanned_at: DateTime<Utc>,
    pub result_count: usize,
}

/// Persisted scan payload written to disk.
#[derive(Debug, Serialize, Deserialize)]
struct ScanFile {
    pub meta: ScanMeta,
    pub results: Vec<CrossRefResult>,
}

/// JSON-file-backed store for scan results.
///
/// Layout:
/// ```text
/// {results_dir}/
///   {timestamp}.json   # ScanFile
///   {timestamp}.json
/// ```
#[derive(Debug, Clone)]
pub struct ScanResultStore {
    dir: PathBuf,
}

impl ScanResultStore {
    /// Create a new store backed by the given directory.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Save scan results to a new timestamped file.
    ///
    /// # Errors
    ///
    /// Returns `PipelineError::Cache` if the directory cannot be created or the
    /// file cannot be written.
    pub fn save(&self, results: &[CrossRefResult]) -> Result<ScanMeta, PipelineError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| {
            PipelineError::Cache(format!("creating results dir {}: {e}", self.dir.display()))
        })?;

        let now = Utc::now();
        let id = now.format("%Y%m%d-%H%M%S-%3f").to_string();
        let meta = ScanMeta {
            id: id.clone(),
            scanned_at: now,
            result_count: results.len(),
        };

        let file = ScanFile {
            meta: meta.clone(),
            results: results.to_vec(),
        };

        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| PipelineError::Cache(format!("serializing scan results: {e}")))?;

        let path = self.dir.join(format!("{id}.json"));
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, &json).map_err(|e| {
            PipelineError::Cache(format!("writing {}: {e}", tmp_path.display()))
        })?;
        std::fs::rename(&tmp_path, &path).map_err(|e| {
            PipelineError::Cache(format!("renaming to {}: {e}", path.display()))
        })?;

        info!(id = %id, count = results.len(), "scan results saved");
        Ok(meta)
    }

    /// List available scans, sorted newest-first.
    ///
    /// # Errors
    ///
    /// Returns `PipelineError::Cache` if the directory cannot be read.
    pub fn list(&self) -> Result<Vec<ScanMeta>, PipelineError> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let entries = std::fs::read_dir(&self.dir).map_err(|e| {
            PipelineError::Cache(format!("reading results dir {}: {e}", self.dir.display()))
        })?;

        let mut metas: Vec<ScanMeta> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "json")
            })
            .filter_map(|e| {
                let content = std::fs::read_to_string(e.path()).ok()?;
                let file: ScanFile = match serde_json::from_str(&content) {
                    Ok(f) => f,
                    Err(err) => {
                        warn!(path = %e.path().display(), %err, "skipping corrupt scan file");
                        return None;
                    }
                };
                Some(file.meta)
            })
            .collect();

        // Newest first
        metas.sort_by(|a, b| b.scanned_at.cmp(&a.scanned_at));
        Ok(metas)
    }

    /// Load full results for a specific scan by ID.
    ///
    /// # Errors
    ///
    /// Returns `PipelineError::Cache` if the file cannot be found or parsed.
    pub fn load(&self, id: &str) -> Result<Vec<CrossRefResult>, PipelineError> {
        let path = self.dir.join(format!("{id}.json"));
        if !path.exists() {
            return Err(PipelineError::Cache(format!(
                "scan result not found: {}",
                path.display()
            )));
        }

        let content = std::fs::read_to_string(&path).map_err(|e| {
            PipelineError::Cache(format!("reading {}: {e}", path.display()))
        })?;

        let file: ScanFile = serde_json::from_str(&content).map_err(|e| {
            PipelineError::Cache(format!("parsing {}: {e}", path.display()))
        })?;

        Ok(file.results)
    }

    /// Load the most recent scan results, if any exist.
    ///
    /// # Errors
    ///
    /// Returns `PipelineError::Cache` on I/O or parse errors.
    pub fn load_latest(&self) -> Result<Option<Vec<CrossRefResult>>, PipelineError> {
        let scans = self.list()?;
        match scans.first() {
            Some(meta) => Ok(Some(self.load(&meta.id)?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{AnalysisResult, FeedEntry, RepoCandidate, RepoMatch};
    use url::Url;

    fn sample_result(name: &str) -> CrossRefResult {
        CrossRefResult {
            analysis: AnalysisResult {
                candidate: RepoCandidate {
                    entry: FeedEntry {
                        title: name.into(),
                        repo_url: Url::parse(&format!("https://github.com/owner/{name}")).unwrap(),
                        description: Some("desc".into()),
                        published: Some(Utc::now()),
                        source_name: "test".into(),
                    },
                    stars: 100,
                    language: Some("Rust".into()),
                    topics: vec!["cli".into()],
                    fork: false,
                    archived: false,
                    owner: "owner".into(),
                    repo_name: name.into(),
                    category: Default::default(),
                },
                summary: "summary".into(),
                key_features: vec!["fast".into()],
                tech_stack: vec!["Rust".into()],
                relevance_score: 0.8,
            },
            matched_repos: vec![RepoMatch {
                own_repo: "my-project".into(),
                relevance: 0.7,
                reason: "similar".into(),
            }],
            ideas: vec!["idea".into()],
            overall_relevance: 0.75,
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScanResultStore::new(dir.path().join("results"));

        let results = vec![sample_result("tool-a"), sample_result("tool-b")];
        let meta = store.save(&results).unwrap();

        assert_eq!(meta.result_count, 2);
        assert!(!meta.id.is_empty());

        let loaded = store.load(&meta.id).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].analysis.candidate.entry.title, "tool-a");
        assert_eq!(loaded[1].analysis.candidate.entry.title, "tool-b");
    }

    #[test]
    fn list_returns_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScanResultStore::new(dir.path().join("results"));

        store.save(&[sample_result("first")]).unwrap();
        // Ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(10));
        store.save(&[sample_result("second")]).unwrap();

        let scans = store.list().unwrap();
        assert_eq!(scans.len(), 2);
        assert!(scans[0].scanned_at >= scans[1].scanned_at);
    }

    #[test]
    fn list_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScanResultStore::new(dir.path().join("nonexistent"));
        let scans = store.list().unwrap();
        assert!(scans.is_empty());
    }

    #[test]
    fn load_nonexistent_id_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScanResultStore::new(dir.path().join("results"));
        assert!(store.load("does-not-exist").is_err());
    }

    #[test]
    fn load_latest_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScanResultStore::new(dir.path().join("results"));
        assert!(store.load_latest().unwrap().is_none());
    }

    #[test]
    fn load_latest_returns_most_recent() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScanResultStore::new(dir.path().join("results"));

        store.save(&[sample_result("old")]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        store.save(&[sample_result("new")]).unwrap();

        let latest = store.load_latest().unwrap().unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].analysis.candidate.entry.title, "new");
    }

    #[test]
    fn save_empty_results() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScanResultStore::new(dir.path().join("results"));

        let meta = store.save(&[]).unwrap();
        assert_eq!(meta.result_count, 0);

        let loaded = store.load(&meta.id).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn corrupt_file_skipped_in_list() {
        let dir = tempfile::tempdir().unwrap();
        let results_dir = dir.path().join("results");
        std::fs::create_dir_all(&results_dir).unwrap();

        // Write a corrupt file
        std::fs::write(results_dir.join("corrupt.json"), "not valid json").unwrap();

        let store = ScanResultStore::new(results_dir);
        let scans = store.list().unwrap();
        assert!(scans.is_empty());
    }
}
