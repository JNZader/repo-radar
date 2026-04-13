#![allow(clippy::manual_async_fn)] // Trait uses RPITIT pattern, impls must match

use std::future::Future;

use serde::Deserialize;
use tracing::{debug, info, warn};
use url::Url;

use crate::domain::model::FeedEntry;
use crate::domain::source::Source;
use crate::infra::error::SourceError;

const GITHUB_API_BASE: &str = "https://api.github.com";

/// Fetches trending AI agent skills from GitHub by searching for repos
/// containing SKILL.md files (following the Agent Skills spec).
///
/// Uses the GitHub Code Search API to discover repos with SKILL.md,
/// then enriches them with star count and metadata.
pub struct GitHubSkillsSource {
    client: reqwest::Client,
    token: Option<String>,
    api_base: String,
    /// Maximum number of skill repos to return (default: 30).
    limit: usize,
}

impl GitHubSkillsSource {
    #[must_use]
    pub fn new(client: reqwest::Client, token: Option<String>, limit: usize) -> Self {
        Self {
            client,
            token,
            api_base: GITHUB_API_BASE.to_string(),
            limit,
        }
    }

    /// Create with a custom API base URL (for tests with wiremock).
    #[cfg(test)]
    pub fn new_with_base(
        client: reqwest::Client,
        token: Option<String>,
        limit: usize,
        api_base: String,
    ) -> Self {
        Self {
            client,
            token,
            api_base,
            limit,
        }
    }

}

/// GitHub Code Search API response.
#[derive(Debug, Deserialize)]
struct CodeSearchResponse {
    #[serde(default)]
    items: Vec<CodeSearchItem>,
}

#[derive(Debug, Deserialize)]
struct CodeSearchItem {
    repository: CodeSearchRepo,
}

#[derive(Debug, Deserialize)]
struct CodeSearchRepo {
    full_name: String,
    html_url: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    updated_at: Option<String>,
}

impl Source for GitHubSkillsSource {
    fn fetch(&self) -> impl Future<Output = Result<Vec<FeedEntry>, SourceError>> + Send {
        let client = self.client.clone();
        let token = self.token.clone();
        let api_base = self.api_base.clone();
        let limit = self.limit;
        async move { fetch_skill_repos(&client, token.as_deref(), &api_base, limit).await }
    }

    fn name(&self) -> &'static str {
        "github-skills"
    }
}

async fn fetch_skill_repos(
    client: &reqwest::Client,
    token: Option<&str>,
    api_base: &str,
    limit: usize,
) -> Result<Vec<FeedEntry>, SourceError> {
    info!("searching GitHub for repos with SKILL.md files");

    // Search for repos containing SKILL.md using GitHub Code Search API.
    // We search for the filename pattern and sort by recently indexed.
    let url = format!(
        "{api_base}/search/code?q=filename:SKILL.md+path:/&sort=indexed&order=desc&per_page={}",
        limit.min(100) // GitHub API max per_page is 100
    );

    let mut req = client
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "repo-radar/0.1");
    if let Some(token) = token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }

    let response = req.send().await.map_err(|e| SourceError::FetchFailed {
        url: url.clone(),
        reason: e.to_string(),
    })?;

    let status = response.status();
    if status.as_u16() == 403 || status.as_u16() == 429 {
        return Err(SourceError::FetchFailed {
            url,
            reason: "GitHub API rate limited — set github_token in config".to_string(),
        });
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(SourceError::FetchFailed {
            url,
            reason: format!("GitHub API returned {status}: {body}"),
        });
    }

    let search_result: CodeSearchResponse =
        response.json().await.map_err(|e| SourceError::ParseFailed(e.to_string()))?;

    let mut entries = Vec::new();
    let mut seen_repos = std::collections::HashSet::new();

    for item in search_result.items {
        let repo = &item.repository;

        // Deduplicate — a repo may have multiple SKILL.md files
        if !seen_repos.insert(repo.full_name.clone()) {
            continue;
        }

        let repo_url = match Url::parse(&repo.html_url) {
            Ok(u) => u,
            Err(_) => {
                warn!(repo = %repo.full_name, "invalid URL, skipping");
                continue;
            }
        };

        let description = repo
            .description
            .as_ref()
            .map(|d| format!("[skill] {} ({}★)", d, repo.stargazers_count))
            .or_else(|| Some(format!("[skill] Agent skill repo ({}★)", repo.stargazers_count)));

        let published = repo
            .updated_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        entries.push(FeedEntry {
            title: repo.full_name.clone(),
            repo_url,
            description,
            published,
            source_name: "github-skills".to_string(),
        });

        debug!(
            repo = %repo.full_name,
            stars = repo.stargazers_count,
            "found skill repo"
        );
    }

    info!(count = entries.len(), "skill repos discovered");
    Ok(entries)
}

