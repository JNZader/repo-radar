use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::domain::model::{KbAnalysis, KbAnalysisStatus};
use crate::infra::error::KbError;

/// SQLite-backed persistence for the knowledge base.
///
/// The connection is wrapped in `Arc<Mutex<_>>` so the struct can be cheaply
/// cloned across async tasks while keeping access serialized (single writer).
pub struct SqliteKb {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteKb {
    /// Open (or create) the SQLite database at `path`, run schema migrations,
    /// and return a ready-to-use `SqliteKb`.
    pub fn open(path: &std::path::Path) -> Result<Self, KbError> {
        let conn =
            Connection::open(path).map_err(|e| KbError::Sqlite(e.to_string()))?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert or replace a `KbAnalysis` row (keyed by `id`).
    pub fn upsert(&self, analysis: &KbAnalysis) -> Result<(), KbError> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let now = Utc::now().to_rfc3339();

        // Determine first_seen_at: keep existing value if row already exists.
        let existing_first_seen: Option<String> = conn
            .query_row(
                "SELECT first_seen_at FROM repos WHERE id = ?1",
                [&analysis.owner_repo_id()],
                |row| row.get(0),
            )
            .ok();

        let first_seen_at = existing_first_seen.as_deref().unwrap_or(&now);
        let analyzed_at = match analysis.status {
            KbAnalysisStatus::Complete => Some(now.clone()),
            KbAnalysisStatus::ParseFailed => None,
        };

        let techniques_json =
            serde_json::to_string(&analysis.techniques).unwrap_or_else(|_| "[]".into());
        let steal_json =
            serde_json::to_string(&analysis.steal).unwrap_or_else(|_| "[]".into());
        let topics_json =
            serde_json::to_string(&analysis.topics).unwrap_or_else(|_| "[]".into());
        let status_str = match analysis.status {
            KbAnalysisStatus::Complete => "complete",
            KbAnalysisStatus::ParseFailed => "parse_failed",
        };

        conn.execute(
            "INSERT OR REPLACE INTO repos (
                id, owner, repo_name, url, stars, language, topics,
                pushed_at, first_seen_at, last_seen_at, analyzed_at,
                status, what, problem, architecture, techniques, steal,
                uniqueness, raw_llm_response, is_own
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15, ?16, ?17,
                ?18, ?19, ?20
            )",
            rusqlite::params![
                analysis.owner_repo_id(),
                analysis.owner,
                analysis.repo_name,
                analysis.url,
                analysis.stars as i64,
                analysis.language,
                topics_json,
                analysis.pushed_at.as_ref().map(|dt| dt.to_rfc3339()),
                first_seen_at,
                now,
                analyzed_at,
                status_str,
                analysis.what,
                analysis.problem,
                analysis.architecture,
                techniques_json,
                steal_json,
                analysis.uniqueness,
                analysis.raw_llm_response,
                analysis.is_own as i64,
            ],
        )
        .map_err(|e| KbError::Sqlite(e.to_string()))?;

        Ok(())
    }

    /// Return `true` if the repo is not in the DB or if `pushed_at` differs
    /// from the stored value (meaning new commits have been pushed).
    pub fn needs_analysis(
        &self,
        owner: &str,
        repo_name: &str,
        pushed_at: Option<DateTime<Utc>>,
    ) -> Result<bool, KbError> {
        let id = format!("{owner}/{repo_name}");
        let conn = self.conn.lock().expect("sqlite mutex poisoned");

        let stored: Option<Option<String>> = conn
            .query_row(
                "SELECT pushed_at FROM repos WHERE id = ?1",
                [&id],
                |row| row.get(0),
            )
            .ok();

        match stored {
            // Row does not exist → needs analysis.
            None => Ok(true),
            // Row exists — compare pushed_at.
            Some(stored_pushed_at) => {
                let incoming = pushed_at.as_ref().map(|dt| dt.to_rfc3339());
                Ok(incoming != stored_pushed_at)
            }
        }
    }

