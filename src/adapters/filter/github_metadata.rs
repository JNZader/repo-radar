#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;

use tracing::{info, warn};
use url::Url;

use crate::config::FilterConfig;
use crate::domain::filter::Filter;
use crate::domain::model::{FeedEntry, RepoCandidate};
use crate::infra::error::FilterError;

/// Fetches GitHub repo metadata and filters candidates by configured criteria.
pub struct GitHubMetadataFilter {
    config: FilterConfig,
    octocrab: octocrab::Octocrab,
}

impl GitHubMetadataFilter {
    /// Create a new `GitHubMetadataFilter`.
    ///
    /// # Errors
    ///
    /// Returns `FilterError::GitHubApi` if the octocrab client cannot be built.
    pub fn new(config: FilterConfig, token: Option<&str>) -> Result<Self, FilterError> {
        let mut builder = octocrab::Octocrab::builder();
        if let Some(t) = token {
            builder = builder.personal_token(t.to_owned());
        }
        let octocrab = builder
            .build()
            .map_err(|e| FilterError::GitHubApi(format!("failed to build GitHub client: {e}")))?;
        Ok(Self { config, octocrab })
    }

    /// Create from an existing octocrab instance (useful for testing with custom base URL).
    #[doc(hidden)]
    pub fn with_octocrab(config: FilterConfig, octocrab: octocrab::Octocrab) -> Self {
        Self { config, octocrab }
    }
}

impl Filter for GitHubMetadataFilter {
    fn filter(
        &self,
        entries: Vec<FeedEntry>,
    ) -> impl Future<Output = Result<Vec<RepoCandidate>, FilterError>> + Send {
        let config = self.config.clone();
        let octocrab = self.octocrab.clone();
        async move { filter_entries(&entries, &config, &octocrab).await }
    }
}

async fn filter_entries(
    entries: &[FeedEntry],
    config: &FilterConfig,
    octocrab: &octocrab::Octocrab,
) -> Result<Vec<RepoCandidate>, FilterError> {
    let mut candidates = Vec::new();

    for entry in entries {
        let Some((owner, repo)) = parse_owner_repo(&entry.repo_url) else {
            warn!(url = %entry.repo_url, "could not parse owner/repo from URL, skipping");
            continue;
        };

        info!(owner = %owner, repo = %repo, "fetching GitHub metadata");

        let repo_data = match octocrab.repos(&owner, &repo).get().await {
            Ok(r) => r,
            Err(octocrab::Error::GitHub { source, .. })
                if source.message.contains("Not Found") =>
            {
                warn!(owner = %owner, repo = %repo, "repo not found (404), skipping");
                continue;
            }
            Err(e) => {
                // Check if it's a generic HTTP 404 or other error
                let err_str = e.to_string();
                if err_str.contains("404") {
                    warn!(owner = %owner, repo = %repo, "repo not found (404), skipping");
                    continue;
                }
                return Err(FilterError::GitHubApi(format!(
                    "failed to fetch {owner}/{repo}: {e}"
                )));
            }
        };

        let stars = u64::from(repo_data.stargazers_count.unwrap_or(0));
        let language = repo_data
            .language
            .as_ref()
            .and_then(|v| v.as_str().map(String::from));
        let is_fork = repo_data.fork.unwrap_or(false);
        let is_archived = repo_data.archived.unwrap_or(false);
        let topics = repo_data
            .topics
            .clone()
            .unwrap_or_default();
        let _description = repo_data.description.clone();

        tracing::debug!(
            %owner, %repo, stars, ?language, is_fork, is_archived, ?topics,
            "fetched repo metadata"
        );

        // Apply filters
        if stars < config.min_stars {
            info!(owner = %owner, repo = %repo, stars, min = config.min_stars, "below min stars, skipping");
            continue;
        }

        if config.exclude_forks && is_fork {
            info!(owner = %owner, repo = %repo, "is a fork, skipping");
            continue;
        }

        if config.exclude_archived && is_archived {
            info!(owner = %owner, repo = %repo, "is archived, skipping");
            continue;
        }

        if !config.languages.is_empty() {
            let matches = language
                .as_ref()
                .is_some_and(|lang| {
                    config
                        .languages
                        .iter()
                        .any(|l| l.eq_ignore_ascii_case(lang))
                });
            if !matches {
                info!(
                    owner = %owner,
                    repo = %repo,
                    language = language.as_deref().unwrap_or("none"),
                    "language not in allowed list, skipping"
                );
                continue;
            }
        }

        if !config.topics.is_empty() {
            let has_overlap = topics.iter().any(|t| {
                config
                    .topics
                    .iter()
                    .any(|ct| ct.eq_ignore_ascii_case(t))
            });
            if !has_overlap {
                info!(owner = %owner, repo = %repo, "no topic overlap, skipping");
                continue;
            }
        }

        candidates.push(RepoCandidate {
            entry: entry.clone(),
            stars,
            language,
            topics,
            fork: is_fork,
            archived: is_archived,
            owner: owner.clone(),
            repo_name: repo.clone(),
        });
    }

    Ok(candidates)
}