/// Categorize a skill repo by domain based on its name, description, and topics.
///
/// Returns a human-readable category string for the skill.
pub fn categorize_skill(name: &str, description: Option<&str>) -> &'static str {
    let text = format!(
        "{} {}",
        name.to_ascii_lowercase(),
        description.unwrap_or("").to_ascii_lowercase()
    );

    if text.contains("test") || text.contains("pytest") || text.contains("playwright") {
        "Testing"
    } else if text.contains("security") || text.contains("audit") || text.contains("guard") {
        "Security"
    } else if text.contains("devops")
        || text.contains("docker")
        || text.contains("ci/cd")
        || text.contains("deploy")
        || text.contains("k8s")
        || text.contains("kubernetes")
    {
        "DevOps"
    } else if text.contains("rag") || text.contains("embed") || text.contains("vector") {
        "RAG/Search"
    } else if text.contains("memory") || text.contains("context") || text.contains("session") {
        "Memory"
    } else if text.contains("react")
        || text.contains("nextjs")
        || text.contains("tailwind")
        || text.contains("ui")
        || text.contains("frontend")
    {
        "UI/UX"
    } else if text.contains("doc") || text.contains("readme") || text.contains("changelog") {
        "Documentation"
    } else if text.contains("workflow") || text.contains("automat") || text.contains("pipeline") {
        "Workflow"
    } else if text.contains("agent") || text.contains("llm") || text.contains("prompt") {
        "AI Agents"
    } else {
        "Other"
    }
}

/// Represents a trending skill with metadata for ranking.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrendingSkill {
    pub name: String,
    pub url: String,
    pub description: Option<String>,
    pub stars: u64,
    pub category: String,
    pub last_updated: Option<String>,
}

/// Parse a list of FeedEntry (from github-skills source) into ranked TrendingSkills.
///
/// Extracts star counts from the description prefix `[skill] ... (N★)` and sorts
/// by stars descending.
pub fn rank_trending_skills(entries: &[FeedEntry]) -> Vec<TrendingSkill> {
    let mut skills: Vec<TrendingSkill> = entries
        .iter()
        .filter(|e| e.source_name == "github-skills")
        .map(|e| {
            let (clean_desc, stars) = parse_skill_description(e.description.as_deref());
            let category = categorize_skill(&e.title, clean_desc.as_deref()).to_string();
            TrendingSkill {
                name: e.title.clone(),
                url: e.repo_url.to_string(),
                description: clean_desc,
                stars,
                category,
                last_updated: e.published.map(|dt| dt.to_rfc3339()),
            }
        })
        .collect();

    // Sort by stars descending (higher stars = more trending)
    skills.sort_by(|a, b| b.stars.cmp(&a.stars));
    skills
}