    /// Full-text search against the `repos_fts` virtual table.
    ///
    /// Each word in `query` is double-quoted before passing to FTS5 MATCH to
    /// prevent operator injection (FTS5 treats `AND`, `OR`, `*`, etc. as operators).
    pub fn search(&self, query: &str) -> Result<Vec<KbAnalysis>, KbError> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(vec![]);
        }

        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT r.id, r.owner, r.repo_name, r.url, r.stars, r.language,
                        r.topics, r.pushed_at, r.first_seen_at, r.last_seen_at,
                        r.analyzed_at, r.status, r.what, r.problem, r.architecture,
                        r.techniques, r.steal, r.uniqueness, r.raw_llm_response, r.is_own
                 FROM repos_fts
                 JOIN repos r ON repos_fts.id = r.id
                 WHERE repos_fts MATCH ?1
                 ORDER BY rank",
            )
            .map_err(|e| KbError::Sqlite(e.to_string()))?;

        let rows = stmt
            .query_map([&sanitized], row_to_kb_analysis)
            .map_err(|e| KbError::Sqlite(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| KbError::Sqlite(e.to_string()))?);
        }
        Ok(results)
    }

    /// Fetch a single entry by its composite ID `"owner/repo_name"`.
    pub fn get(&self, id: &str) -> Result<Option<KbAnalysis>, KbError> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let result = conn.query_row(
            "SELECT id, owner, repo_name, url, stars, language, topics,
                    pushed_at, first_seen_at, last_seen_at, analyzed_at,
                    status, what, problem, architecture, techniques, steal,
                    uniqueness, raw_llm_response, is_own
             FROM repos WHERE id = ?1",
            [id],
            row_to_kb_analysis,
        );

        match result {
            Ok(analysis) => Ok(Some(analysis)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(KbError::Sqlite(e.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Schema migrations
// ---------------------------------------------------------------------------

fn run_migrations(conn: &Connection) -> Result<(), KbError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS repos (
            id             TEXT PRIMARY KEY,
            owner          TEXT NOT NULL,
            repo_name      TEXT NOT NULL,
            url            TEXT NOT NULL,
            stars          INTEGER NOT NULL DEFAULT 0,
            language       TEXT,
            topics         TEXT NOT NULL DEFAULT '[]',
            pushed_at      TEXT,
            first_seen_at  TEXT NOT NULL,
            last_seen_at   TEXT NOT NULL,
            analyzed_at    TEXT,
            status         TEXT NOT NULL DEFAULT 'pending',
            what           TEXT NOT NULL DEFAULT '',
            problem        TEXT NOT NULL DEFAULT '',
            architecture   TEXT NOT NULL DEFAULT '',
            techniques     TEXT NOT NULL DEFAULT '[]',
            steal          TEXT NOT NULL DEFAULT '[]',
            uniqueness     TEXT NOT NULL DEFAULT '',
            raw_llm_response TEXT
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS repos_fts USING fts5(
            id UNINDEXED,
            what,
            problem,
            architecture,
            techniques,
            uniqueness,
            content='repos',
            content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS repos_ai AFTER INSERT ON repos BEGIN
            INSERT INTO repos_fts(rowid, id, what, problem, architecture, techniques, uniqueness)
            VALUES (new.rowid, new.id, new.what, new.problem, new.architecture, new.techniques, new.uniqueness);
        END;

        CREATE TRIGGER IF NOT EXISTS repos_au AFTER UPDATE ON repos BEGIN
            INSERT INTO repos_fts(repos_fts, rowid, id, what, problem, architecture, techniques, uniqueness)
            VALUES ('delete', old.rowid, old.id, old.what, old.problem, old.architecture, old.techniques, old.uniqueness);
            INSERT INTO repos_fts(rowid, id, what, problem, architecture, techniques, uniqueness)
            VALUES (new.rowid, new.id, new.what, new.problem, new.architecture, new.techniques, new.uniqueness);
        END;

        PRAGMA journal_mode=WAL;",
    )
    .map_err(|e| KbError::Sqlite(e.to_string()))?;

    // Guarded migration: add `is_own` column only if it doesn't exist yet.
    // This keeps run_migrations() idempotent on existing databases.
    let has_is_own: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('repos') WHERE name='is_own'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_is_own {
        conn.execute_batch(
            "ALTER TABLE repos ADD COLUMN is_own INTEGER NOT NULL DEFAULT 0",
        )
        .map_err(|e| KbError::Sqlite(e.to_string()))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Row mapping
// ---------------------------------------------------------------------------

fn row_to_kb_analysis(row: &rusqlite::Row<'_>) -> rusqlite::Result<KbAnalysis> {
    let id: String = row.get(0)?;
    let owner: String = row.get(1)?;
    let repo_name: String = row.get(2)?;
    let url: String = row.get(3)?;
    let stars: i64 = row.get(4)?;
    let language: Option<String> = row.get(5)?;
    let topics_json: String = row.get(6)?;
    let pushed_at_str: Option<String> = row.get(7)?;
    let first_seen_at_str: String = row.get(8)?;
    let last_seen_at_str: String = row.get(9)?;
    let analyzed_at_str: Option<String> = row.get(10)?;
    let status_str: String = row.get(11)?;
    let what: String = row.get(12)?;
    let problem: String = row.get(13)?;
    let architecture: String = row.get(14)?;
    let techniques_json: String = row.get(15)?;
    let steal_json: String = row.get(16)?;
    let uniqueness: String = row.get(17)?;
    let raw_llm_response: Option<String> = row.get(18)?;
    let is_own_int: i64 = row.get(19).unwrap_or(0);

    let _ = id; // composite key derived from owner/repo_name

    let topics: Vec<String> =
        serde_json::from_str(&topics_json).unwrap_or_default();
    let techniques: Vec<String> =
        serde_json::from_str(&techniques_json).unwrap_or_default();
    let steal: Vec<String> =
        serde_json::from_str(&steal_json).unwrap_or_default();

    let pushed_at = pushed_at_str.and_then(|s| s.parse::<DateTime<Utc>>().ok());
    let first_seen_at = first_seen_at_str
        .parse::<DateTime<Utc>>()
        .unwrap_or_else(|_| Utc::now());
    let last_seen_at = last_seen_at_str
        .parse::<DateTime<Utc>>()
        .unwrap_or_else(|_| Utc::now());
    let analyzed_at = analyzed_at_str.and_then(|s| s.parse::<DateTime<Utc>>().ok());

    let status = if status_str == "parse_failed" {
        KbAnalysisStatus::ParseFailed
    } else {
        KbAnalysisStatus::Complete
    };

    Ok(KbAnalysis {
        owner,
        repo_name,
        url,
        stars: stars as u64,
        language,
        topics,
        pushed_at,
        first_seen_at,
        last_seen_at,
        analyzed_at,
        status,
        what,
        problem,
        architecture,
        techniques,
        steal,
        uniqueness,
        raw_llm_response,
        is_own: is_own_int != 0,
    })
}

// ---------------------------------------------------------------------------
// FTS query sanitization
// ---------------------------------------------------------------------------

/// Double-quote each whitespace-separated token so FTS5 treats them as
/// literal phrases instead of operators.
fn sanitize_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|token| format!("\"{}\"", token.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn in_memory_kb() -> SqliteKb {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        run_migrations(&conn).expect("migrations");
        SqliteKb {
            conn: Arc::new(Mutex::new(conn)),
        }
    }

    fn sample_analysis(owner: &str, repo_name: &str) -> KbAnalysis {
        KbAnalysis {
            owner: owner.into(),
            repo_name: repo_name.into(),
            url: format!("https://github.com/{owner}/{repo_name}"),
            stars: 42,
            language: Some("Rust".into()),
            topics: vec!["cli".into()],
            pushed_at: Some(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap()),
            first_seen_at: Utc::now(),
            last_seen_at: Utc::now(),
            analyzed_at: Some(Utc::now()),
            status: KbAnalysisStatus::Complete,
            what: "A fast CLI tool".into(),
            problem: "Speed up repo analysis".into(),
            architecture: "hexagonal".into(),
            techniques: vec!["async".into(), "pipeline".into()],
            steal: vec!["plugin system".into()],
            uniqueness: "Blazing fast".into(),
            raw_llm_response: None,
            is_own: false,
        }
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().expect("open db");
        run_migrations(&conn).expect("first migration");
        run_migrations(&conn).expect("second migration — must be idempotent");
    }

    #[test]
    fn upsert_and_get_round_trip() {
        let kb = in_memory_kb();
        let analysis = sample_analysis("owner", "my-repo");

        kb.upsert(&analysis).expect("upsert");
        let fetched = kb.get("owner/my-repo").expect("get").expect("row exists");

        assert_eq!(fetched.owner, "owner");
        assert_eq!(fetched.repo_name, "my-repo");
        assert_eq!(fetched.what, "A fast CLI tool");
        assert_eq!(fetched.techniques, vec!["async", "pipeline"]);
    }

    #[test]
    fn upsert_twice_gives_single_row() {
        let kb = in_memory_kb();
        let mut analysis = sample_analysis("owner", "repo");
        kb.upsert(&analysis).expect("first upsert");

        analysis.what = "Updated description".into();
        kb.upsert(&analysis).expect("second upsert");

        let fetched = kb.get("owner/repo").expect("get").expect("row exists");
        assert_eq!(fetched.what, "Updated description");

        let conn = kb.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM repos WHERE id = 'owner/repo'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1, "upsert must not create duplicate rows");
    }

    #[test]
    fn get_returns_none_for_missing_id() {
        let kb = in_memory_kb();
        let result = kb.get("no/such").expect("get");
        assert!(result.is_none());
    }

    #[test]
    fn needs_analysis_true_when_not_in_db() {
        let kb = in_memory_kb();
        let result = kb.needs_analysis("owner", "unknown", None).expect("check");
        assert!(result, "absent repo must need analysis");
    }

    #[test]
    fn needs_analysis_false_when_pushed_at_matches() {
        let kb = in_memory_kb();
        let pushed = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
        let mut analysis = sample_analysis("owner", "repo");
        analysis.pushed_at = Some(pushed);
        kb.upsert(&analysis).expect("upsert");

        let result = kb
            .needs_analysis("owner", "repo", Some(pushed))
            .expect("check");
        assert!(!result, "same pushed_at means no analysis needed");
    }

    #[test]
    fn needs_analysis_true_when_pushed_at_differs() {
        let kb = in_memory_kb();
        let old = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let new = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();

        let mut analysis = sample_analysis("owner", "repo");
        analysis.pushed_at = Some(old);
        kb.upsert(&analysis).expect("upsert");

        let result = kb
            .needs_analysis("owner", "repo", Some(new))
            .expect("check");
        assert!(result, "different pushed_at means stale, needs re-analysis");
    }

    #[test]
    fn search_returns_matching_results() {
        let kb = in_memory_kb();
        kb.upsert(&sample_analysis("owner", "repo-a")).expect("upsert a");
        let mut b = sample_analysis("owner", "repo-b");
        b.what = "A database migration tool".into();
        b.problem = "Schema evolution is painful".into();
        kb.upsert(&b).expect("upsert b");

        let results = kb.search("migration").expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].repo_name, "repo-b");
    }

    #[test]
    fn search_returns_empty_on_no_match() {
        let kb = in_memory_kb();
        kb.upsert(&sample_analysis("owner", "repo")).expect("upsert");
        let results = kb.search("xyznonexistent").expect("search");
        assert!(results.is_empty());
    }

    #[test]
    fn search_sanitizes_special_chars() {
        let kb = in_memory_kb();
        kb.upsert(&sample_analysis("owner", "repo")).expect("upsert");
        // These would crash FTS5 without sanitization
        let results = kb.search("fast AND pipeline").expect("search with operators");
        // Should not panic; we just verify it runs cleanly
        let _ = results;
    }

    #[test]
    fn sanitize_fts_query_double_quotes_tokens() {
        assert_eq!(sanitize_fts_query("hello world"), "\"hello\" \"world\"");
        // FTS5 operators (AND, OR) and wildcards (*) become literal when double-quoted.
        assert_eq!(sanitize_fts_query("AND OR *"), "\"AND\" \"OR\" \"*\"");
        assert_eq!(sanitize_fts_query(""), "");
        // Embedded double-quotes are stripped to prevent injection.
        assert_eq!(sanitize_fts_query("he\"llo"), "\"hello\"");
    }

    // ── Phase 6.1: named integration tests ──────────────────────────────────

    #[test]
    fn kb_roundtrip_upsert_and_get() {
        let kb = in_memory_kb();
        let analysis = sample_analysis("acme", "roundtrip-repo");
        kb.upsert(&analysis).expect("upsert");

        let fetched = kb
            .get("acme/roundtrip-repo")
            .expect("get")
            .expect("row must exist");

        assert_eq!(fetched.owner, "acme");
        assert_eq!(fetched.repo_name, "roundtrip-repo");
        assert_eq!(fetched.url, "https://github.com/acme/roundtrip-repo");
        assert_eq!(fetched.stars, 42);
        assert_eq!(fetched.language.as_deref(), Some("Rust"));
        assert_eq!(fetched.topics, vec!["cli"]);
        assert_eq!(fetched.what, "A fast CLI tool");
        assert_eq!(fetched.problem, "Speed up repo analysis");
        assert_eq!(fetched.architecture, "hexagonal");
        assert_eq!(fetched.techniques, vec!["async", "pipeline"]);
        assert_eq!(fetched.steal, vec!["plugin system"]);
        assert_eq!(fetched.uniqueness, "Blazing fast");
        assert_eq!(fetched.status, KbAnalysisStatus::Complete);
    }

    #[test]
    fn kb_needs_analysis_false_when_pushed_at_matches() {
        let kb = in_memory_kb();
        let pushed = Utc.with_ymd_and_hms(2025, 3, 15, 10, 0, 0).unwrap();
        let mut analysis = sample_analysis("org", "cached-repo");
        analysis.pushed_at = Some(pushed);
        kb.upsert(&analysis).expect("upsert");

        let result = kb
            .needs_analysis("org", "cached-repo", Some(pushed))
            .expect("needs_analysis");
        assert!(!result, "same pushed_at must return false (cache hit)");
    }

    #[test]
    fn kb_needs_analysis_true_when_pushed_at_changes() {
        let kb = in_memory_kb();
        let t0 = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let t1 = t0 + chrono::Duration::days(1);

        let mut analysis = sample_analysis("org", "stale-repo");
        analysis.pushed_at = Some(t0);
        kb.upsert(&analysis).expect("upsert");

        let result = kb
            .needs_analysis("org", "stale-repo", Some(t1))
            .expect("needs_analysis");
        assert!(result, "different pushed_at (t0 + 1 day) must return true");
    }

    #[test]
    fn kb_fts_search_finds_matching_repo() {
        let kb = in_memory_kb();
        let mut analysis = sample_analysis("acme", "cli-rust-tool");
        analysis.what = "CLI tool for Rust developers".into();
        kb.upsert(&analysis).expect("upsert");

        let results = kb.search("rust developers").expect("search");
        assert_eq!(results.len(), 1, "should find the matching repo");
        assert_eq!(results[0].repo_name, "cli-rust-tool");
    }

    #[test]
    fn kb_fts_search_ignores_unrelated() {
        let kb = in_memory_kb();

        let mut a = sample_analysis("acme", "rust-tool");
        a.what = "CLI tool for Rust developers".into();
        a.problem = "Slow builds".into();
        a.architecture = "pipeline".into();
        a.uniqueness = "incremental compilation".into();
        kb.upsert(&a).expect("upsert a");

        let mut b = sample_analysis("acme", "python-scraper");
        b.what = "Web scraper written in Python".into();
        b.problem = "Data extraction".into();
        b.architecture = "monolith".into();
        b.uniqueness = "fast HTML parsing".into();
        kb.upsert(&b).expect("upsert b");

        // Search for a term that only appears in `a`
        let results = kb.search("rust").expect("search");
        assert_eq!(results.len(), 1, "only the Rust repo should match");
        assert_eq!(results[0].repo_name, "rust-tool");
    }

    #[test]
    fn kb_fts_special_chars_dont_crash() {
        let kb = in_memory_kb();
        kb.upsert(&sample_analysis("acme", "any-repo")).expect("upsert");

        // FTS5 boolean operators without sanitization would panic
        let results = kb.search("rust AND cli OR").expect("should not panic");
        // Result may be empty or non-empty — just verifying no crash
        let _ = results;
    }

    // ── Phase 5: is_own migration safety ────────────────────────────────────

    #[test]
    fn migration_is_idempotent_for_existing_db_with_is_own_column() {
        // Create DB, run migrations once (creates is_own column)
        // Run migrations AGAIN on the same DB (by opening it a second time)
        // Verify: no panic, no error, DB is still functional
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.sqlite");
        let _kb1 = SqliteKb::open(&path).expect("first open");
        let kb2 = SqliteKb::open(&path).expect("second open — must not fail");
        // Verify it's still usable
        let results = kb2.search("anything").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn upsert_and_get_preserves_is_own_true() {
        let dir = tempfile::TempDir::new().unwrap();
        let kb = SqliteKb::open(&dir.path().join("test.sqlite")).unwrap();

        let mut analysis = sample_analysis("local", "my-project");
        analysis.is_own = true;

        kb.upsert(&analysis).unwrap();
        let retrieved = kb
            .get("local/my-project")
            .unwrap()
            .expect("should exist");
        assert!(retrieved.is_own, "is_own should be true after round-trip");
    }
}
