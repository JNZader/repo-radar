#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::collections::HashSet;
use std::future::Future;

use tracing::{info, warn};

use crate::domain::crossref::CrossRef;
use crate::domain::model::{AnalysisResult, CrossRefResult, RepoMatch};
use crate::infra::error::CrossRefError;

/// A user's own repository with language and topics for similarity matching.
#[derive(Debug, Clone)]
struct OwnRepo {
    name: String,
    language: Option<String>,
    topics: Vec<String>,
}

/// Cross-references discovered repos against the authenticated GitHub user's own repos.
pub struct GitHubCrossRef {
    username: String,
    octocrab: octocrab::Octocrab,
}

impl GitHubCrossRef {
    /// Create a new `GitHubCrossRef`.
    ///
    /// # Errors
    ///
    /// Returns `CrossRefError::Network` if the octocrab client cannot be built.
    pub fn new(username: String, token: Option<&str>) -> Result<Self, CrossRefError> {
        let mut builder = octocrab::Octocrab::builder();
        if let Some(t) = token {
            builder = builder.personal_token(t.to_owned());
        }
        let octocrab = builder
            .build()
            .map_err(|e| CrossRefError::Network(format!("failed to build GitHub client: {e}")))?;
        Ok(Self { username, octocrab })
    }

    /// Create from an existing octocrab instance (useful for testing with custom base URL).
    #[doc(hidden)]
    pub fn with_octocrab(username: String, octocrab: octocrab::Octocrab) -> Self {
        Self { username, octocrab }
    }

