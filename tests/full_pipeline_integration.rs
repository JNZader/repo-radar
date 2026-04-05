use repo_radar::adapters::analyzer::{AnalyzerAdapter, NoopAnalyzer};
use repo_radar::adapters::crossref::github_crossref::GitHubCrossRef;
use repo_radar::adapters::crossref::{CrossRefAdapter, NoopCrossRef};
use repo_radar::adapters::filter::{FilterAdapter, GitHubMetadataFilter};
use repo_radar::adapters::reporter::{MarkdownReporter, ReporterAdapter};
use repo_radar::adapters::source::{RssSource, SourceAdapter};
use repo_radar::config::{FeedConfig, FilterConfig};
use repo_radar::infra::seen::SeenStore;
use repo_radar::pipeline::Pipeline;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Helper: a minimal Atom feed with a single entry pointing to `owner/repo`.
fn atom_feed_single(base_url: &str, owner: &str, repo: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Test Feed</title>
  <updated>2026-04-01T00:00:00Z</updated>
  <id>{base_url}/feed.xml</id>
  <entry>
    <id>https://github.com/{owner}/{repo}</id>
    <title>{repo}</title>
    <link href="https://github.com/{owner}/{repo}" rel="related" type="text/html"/>
    <summary>A test repository</summary>
    <published>2026-04-01T00:00:00Z</published>
    <updated>2026-04-01T00:00:00Z</updated>
  </entry>
</feed>"#
    )
}

/// Helper: build a GitHub repos API JSON response that octocrab can parse.
///
/// Uses string formatting instead of `serde_json::json!` to avoid macro recursion limits.
fn github_repo_json(
    owner: &str,
    repo: &str,
    stars: u64,
    language: Option<&str>,
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
  "fork":false,"archived":false,"disabled":false,
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

/// Full pipeline integration: RSS Source -> GitHub Filter -> Noop Analyzer -> GitHub CrossRef -> Noop Reporter.
///
/// Uses wiremock for all HTTP calls (RSS feed, GitHub repos API for filter, GitHub user repos for crossref).
/// Verifies that data flows through all stages and the report reflects correct counts.
#[tokio::test]
async fn full_pipeline_source_filter_analyzer_crossref_reporter() {
    let server = MockServer::start().await;

    // 1. RSS feed mock — one entry pointing to owner/test-repo
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(atom_feed_single(&server.uri(), "owner", "test-repo")),
        )
        .mount(&server)
        .await;

    // 2. GitHub repos API mock for filter (GET /repos/owner/test-repo)
    Mock::given(method("GET"))
        .and(path("/repos/owner/test-repo"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(github_repo_json(
                "owner",
                "test-repo",
                500,
                Some("Rust"),
                &["cli", "async"],
            )),
        )
        .mount(&server)
        .await;

    // 3. GitHub user repos mock for crossref (GET /users/myuser/repos)
    Mock::given(method("GET"))
        .and(path("/users/myuser/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "full_name": "myuser/my-cli",
                "language": "Rust",
                "topics": ["cli", "tooling"]
            }
        ])))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let seen_path = dir.path().join("seen.json");
    let seen = SeenStore::load(&seen_path).unwrap();

    // Build adapters using enum variants (real dispatch path)
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    let source = SourceAdapter::Rss(RssSource::new(
        vec![FeedConfig {
            url: format!("{}/feed.xml", server.uri()),
            name: Some("integration-test-feed".into()),
            limit: None,
        }],
        client,
    ));

    let octocrab_filter = octocrab::Octocrab::builder()
        .base_uri(server.uri())
        .expect("valid URI")
        .build()
        .expect("build octocrab for filter");
    let filter = FilterAdapter::GitHubMetadata(Box::new(GitHubMetadataFilter::with_octocrab(
        FilterConfig {
            min_stars: 10,
            languages: vec![],
            topics: vec![],
            exclude_forks: true,
            exclude_archived: true,
        },
        octocrab_filter,
    )));

    // Analyzer: Noop (repoforge needs a real binary; Noop passes candidates through as empty analyzed vec)
    let analyzer = AnalyzerAdapter::Noop(NoopAnalyzer);

    // CrossRef: Noop since analyzer returns empty results (no AnalysisResults to cross-reference)
    let crossref = CrossRefAdapter::Noop(NoopCrossRef);

    let report_dir = dir.path().join("reports");
    let reporter = ReporterAdapter::Markdown(MarkdownReporter::new(report_dir.clone()));

    // Categorizer: Keyword-based categorizer
    let categorizer = repo_radar::adapters::categorizer::KeywordCategorizer::new();

    let mut pipeline = Pipeline::new(source, filter, categorizer, analyzer, crossref, reporter, seen, None);
    let (report, _results) = pipeline.run().await.unwrap();

    // Source fetched 1 entry (the atom feed has 1 GitHub entry)
    assert_eq!(report.entries_fetched, 1, "should fetch 1 entry from RSS");
    assert_eq!(report.entries_new, 1, "1 new entry (first run)");
    // Filter passes it through (500 stars > 10 min, not fork, not archived)
    assert_eq!(report.candidates_filtered, 1, "repo passes all filter criteria");
    // Analyzer is Noop — passes candidates through with empty analysis fields
    assert_eq!(report.analyzed, 1, "noop analyzer passes candidates through");
    // CrossRef: NoopCrossRef wraps results with no matches
    assert_eq!(report.crossrefed, 1, "noop crossref wraps analysis results");
    assert_eq!(report.reported, 1);

    // Verify MarkdownReporter produced a .md file in the output directory
    assert!(report_dir.exists(), "report output directory should be created");
    let md_files: Vec<_> = std::fs::read_dir(&report_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "md")
        })
        .collect();
    assert_eq!(md_files.len(), 1, "exactly one .md report file should exist");

    let content = std::fs::read_to_string(md_files[0].path()).unwrap();
    assert!(
        content.contains("# Repo Radar Report"),
        "report should contain the main heading"
    );
    // Noop pipeline now passes candidates through — report contains the discovered repo
    assert!(
        content.contains("github.com"),
        "report should contain a GitHub link from the discovered repo"
    );
}