/// Parse the enriched description format: `[skill] actual description (N★)`
/// Returns (clean_description, star_count).
fn parse_skill_description(desc: Option<&str>) -> (Option<String>, u64) {
    let Some(desc) = desc else {
        return (None, 0);
    };

    // Strip the [skill] prefix
    let text = desc
        .strip_prefix("[skill] ")
        .unwrap_or(desc);

    // Extract stars from trailing `(N★)` pattern
    if let Some(star_start) = text.rfind('(') {
        let after = &text[star_start + 1..];
        if let Some(star_end) = after.find('★') {
            let star_str = &after[..star_end];
            if let Ok(stars) = star_str.parse::<u64>() {
                let clean = text[..star_start].trim().to_string();
                let clean = if clean.is_empty() {
                    None
                } else {
                    Some(clean)
                };
                return (clean, stars);
            }
        }
    }

    (Some(text.to_string()), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn code_search_response_json() -> serde_json::Value {
        serde_json::json!({
            "total_count": 3,
            "items": [
                {
                    "name": "SKILL.md",
                    "path": "SKILL.md",
                    "repository": {
                        "full_name": "author/react-skill",
                        "html_url": "https://github.com/author/react-skill",
                        "description": "React 19 patterns for AI agents",
                        "stargazers_count": 150,
                        "updated_at": "2026-04-01T10:00:00Z"
                    }
                },
                {
                    "name": "SKILL.md",
                    "path": "skills/testing/SKILL.md",
                    "repository": {
                        "full_name": "author/test-skill",
                        "html_url": "https://github.com/author/test-skill",
                        "description": "Playwright testing skill",
                        "stargazers_count": 75,
                        "updated_at": "2026-03-20T08:00:00Z"
                    }
                },
                {
                    "name": "SKILL.md",
                    "path": "SKILL.md",
                    "repository": {
                        "full_name": "author/react-skill",
                        "html_url": "https://github.com/author/react-skill",
                        "description": "React 19 patterns for AI agents",
                        "stargazers_count": 150,
                        "updated_at": "2026-04-01T10:00:00Z"
                    }
                }
            ]
        })
    }

    #[tokio::test]
    async fn fetches_skill_repos_from_github() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(code_search_response_json()),
            )
            .mount(&server)
            .await;

        let source = GitHubSkillsSource::new_with_base(
            reqwest::Client::new(),
            None,
            30,
            server.uri(),
        );

        let entries = source.fetch().await.unwrap();

        // 3 items but one is a duplicate (author/react-skill appears twice)
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "author/react-skill");
        assert_eq!(entries[1].title, "author/test-skill");
        assert_eq!(entries[0].source_name, "github-skills");
    }

    #[tokio::test]
    async fn handles_rate_limit_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(ResponseTemplate::new(403).set_body_string("rate limited"))
            .mount(&server)
            .await;

        let source = GitHubSkillsSource::new_with_base(
            reqwest::Client::new(),
            None,
            30,
            server.uri(),
        );

        let result = source.fetch().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("rate limited"));
    }

    #[tokio::test]
    async fn handles_empty_response() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"total_count": 0, "items": []})),
            )
            .mount(&server)
            .await;

        let source = GitHubSkillsSource::new_with_base(
            reqwest::Client::new(),
            None,
            30,
            server.uri(),
        );

        let entries = source.fetch().await.unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn name_returns_github_skills() {
        let source = GitHubSkillsSource::new(reqwest::Client::new(), None, 30);
        assert_eq!(source.name(), "github-skills");
    }

    #[test]
    fn categorize_skill_detects_testing() {
        assert_eq!(categorize_skill("playwright-testing", None), "Testing");
        assert_eq!(categorize_skill("skill", Some("pytest helpers")), "Testing");
    }

    #[test]
    fn categorize_skill_detects_security() {
        assert_eq!(categorize_skill("security-audit", None), "Security");
        assert_eq!(categorize_skill("skillguard", None), "Security");
    }

    #[test]
    fn categorize_skill_detects_ai_agents() {
        assert_eq!(categorize_skill("agent-testing", Some("AI agent tools")), "Testing");
        assert_eq!(categorize_skill("llm-skill", None), "AI Agents");
        assert_eq!(categorize_skill("prompt-eng", Some("prompt engineering")), "AI Agents");
    }

    #[test]
    fn categorize_skill_returns_other_for_unknown() {
        assert_eq!(categorize_skill("misc-tool", Some("does stuff")), "Other");
    }

    #[test]
    fn parse_skill_description_extracts_stars() {
        let (desc, stars) = parse_skill_description(Some("[skill] A great tool (42★)"));
        assert_eq!(desc.as_deref(), Some("A great tool"));
        assert_eq!(stars, 42);
    }

    #[test]
    fn parse_skill_description_handles_no_skill_prefix() {
        let (desc, stars) = parse_skill_description(Some("plain description (10★)"));
        assert_eq!(desc.as_deref(), Some("plain description"));
        assert_eq!(stars, 10);
    }

    #[test]
    fn parse_skill_description_handles_no_stars() {
        let (desc, stars) = parse_skill_description(Some("[skill] Just a tool"));
        assert_eq!(desc.as_deref(), Some("Just a tool"));
        assert_eq!(stars, 0);
    }

    #[test]
    fn parse_skill_description_handles_none() {
        let (desc, stars) = parse_skill_description(None);
        assert!(desc.is_none());
        assert_eq!(stars, 0);
    }

    #[test]
    fn rank_trending_skills_sorts_by_stars() {
        let entries = vec![
            FeedEntry {
                title: "author/low-stars".into(),
                repo_url: Url::parse("https://github.com/author/low-stars").unwrap(),
                description: Some("[skill] Low stars tool (5★)".into()),
                published: None,
                source_name: "github-skills".into(),
            },
            FeedEntry {
                title: "author/high-stars".into(),
                repo_url: Url::parse("https://github.com/author/high-stars").unwrap(),
                description: Some("[skill] High stars tool (500★)".into()),
                published: None,
                source_name: "github-skills".into(),
            },
            FeedEntry {
                title: "author/mid-stars".into(),
                repo_url: Url::parse("https://github.com/author/mid-stars").unwrap(),
                description: Some("[skill] Mid stars tool (50★)".into()),
                published: None,
                source_name: "github-skills".into(),
            },
        ];

        let ranked = rank_trending_skills(&entries);
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].name, "author/high-stars");
        assert_eq!(ranked[0].stars, 500);
        assert_eq!(ranked[1].name, "author/mid-stars");
        assert_eq!(ranked[1].stars, 50);
        assert_eq!(ranked[2].name, "author/low-stars");
        assert_eq!(ranked[2].stars, 5);
    }

    #[test]
    fn rank_trending_skills_ignores_non_skill_entries() {
        let entries = vec![
            FeedEntry {
                title: "author/normal-repo".into(),
                repo_url: Url::parse("https://github.com/author/normal-repo").unwrap(),
                description: Some("Not a skill".into()),
                published: None,
                source_name: "github-trending".into(), // Different source
            },
            FeedEntry {
                title: "author/skill-repo".into(),
                repo_url: Url::parse("https://github.com/author/skill-repo").unwrap(),
                description: Some("[skill] A skill (10★)".into()),
                published: None,
                source_name: "github-skills".into(),
            },
        ];

        let ranked = rank_trending_skills(&entries);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].name, "author/skill-repo");
    }

    #[test]
    fn trending_skill_serde_round_trip() {
        let skill = TrendingSkill {
            name: "author/skill".into(),
            url: "https://github.com/author/skill".into(),
            description: Some("A great skill".into()),
            stars: 42,
            category: "Testing".into(),
            last_updated: Some("2026-04-01T00:00:00+00:00".into()),
        };
        let json = serde_json::to_string(&skill).unwrap();
        let deserialized: TrendingSkill = serde_json::from_str(&json).unwrap();
        assert_eq!(skill, deserialized);
    }
}
