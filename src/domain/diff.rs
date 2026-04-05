use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::domain::model::CrossRefResult;
use crate::infra::scan_store::ScanMeta;

/// Minimum absolute score delta to classify a repo as "changed".
pub const SCORE_CHANGE_THRESHOLD: f64 = 0.05_f64;

/// Overall result of comparing two scan snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanDiff {
    /// Metadata for the earlier (A) scan.
    pub scan_a: ScanMeta,
    /// Metadata for the later (B) scan.
    pub scan_b: ScanMeta,
    /// Repos present in B but not in A.
    pub new_repos: Vec<CrossRefResult>,
    /// Repos present in A but not in B.
    pub removed_repos: Vec<CrossRefResult>,
    /// Repos present in both scans with a score change >= threshold.
    pub changed_repos: Vec<RepoDiff>,
    /// Count of repos present in both scans with no significant change.
    pub unchanged_count: usize,
}

/// A single repo whose score changed between two scans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoDiff {
    /// The B (latest) version of the result.
    pub result: CrossRefResult,
    /// B.overall_relevance - A.overall_relevance (positive = improved).
    pub score_delta: f64,
    /// Ideas present in B but not in A (exact string match).
    pub new_ideas: Vec<String>,
}

/// Compare two scan snapshots and produce a [`ScanDiff`].
///
/// This is a pure function — no I/O, no side effects.
///
/// # Arguments
///
/// * `meta_a` — metadata for scan A (the earlier snapshot)
/// * `meta_b` — metadata for scan B (the later snapshot)
/// * `results_a` — full results from scan A
/// * `results_b` — full results from scan B
pub fn compute_diff(
    meta_a: ScanMeta,
    meta_b: ScanMeta,
    results_a: &[CrossRefResult],
    results_b: &[CrossRefResult],
) -> ScanDiff {
    // Build lookup maps keyed by "owner/repo_name"
    let map_a: HashMap<String, &CrossRefResult> = results_a
        .iter()
        .map(|r| {
            let key = format!(
                "{}/{}",
                r.analysis.candidate.owner, r.analysis.candidate.repo_name
            );
            (key, r)
        })
        .collect();

    let map_b: HashMap<String, &CrossRefResult> = results_b
        .iter()
        .map(|r| {
            let key = format!(
                "{}/{}",
                r.analysis.candidate.owner, r.analysis.candidate.repo_name
            );
            (key, r)
        })
        .collect();

    // Repos in B not in A → new
    let mut new_repos: Vec<CrossRefResult> = results_b
        .iter()
        .filter(|r| {
            let key = format!(
                "{}/{}",
                r.analysis.candidate.owner, r.analysis.candidate.repo_name
            );
            !map_a.contains_key(&key)
        })
        .cloned()
        .collect();
    new_repos.sort_by(|a, b| {
        b.overall_relevance
            .partial_cmp(&a.overall_relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Repos in A not in B → removed
    let mut removed_repos: Vec<CrossRefResult> = results_a
        .iter()
        .filter(|r| {
            let key = format!(
                "{}/{}",
                r.analysis.candidate.owner, r.analysis.candidate.repo_name
            );
            !map_b.contains_key(&key)
        })
        .cloned()
        .collect();
    removed_repos.sort_by(|a, b| {
        b.overall_relevance
            .partial_cmp(&a.overall_relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Repos in both → changed or unchanged
    let mut changed_repos: Vec<RepoDiff> = Vec::new();
    let mut unchanged_count = 0usize;

    for (key, result_b) in &map_b {
        if let Some(result_a) = map_a.get(key) {
            let delta = result_b.overall_relevance - result_a.overall_relevance;

            // New ideas: in B but not in A (exact string match)
            let ideas_a: std::collections::HashSet<&String> =
                result_a.ideas.iter().collect();
            let new_ideas: Vec<String> = result_b
                .ideas
                .iter()
                .filter(|idea| !ideas_a.contains(idea))
                .cloned()
                .collect();

            if delta.abs() >= SCORE_CHANGE_THRESHOLD {
                changed_repos.push(RepoDiff {
                    result: (*result_b).clone(),
                    score_delta: delta,
                    new_ideas,
                });
            } else {
                unchanged_count += 1;
            }
        }
    }

    // Sort changed repos by absolute delta descending
    changed_repos.sort_by(|a, b| {
        b.score_delta
            .abs()
            .partial_cmp(&a.score_delta.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    ScanDiff {
        scan_a: meta_a,
        scan_b: meta_b,
        new_repos,
        removed_repos,
        changed_repos,
        unchanged_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{AnalysisResult, FeedEntry, RepoCandidate, RepoMatch};
    use chrono::Utc;
    use url::Url;

    fn make_meta(id: &str) -> ScanMeta {
        ScanMeta {
            id: id.to_string(),
            scanned_at: Utc::now(),
            result_count: 0,
        }
    }

    fn make_result(owner: &str, repo: &str, relevance: f64, ideas: Vec<String>) -> CrossRefResult {
        CrossRefResult {
            analysis: AnalysisResult {
                candidate: RepoCandidate {
                    entry: FeedEntry {
                        title: repo.to_string(),
                        repo_url: Url::parse(&format!("https://github.com/{owner}/{repo}"))
                            .unwrap(),
                        description: None,
                        published: None,
                        source_name: "test".to_string(),
                    },
                    stars: 100,
                    language: Some("Rust".to_string()),
                    topics: vec![],
                    fork: false,
                    archived: false,
                    owner: owner.to_string(),
                    repo_name: repo.to_string(),
                    category: Default::default(),
                },
                summary: "test".to_string(),
                key_features: vec![],
                tech_stack: vec![],
                relevance_score: relevance,
            },
            matched_repos: vec![RepoMatch {
                own_repo: "my/repo".to_string(),
                relevance: 0.5,
                reason: "test".to_string(),
            }],
            ideas,
            overall_relevance: relevance,
        }
    }

    #[test]
    fn test_compute_diff_classifies_all_categories() {
        let results_a = vec![
            make_result("owner", "stable", 0.7, vec!["idea-a".to_string()]),
            make_result("owner", "changed", 0.5, vec![]),
            make_result("owner", "removed", 0.6, vec![]),
        ];
        let results_b = vec![
            make_result("owner", "stable", 0.7, vec!["idea-a".to_string()]),
            make_result("owner", "changed", 0.65, vec![]),
            make_result("owner", "new-repo", 0.8, vec![]),
        ];

        let diff = compute_diff(
            make_meta("scan-a"),
            make_meta("scan-b"),
            &results_a,
            &results_b,
        );

        assert_eq!(diff.new_repos.len(), 1, "should find 1 new repo");
        assert_eq!(
            diff.new_repos[0].analysis.candidate.repo_name,
            "new-repo"
        );

        assert_eq!(diff.removed_repos.len(), 1, "should find 1 removed repo");
        assert_eq!(
            diff.removed_repos[0].analysis.candidate.repo_name,
            "removed"
        );

        assert_eq!(diff.changed_repos.len(), 1, "should find 1 changed repo");
        let changed = &diff.changed_repos[0];
        assert_eq!(changed.result.analysis.candidate.repo_name, "changed");
        assert!((changed.score_delta - 0.15).abs() < 1e-10);

        assert_eq!(diff.unchanged_count, 1, "stable should be unchanged");
    }

    #[test]
    fn test_threshold_boundary() {
        // delta exactly 0.05 → changed
        let results_a = vec![make_result("owner", "exact", 0.5, vec![])];
        let results_b_exact = vec![make_result("owner", "exact", 0.55, vec![])];
        let diff = compute_diff(
            make_meta("a"),
            make_meta("b"),
            &results_a,
            &results_b_exact,
        );
        assert_eq!(
            diff.changed_repos.len(),
            1,
            "delta == 0.05 should be classified as changed"
        );
        assert_eq!(diff.unchanged_count, 0);

        // delta 0.049 → unchanged
        let results_b_just_below = vec![make_result("owner", "exact", 0.549, vec![])];
        let diff2 = compute_diff(
            make_meta("a"),
            make_meta("b"),
            &results_a,
            &results_b_just_below,
        );
        assert_eq!(
            diff2.changed_repos.len(),
            0,
            "delta < 0.05 should be unchanged"
        );
        assert_eq!(diff2.unchanged_count, 1);
    }

    #[test]
    fn test_empty_scan_a() {
        // All B repos should appear as new when A is empty
        let results_a: Vec<CrossRefResult> = vec![];
        let results_b = vec![
            make_result("owner", "repo-1", 0.8, vec![]),
            make_result("owner", "repo-2", 0.6, vec![]),
        ];

        let diff = compute_diff(make_meta("a"), make_meta("b"), &results_a, &results_b);

        assert_eq!(diff.new_repos.len(), 2, "all repos should be new");
        assert!(diff.removed_repos.is_empty());
        assert!(diff.changed_repos.is_empty());
        assert_eq!(diff.unchanged_count, 0);
    }

    #[test]
    fn test_empty_scan_b() {
        // All A repos should appear as removed when B is empty
        let results_a = vec![
            make_result("owner", "repo-1", 0.8, vec![]),
            make_result("owner", "repo-2", 0.6, vec![]),
        ];
        let results_b: Vec<CrossRefResult> = vec![];

        let diff = compute_diff(make_meta("a"), make_meta("b"), &results_a, &results_b);

        assert!(diff.new_repos.is_empty());
        assert_eq!(diff.removed_repos.len(), 2, "all repos should be removed");
        assert!(diff.changed_repos.is_empty());
        assert_eq!(diff.unchanged_count, 0);
    }

    #[test]
    fn test_both_scans_empty() {
        let results_a: Vec<CrossRefResult> = vec![];
        let results_b: Vec<CrossRefResult> = vec![];

        let diff = compute_diff(make_meta("a"), make_meta("b"), &results_a, &results_b);

        assert!(diff.new_repos.is_empty());
        assert!(diff.removed_repos.is_empty());
        assert!(diff.changed_repos.is_empty());
        assert_eq!(diff.unchanged_count, 0);
    }

    #[test]
    fn test_same_score_not_classified_as_changed() {
        // Repos with exactly the same score must land in unchanged_count, never in changed_repos
        let results_a = vec![
            make_result("owner", "repo-a", 0.7, vec![]),
            make_result("owner", "repo-b", 0.5, vec![]),
        ];
        let results_b = vec![
            make_result("owner", "repo-a", 0.7, vec![]),
            make_result("owner", "repo-b", 0.5, vec![]),
        ];

        let diff = compute_diff(make_meta("a"), make_meta("b"), &results_a, &results_b);

        assert!(diff.changed_repos.is_empty(), "identical scores must not produce changed repos");
        assert_eq!(diff.unchanged_count, 2);
        assert!(diff.new_repos.is_empty());
        assert!(diff.removed_repos.is_empty());
    }

    #[test]
    fn test_large_diff_mixed_categories() {
        // 10 repos: 4 new, 3 removed, 2 changed (delta >= 0.05), 1 unchanged
        let results_a: Vec<_> = vec![
            make_result("owner", "stable", 0.5, vec![]),       // unchanged (delta=0)
            make_result("owner", "changed-up", 0.4, vec![]),   // delta +0.2 → changed
            make_result("owner", "changed-down", 0.8, vec![]), // delta -0.2 → changed
            make_result("owner", "removed-1", 0.6, vec![]),
            make_result("owner", "removed-2", 0.7, vec![]),
            make_result("owner", "removed-3", 0.5, vec![]),
        ];
        let results_b: Vec<_> = vec![
            make_result("owner", "stable", 0.5, vec![]),        // unchanged
            make_result("owner", "changed-up", 0.6, vec![]),    // delta +0.2
            make_result("owner", "changed-down", 0.6, vec![]),  // delta -0.2
            make_result("owner", "new-1", 0.9, vec![]),
            make_result("owner", "new-2", 0.85, vec![]),
            make_result("owner", "new-3", 0.75, vec![]),
            make_result("owner", "new-4", 0.65, vec![]),
        ];

        let diff = compute_diff(make_meta("a"), make_meta("b"), &results_a, &results_b);

        assert_eq!(diff.new_repos.len(), 4, "expected 4 new repos");
        assert_eq!(diff.removed_repos.len(), 3, "expected 3 removed repos");
        assert_eq!(diff.changed_repos.len(), 2, "expected 2 changed repos");
        assert_eq!(diff.unchanged_count, 1, "expected 1 unchanged repo");

        // New repos sorted by relevance descending
        assert!(
            diff.new_repos[0].overall_relevance >= diff.new_repos[1].overall_relevance,
            "new_repos must be sorted by relevance desc"
        );

        // Changed repos sorted by absolute delta descending
        let deltas: Vec<f64> = diff.changed_repos.iter().map(|r| r.score_delta.abs()).collect();
        assert!(deltas[0] >= deltas[deltas.len() - 1], "changed_repos must be sorted by |delta| desc");
    }

    #[test]
    fn test_threshold_exactly_below_does_not_change() {
        // delta = 0.0499... must stay as unchanged (just below threshold)
        let results_a = vec![make_result("owner", "repo", 0.5, vec![])];
        let results_b = vec![make_result("owner", "repo", 0.5499, vec![])];

        let diff = compute_diff(make_meta("a"), make_meta("b"), &results_a, &results_b);

        assert!(diff.changed_repos.is_empty(), "delta < 0.05 must be unchanged");
        assert_eq!(diff.unchanged_count, 1);
    }

    #[test]
    fn test_negative_delta_classified_as_changed() {
        // Score drops by more than threshold → should appear in changed_repos with negative delta
        let results_a = vec![make_result("owner", "repo", 0.8, vec![])];
        let results_b = vec![make_result("owner", "repo", 0.7, vec![])];

        let diff = compute_diff(make_meta("a"), make_meta("b"), &results_a, &results_b);

        assert_eq!(diff.changed_repos.len(), 1);
        assert!(diff.changed_repos[0].score_delta < 0.0, "negative delta expected");
        assert!(
            (diff.changed_repos[0].score_delta - (-0.1)).abs() < 1e-10,
            "delta should be -0.1"
        );
    }

    #[test]
    fn test_new_ideas_detection() {
        let results_a = vec![make_result(
            "owner",
            "repo",
            0.7,
            vec!["old-idea".to_string()],
        )];
        let results_b = vec![make_result(
            "owner",
            "repo",
            0.8,
            vec!["old-idea".to_string(), "new-idea".to_string()],
        )];

        let diff = compute_diff(make_meta("a"), make_meta("b"), &results_a, &results_b);

        assert_eq!(diff.changed_repos.len(), 1);
        let repo_diff = &diff.changed_repos[0];
        assert_eq!(repo_diff.new_ideas, vec!["new-idea".to_string()]);
    }
}