/// Parse `owner` and `repo` from a GitHub URL like `https://github.com/owner/repo`.
fn parse_owner_repo(url: &Url) -> Option<(String, String)> {
    if url.host_str() != Some("github.com") {
        return None;
    }
    let mut segments = url.path_segments()?;
    let owner = segments.next().filter(|s| !s.is_empty())?;
    let repo = segments.next().filter(|s| !s.is_empty())?;
    // Strip .git suffix if present
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    Some((owner.to_string(), repo.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_entry(owner: &str, repo: &str) -> FeedEntry {
        FeedEntry {
            title: format!("{repo}"),
            repo_url: Url::parse(&format!("https://github.com/{owner}/{repo}")).unwrap(),
            description: Some("A test repo".into()),
            published: Some(Utc::now()),
            source_name: "test-feed".into(),
        }
    }

    fn github_repo_json(
        owner: &str,
        repo: &str,
        stars: u64,
        language: Option<&str>,
        fork: bool,
        archived: bool,
        topics: &[&str],
    ) -> serde_json::Value {
        let lang_str = language
            .map(|l| format!("\"{l}\""))
            .unwrap_or_else(|| "null".into());
        let topics_str = topics
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(",");
        let api = "https://api.github.com";
        let json_str = format!(
            r#"{{
  "id":12345,"node_id":"R_kgDOABCDEF","name":"{repo}","full_name":"{owner}/{repo}",
  "private":false,
  "owner":{{"login":"{owner}","id":1,"node_id":"MDQ6VXNlcjE=",
    "avatar_url":"https://avatars.githubusercontent.com/u/1",
    "url":"{api}/users/{owner}",
    "html_url":"https://github.com/{owner}",
    "type":"User","site_admin":false,"gravatar_id":"",
    "followers_url":"{api}/users/{owner}/followers",
    "following_url":"{api}/users/{owner}/following{{/other_user}}",
    "gists_url":"{api}/users/{owner}/gists{{/gist_id}}",
    "starred_url":"{api}/users/{owner}/starred{{/owner}}{{/repo}}",
    "subscriptions_url":"{api}/users/{owner}/subscriptions",
    "organizations_url":"{api}/users/{owner}/orgs",
    "repos_url":"{api}/users/{owner}/repos",
    "events_url":"{api}/users/{owner}/events{{/privacy}}",
    "received_events_url":"{api}/users/{owner}/received_events"}},
  "html_url":"https://github.com/{owner}/{repo}",
  "description":"Description of {repo}",
  "fork":{fork},"archived":{archived},"disabled":false,
  "stargazers_count":{stars},"watchers_count":{stars},"forks_count":10,"open_issues_count":5,
  "language":{lang_str},"topics":[{topics_str}],"default_branch":"main",
  "url":"{api}/repos/{owner}/{repo}",
  "forks_url":"{api}/repos/{owner}/{repo}/forks",
  "keys_url":"{api}/repos/{owner}/{repo}/keys{{/key_id}}",
  "collaborators_url":"{api}/repos/{owner}/{repo}/collaborators{{/collaborator}}",
  "teams_url":"{api}/repos/{owner}/{repo}/teams",
  "hooks_url":"{api}/repos/{owner}/{repo}/hooks",
  "issue_events_url":"{api}/repos/{owner}/{repo}/issues/events{{/number}}",
  "events_url":"{api}/repos/{owner}/{repo}/events",
  "assignees_url":"{api}/repos/{owner}/{repo}/assignees{{/user}}",
  "branches_url":"{api}/repos/{owner}/{repo}/branches{{/branch}}",
  "tags_url":"{api}/repos/{owner}/{repo}/tags",
  "blobs_url":"{api}/repos/{owner}/{repo}/git/blobs{{/sha}}",
  "git_tags_url":"{api}/repos/{owner}/{repo}/git/tags{{/sha}}",
  "git_refs_url":"{api}/repos/{owner}/{repo}/git/refs{{/sha}}",
  "trees_url":"{api}/repos/{owner}/{repo}/git/trees{{/sha}}",
  "statuses_url":"{api}/repos/{owner}/{repo}/statuses/{{sha}}",
  "languages_url":"{api}/repos/{owner}/{repo}/languages",
  "stargazers_url":"{api}/repos/{owner}/{repo}/stargazers",
  "contributors_url":"{api}/repos/{owner}/{repo}/contributors",
  "subscribers_url":"{api}/repos/{owner}/{repo}/subscribers",
  "subscription_url":"{api}/repos/{owner}/{repo}/subscription",
  "commits_url":"{api}/repos/{owner}/{repo}/commits{{/sha}}",
  "git_commits_url":"{api}/repos/{owner}/{repo}/git/commits{{/sha}}",
  "comments_url":"{api}/repos/{owner}/{repo}/comments{{/number}}",
  "issue_comment_url":"{api}/repos/{owner}/{repo}/issues/comments{{/number}}",
  "contents_url":"{api}/repos/{owner}/{repo}/contents/{{+path}}",
  "compare_url":"{api}/repos/{owner}/{repo}/compare/{{base}}...{{head}}",
  "merges_url":"{api}/repos/{owner}/{repo}/merges",
  "archive_url":"{api}/repos/{owner}/{repo}/{{archive_format}}{{/ref}}",
  "downloads_url":"{api}/repos/{owner}/{repo}/downloads",
  "issues_url":"{api}/repos/{owner}/{repo}/issues{{/number}}",
  "pulls_url":"{api}/repos/{owner}/{repo}/pulls{{/number}}",
  "milestones_url":"{api}/repos/{owner}/{repo}/milestones{{/number}}",
  "notifications_url":"{api}/repos/{owner}/{repo}/notifications{{?since,all,participating}}",
  "labels_url":"{api}/repos/{owner}/{repo}/labels{{/name}}",
  "releases_url":"{api}/repos/{owner}/{repo}/releases{{/id}}",
  "deployments_url":"{api}/repos/{owner}/{repo}/deployments",
  "git_url":"git://github.com/{owner}/{repo}.git",
  "ssh_url":"git@github.com:{owner}/{repo}.git",
  "clone_url":"https://github.com/{owner}/{repo}.git",
  "svn_url":"https://github.com/{owner}/{repo}",
  "size":1024,"has_issues":true,"has_projects":true,"has_downloads":true,
  "has_wiki":true,"has_pages":false,"is_template":false,"visibility":"public",
  "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-03-01T00:00:00Z",
  "pushed_at":"2026-03-01T00:00:00Z"
}}"#
        );
        serde_json::from_str(&json_str).expect("valid github repo JSON fixture")
    }

    async fn setup_mock_github(
        server: &MockServer,
        owner: &str,
        repo: &str,
        response: ResponseTemplate,
    ) {
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}")))
            .respond_with(response)
            .mount(server)
            .await;
    }

    fn build_filter_with_mock(
        server: &MockServer,
        config: FilterConfig,
    ) -> GitHubMetadataFilter {
        let octocrab = octocrab::Octocrab::builder()
            .base_uri(server.uri())
            .expect("valid URI")
            .build()
            .expect("build octocrab");
        GitHubMetadataFilter::with_octocrab(config, octocrab)
    }

    #[tokio::test]
    async fn filters_by_min_stars() {
        let server = MockServer::start().await;

        setup_mock_github(
            &server,
            "owner",
            "low-stars",
            ResponseTemplate::new(200)
                .set_body_json(github_repo_json("owner", "low-stars", 5, Some("Rust"), false, false, &[])),
        )
        .await;

        let config = FilterConfig {
            min_stars: 10,
            languages: vec![],
            topics: vec![],
            exclude_forks: false,
            exclude_archived: false,
        };

        let filter = build_filter_with_mock(&server, config);
        let entries = vec![sample_entry("owner", "low-stars")];
        let candidates = filter.filter(entries).await.unwrap();

        assert!(candidates.is_empty(), "repo with 5 stars should be excluded when min_stars=10");
    }

    #[tokio::test]
    async fn filters_by_language() {
        let server = MockServer::start().await;

        setup_mock_github(
            &server,
            "owner",
            "python-repo",
            ResponseTemplate::new(200)
                .set_body_json(github_repo_json("owner", "python-repo", 100, Some("Python"), false, false, &[])),
        )
        .await;

        let config = FilterConfig {
            min_stars: 0,
            languages: vec!["Rust".into(), "TypeScript".into()],
            topics: vec![],
            exclude_forks: false,
            exclude_archived: false,
        };

        let filter = build_filter_with_mock(&server, config);
        let entries = vec![sample_entry("owner", "python-repo")];
        let candidates = filter.filter(entries).await.unwrap();

        assert!(candidates.is_empty(), "Python repo should be excluded when languages=[Rust, TypeScript]");
    }

    #[tokio::test]
    async fn excludes_forks_when_configured() {
        let server = MockServer::start().await;

        setup_mock_github(
            &server,
            "owner",
            "forked-repo",
            ResponseTemplate::new(200)
                .set_body_json(github_repo_json("owner", "forked-repo", 100, Some("Rust"), true, false, &[])),
        )
        .await;

        let config = FilterConfig {
            min_stars: 0,
            languages: vec![],
            topics: vec![],
            exclude_forks: true,
            exclude_archived: false,
        };

        let filter = build_filter_with_mock(&server, config);
        let entries = vec![sample_entry("owner", "forked-repo")];
        let candidates = filter.filter(entries).await.unwrap();

        assert!(candidates.is_empty(), "forked repo should be excluded when exclude_forks=true");
    }

    #[tokio::test]
    async fn excludes_archived_repos() {
        let server = MockServer::start().await;

        setup_mock_github(
            &server,
            "owner",
            "old-repo",
            ResponseTemplate::new(200)
                .set_body_json(github_repo_json("owner", "old-repo", 100, Some("Rust"), false, true, &[])),
        )
        .await;

        let config = FilterConfig {
            min_stars: 0,
            languages: vec![],
            topics: vec![],
            exclude_forks: false,
            exclude_archived: true,
        };

        let filter = build_filter_with_mock(&server, config);
        let entries = vec![sample_entry("owner", "old-repo")];
        let candidates = filter.filter(entries).await.unwrap();

        assert!(candidates.is_empty(), "archived repo should be excluded when exclude_archived=true");
    }

    #[tokio::test]
    async fn handles_404_gracefully() {
        let server = MockServer::start().await;

        setup_mock_github(
            &server,
            "owner",
            "deleted-repo",
            ResponseTemplate::new(404)
                .set_body_json(serde_json::json!({"message": "Not Found"})),
        )
        .await;

        let config = FilterConfig::default();
        let filter = build_filter_with_mock(&server, config);
        let entries = vec![sample_entry("owner", "deleted-repo")];
        let candidates = filter.filter(entries).await.unwrap();

        assert!(candidates.is_empty(), "404 repo should be skipped, not error");
    }

    #[tokio::test]
    async fn passes_through_when_all_filters_match() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/great-repo"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(github_repo_json(
                        "owner",
                        "great-repo",
                        500,
                        Some("Rust"),
                        false,
                        false,
                        &["cli", "tooling"],
                    )),
            )
            .expect(1)
            .mount(&server)
            .await;

        let config = FilterConfig {
            min_stars: 10,
            languages: vec!["Rust".into()],
            topics: vec!["cli".into()],
            exclude_forks: true,
            exclude_archived: true,
        };

        let filter = build_filter_with_mock(&server, config);
        let entries = vec![sample_entry("owner", "great-repo")];
        let candidates = filter.filter(entries).await.unwrap();

        assert_eq!(candidates.len(), 1);
        let c = &candidates[0];
        assert_eq!(c.owner, "owner");
        assert_eq!(c.repo_name, "great-repo");
        assert_eq!(c.stars, 500);
        assert_eq!(c.language.as_deref(), Some("Rust"));
        assert!(!c.fork);
        assert!(!c.archived);
        assert_eq!(c.topics, vec!["cli", "tooling"]);
    }

    #[test]
    fn parse_owner_repo_valid() {
        let url = Url::parse("https://github.com/rust-lang/rust").unwrap();
        let (owner, repo) = parse_owner_repo(&url).unwrap();
        assert_eq!(owner, "rust-lang");
        assert_eq!(repo, "rust");
    }

    #[test]
    fn parse_owner_repo_with_git_suffix() {
        let url = Url::parse("https://github.com/owner/repo.git").unwrap();
        let (owner, repo) = parse_owner_repo(&url).unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn parse_owner_repo_non_github() {
        let url = Url::parse("https://gitlab.com/owner/repo").unwrap();
        assert!(parse_owner_repo(&url).is_none());
    }
}