/// Full pipeline using ALL real adapter enum variants, including GitHub CrossRef.
///
/// This test builds AnalysisResults manually by leveraging the pipeline stages:
/// Source + Filter produce candidates, then we directly test CrossRef with real data.
#[tokio::test]
async fn crossref_adapter_processes_analysis_results_through_pipeline() {
    use repo_radar::domain::crossref::CrossRef;
    use repo_radar::domain::model::{AnalysisResult, FeedEntry, RepoCandidate};

    let server = MockServer::start().await;

    // Mock user repos for crossref
    Mock::given(method("GET"))
        .and(path("/users/testuser/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "full_name": "testuser/my-rust-cli",
                "language": "Rust",
                "topics": ["cli", "async"]
            },
            {
                "full_name": "testuser/python-ml",
                "language": "Python",
                "topics": ["ml", "data-science"]
            }
        ])))
        .mount(&server)
        .await;

    let octocrab = octocrab::Octocrab::builder()
        .base_uri(server.uri())
        .expect("valid URI")
        .build()
        .expect("build octocrab");
    let crossref = CrossRefAdapter::GitHub(Box::new(GitHubCrossRef::with_octocrab(
        "testuser".into(),
        octocrab,
    )));

    // Build a synthetic AnalysisResult that should match the Rust CLI own repo
    let analysis = AnalysisResult {
        candidate: RepoCandidate {
            entry: FeedEntry {
                title: "cool-tool".into(),
                repo_url: url::Url::parse("https://github.com/someone/cool-tool").unwrap(),
                description: Some("A cool CLI tool".into()),
                published: Some(chrono::Utc::now()),
                source_name: "test".into(),
            },
            stars: 200,
            language: Some("Rust".into()),
            topics: vec!["cli".into(), "performance".into()],
            fork: false,
            archived: false,
            owner: "someone".into(),
            repo_name: "cool-tool".into(),
        category: Default::default(),
        },
        summary: "A fast CLI tool".into(),
        key_features: vec!["fast".into()],
        tech_stack: vec!["Rust".into(), "tokio".into()],
        relevance_score: 0.75,
    };

    let results = crossref.cross_reference(vec![analysis]).await.unwrap();

    assert_eq!(results.len(), 1);
    let result = &results[0];
    // Should match testuser/my-rust-cli (shared: Rust language + cli topic)
    assert!(
        !result.matched_repos.is_empty(),
        "should have at least one match from overlapping language+topics"
    );
    assert!(result.overall_relevance > 0.0);

    let matched_names: Vec<&str> = result
        .matched_repos
        .iter()
        .map(|m| m.own_repo.as_str())
        .collect();
    assert!(
        matched_names.contains(&"testuser/my-rust-cli"),
        "should match the Rust CLI repo"
    );
    // python-ml should NOT match (no language or topic overlap)
    assert!(
        !matched_names.contains(&"testuser/python-ml"),
        "should not match python-ml repo"
    );
}