    /// Fetch the authenticated user's own public repos via octocrab.
    async fn fetch_own_repos(&self) -> Result<Vec<OwnRepo>, CrossRefError> {
        let page = self
            .octocrab
            .get::<Vec<serde_json::Value>, _, _>(
                format!("/users/{}/repos?per_page=100&type=owner", self.username),
                None::<&()>,
            )
            .await
            .map_err(|e| CrossRefError::Network(format!("failed to fetch own repos: {e}")))?;

        let repos = page
            .into_iter()
            .map(|r| {
                let name = r["full_name"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let language = r["language"].as_str().map(String::from);
                let topics = r["topics"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                OwnRepo {
                    name,
                    language,
                    topics,
                }
            })
            .collect();

        Ok(repos)
    }

    /// Compute Jaccard similarity between an analysis result and a user's own repo,
    /// using case-insensitive comparison of language + topics.
    fn compute_similarity(analysis: &AnalysisResult, own_repo: &OwnRepo) -> f64 {
        let mut analysis_set: HashSet<String> = analysis
            .candidate
            .topics
            .iter()
            .map(|t| t.to_ascii_lowercase())
            .collect();
        if let Some(ref lang) = analysis.candidate.language {
            analysis_set.insert(lang.to_ascii_lowercase());
        }

        let mut own_set: HashSet<String> = own_repo
            .topics
            .iter()
            .map(|t| t.to_ascii_lowercase())
            .collect();
        if let Some(ref lang) = own_repo.language {
            own_set.insert(lang.to_ascii_lowercase());
        }

        if analysis_set.is_empty() && own_set.is_empty() {
            return 0.0;
        }

        let intersection = analysis_set.intersection(&own_set).count();
        let union = analysis_set.union(&own_set).count();

        if union == 0 {
            0.0
        } else {
            intersection as f64 / union as f64
        }
    }
}

impl CrossRef for GitHubCrossRef {
    fn cross_reference(
        &self,
        results: Vec<AnalysisResult>,
    ) -> impl Future<Output = Result<Vec<CrossRefResult>, CrossRefError>> + Send {
        async move {
            let own_repos = self.fetch_own_repos().await?;

            if own_repos.is_empty() {
                info!(username = %self.username, "no own repos found, returning results without matches");
                return Ok(results
                    .into_iter()
                    .map(|analysis| CrossRefResult {
                        analysis,
                        matched_repos: vec![],
                        ideas: vec![],
                        overall_relevance: 0.0,
                    })
                    .collect());
            }

            let mut crossref_results = Vec::with_capacity(results.len());

            for analysis in results {
                let mut matches = Vec::new();

                for own_repo in &own_repos {
                    let score = Self::compute_similarity(&analysis, own_repo);
                    if score > 0.0 {
                        let overlapping: Vec<String> = {
                            let a_set: HashSet<String> = analysis
                                .candidate
                                .topics
                                .iter()
                                .map(|t| t.to_ascii_lowercase())
                                .collect();
                            let o_set: HashSet<String> = own_repo
                                .topics
                                .iter()
                                .map(|t| t.to_ascii_lowercase())
                                .collect();
                            a_set.intersection(&o_set).cloned().collect()
                        };
                        let lang_match = match (&analysis.candidate.language, &own_repo.language) {
                            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => {
                                format!("shared language: {a}")
                            }
                            _ => String::new(),
                        };
                        let mut reason_parts = Vec::new();
                        if !lang_match.is_empty() {
                            reason_parts.push(lang_match);
                        }
                        if !overlapping.is_empty() {
                            reason_parts.push(format!("shared topics: {}", overlapping.join(", ")));
                        }
                        matches.push(RepoMatch {
                            own_repo: own_repo.name.clone(),
                            relevance: score,
                            reason: reason_parts.join("; "),
                        });
                    }
                }

                let overall_relevance = if matches.is_empty() {
                    0.0
                } else {
                    matches.iter().map(|m| m.relevance).sum::<f64>() / matches.len() as f64
                };

                let ideas: Vec<String> = matches
                    .iter()
                    .filter(|m| m.relevance > 0.2)
                    .map(|m| {
                        format!(
                            "Explore patterns from {} — overlaps with {}",
                            analysis.candidate.repo_name, m.own_repo
                        )
                    })
                    .collect();

                if !matches.is_empty() {
                    info!(
                        repo = %analysis.candidate.repo_name,
                        match_count = matches.len(),
                        overall_relevance,
                        "cross-reference matches found"
                    );
                } else {
                    warn!(
                        repo = %analysis.candidate.repo_name,
                        "no cross-reference matches"
                    );
                }

                crossref_results.push(CrossRefResult {
                    analysis,
                    matched_repos: matches,
                    ideas,
                    overall_relevance,
                });
            }

            Ok(crossref_results)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use url::Url;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::domain::model::{FeedEntry, RepoCandidate};

    fn sample_analysis(
        language: Option<&str>,
        topics: &[&str],
    ) -> AnalysisResult {
        AnalysisResult {
            candidate: RepoCandidate {
                entry: FeedEntry {
                    title: "test-repo".into(),
                    repo_url: Url::parse("https://github.com/test/test-repo").unwrap(),
                    description: Some("A test repo".into()),
                    published: Some(Utc::now()),
                    source_name: "test-feed".into(),
                },
                stars: 100,
                language: language.map(String::from),
                topics: topics.iter().map(|t| (*t).to_string()).collect(),
                fork: false,
                archived: false,
                owner: "test".into(),
                repo_name: "test-repo".into(),
                category: Default::default(),
            },
            summary: "A great test repo".into(),
            key_features: vec!["fast".into()],
            tech_stack: vec!["Rust".into()],
            relevance_score: 0.8,
        }
    }

    fn own_repos_json(repos: &[(&str, &str, Option<&str>, &[&str])]) -> serde_json::Value {
        let arr: Vec<serde_json::Value> = repos
            .iter()
            .map(|(owner, name, lang, topics)| {
                let topics_arr: Vec<serde_json::Value> = topics
                    .iter()
                    .map(|t| serde_json::Value::String((*t).to_string()))
                    .collect();
                serde_json::json!({
                    "full_name": format!("{owner}/{name}"),
                    "language": lang,
                    "topics": topics_arr,
                })
            })
            .collect();
        serde_json::Value::Array(arr)
    }

    fn build_crossref_with_mock(server: &MockServer, username: &str) -> GitHubCrossRef {
        let octocrab = octocrab::Octocrab::builder()
            .base_uri(server.uri())
            .expect("valid URI")
            .build()
            .expect("build octocrab");
        GitHubCrossRef::with_octocrab(username.to_string(), octocrab)
    }

    #[tokio::test]
    async fn overlap_matching_language_and_topics() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/testuser/repos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(own_repos_json(&[(
                "testuser",
                "my-cli",
                Some("Rust"),
                &["cli", "tooling"],
            )])))
            .mount(&server)
            .await;

        let crossref = build_crossref_with_mock(&server, "testuser");
        let analysis = sample_analysis(Some("Rust"), &["cli", "async"]);
        let results = crossref.cross_reference(vec![analysis]).await.unwrap();

        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert!(!result.matched_repos.is_empty(), "should have at least one match");
        let m = &result.matched_repos[0];
        assert_eq!(m.own_repo, "testuser/my-cli");
        assert!(m.relevance > 0.0, "relevance should be positive for overlapping language+topics");
        assert!(result.overall_relevance > 0.0);
    }

    #[tokio::test]
    async fn empty_own_repos_returns_zero_relevance() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/testuser/repos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let crossref = build_crossref_with_mock(&server, "testuser");
        let analysis = sample_analysis(Some("Rust"), &["cli"]);
        let results = crossref.cross_reference(vec![analysis]).await.unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].matched_repos.is_empty());
        assert_eq!(results[0].overall_relevance, 0.0);
    }

    #[tokio::test]
    async fn api_error_500_returns_network_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/testuser/repos"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&server)
            .await;

        let crossref = build_crossref_with_mock(&server, "testuser");
        let analysis = sample_analysis(Some("Rust"), &["cli"]);
        let result = crossref.cross_reference(vec![analysis]).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            CrossRefError::Network(msg) => {
                assert!(msg.contains("failed to fetch own repos"), "error message: {msg}");
            }
            other => panic!("expected CrossRefError::Network, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_overlap_returns_empty_matches() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/testuser/repos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(own_repos_json(&[(
                "testuser",
                "my-python-lib",
                Some("Python"),
                &["data-science", "ml"],
            )])))
            .mount(&server)
            .await;

        let crossref = build_crossref_with_mock(&server, "testuser");
        let analysis = sample_analysis(Some("Rust"), &["cli", "tooling"]);
        let results = crossref.cross_reference(vec![analysis]).await.unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].matched_repos.is_empty(), "no overlap should yield empty matches");
        assert_eq!(results[0].overall_relevance, 0.0);
    }

    #[tokio::test]
    async fn multiple_matches_with_varying_scores() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/testuser/repos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(own_repos_json(&[
                ("testuser", "rust-cli", Some("Rust"), &["cli"]),
                ("testuser", "rust-web", Some("Rust"), &["web", "async"]),
                ("testuser", "go-service", Some("Go"), &["microservice"]),
            ])))
            .mount(&server)
            .await;

        let crossref = build_crossref_with_mock(&server, "testuser");
        // Analysis has Rust + cli + async — should match rust-cli (Rust, cli) and rust-web (Rust, async)
        let analysis = sample_analysis(Some("Rust"), &["cli", "async"]);
        let results = crossref.cross_reference(vec![analysis]).await.unwrap();

        assert_eq!(results.len(), 1);
        let result = &results[0];
        // Should match rust-cli and rust-web, but NOT go-service
        assert_eq!(
            result.matched_repos.len(),
            2,
            "should match 2 repos: rust-cli and rust-web"
        );

        let names: Vec<&str> = result.matched_repos.iter().map(|m| m.own_repo.as_str()).collect();
        assert!(names.contains(&"testuser/rust-cli"));
        assert!(names.contains(&"testuser/rust-web"));

        // Scores should differ because overlap sets are different
        let cli_match = result.matched_repos.iter().find(|m| m.own_repo == "testuser/rust-cli").unwrap();
        let web_match = result.matched_repos.iter().find(|m| m.own_repo == "testuser/rust-web").unwrap();
        // rust-cli: {rust, cli} vs {rust, cli, async} → intersection=2, union=3 → 0.667
        // rust-web: {rust, web, async} vs {rust, cli, async} → intersection=2, union=4 → 0.5
        assert!(
            cli_match.relevance > web_match.relevance,
            "rust-cli should have higher relevance than rust-web"
        );
        assert!(result.overall_relevance > 0.0);
    }
}
