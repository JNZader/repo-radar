use crate::domain::categorizer::Categorizer;
use crate::domain::model::{RepoCandidate, RepoCategory};
use crate::infra::error::CategorizerError;

/// Keyword-based categorizer that assigns a category to each repo based on
/// its topics, description, repo name, and language.
#[derive(Debug, Clone)]
pub struct KeywordCategorizer;

impl KeywordCategorizer {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for KeywordCategorizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Mapping from keywords to categories, checked in priority order.
const CATEGORY_RULES: &[(&[&str], RepoCategory)] = &[
    (
        &[
            "agent", "ai-agent", "llm-agent", "autonomous", "multi-agent",
            "langchain", "langgraph", "autogen", "crewai", "mcp", "tool-use",
            "function-calling", "agentic",
        ],
        RepoCategory::AiAgents,
    ),
    (
        &[
            "rag", "retrieval", "search", "vector", "embedding", "semantic-search",
            "reranking", "indexing", "faiss", "pinecone", "qdrant", "weaviate",
            "chromadb", "milvus",
        ],
        RepoCategory::RagSearch,
    ),
    (
        &[
            "memory", "context-window", "long-term-memory", "conversation-memory",
            "mem0", "memgpt", "knowledge-graph",
        ],
        RepoCategory::Memory,
    ),
    (
        &[
            "security", "vulnerability", "cve", "sast", "dast", "pentest",
            "auth", "authentication", "authorization", "encryption", "crypto",
            "firewall", "audit", "compliance", "owasp",
        ],
        RepoCategory::Security,
    ),
    (
        &[
            "devops", "cicd", "ci-cd", "docker", "kubernetes", "k8s", "terraform",
            "ansible", "helm", "infrastructure", "iac", "monitoring", "observability",
            "prometheus", "grafana", "deploy", "deployment", "pipeline",
            "github-actions", "gitlab-ci",
        ],
        RepoCategory::DevOps,
    ),
    (
        &[
            "documentation", "docs", "readme", "wiki", "docusaurus", "mdx",
            "mdbook", "rustdoc", "jsdoc", "typedoc", "openapi", "swagger",
            "api-docs", "changelog",
        ],
        RepoCategory::Documentation,
    ),
    (
        &[
            "testing", "test", "e2e", "unit-test", "integration-test", "playwright",
            "cypress", "jest", "pytest", "vitest", "coverage", "tdd", "bdd",
            "snapshot", "fuzzing", "property-testing",
        ],
        RepoCategory::Testing,
    ),
    (
        &[
            "ui", "ux", "frontend", "component", "design-system", "storybook",
            "tailwind", "css", "animation", "responsive", "accessibility", "a11y",
            "react", "vue", "svelte", "angular", "nextjs",
        ],
        RepoCategory::UiUx,
    ),
    (
        &[
            "workflow", "automation", "orchestration", "scheduler", "cron",
            "task-runner", "make", "just", "turborepo", "nx", "monorepo",
            "build-system", "cli", "terminal",
        ],
        RepoCategory::Workflow,
    ),
];

/// Categorize a single candidate based on its metadata.
fn categorize_candidate(candidate: &RepoCandidate) -> RepoCategory {
    // Collect all searchable text: topics + description + repo name
    let topics_lower: Vec<String> = candidate.topics.iter().map(|t| t.to_lowercase()).collect();

    let description_lower = candidate
        .entry
        .description
        .as_deref()
        .unwrap_or("")
        .to_lowercase();

    let name_lower = candidate.repo_name.to_lowercase();

    for (keywords, category) in CATEGORY_RULES {
        for keyword in *keywords {
            let kw = *keyword;

            // Check topics (exact match)
            if topics_lower.iter().any(|t| t == kw || t.contains(kw)) {
                return category.clone();
            }

            // Check repo name (contains)
            if name_lower.contains(kw) {
                return category.clone();
            }

            // Check description (word boundary-ish: contains the keyword)
            if description_lower.contains(kw) {
                return category.clone();
            }
        }
    }

    RepoCategory::Other
}

impl Categorizer for KeywordCategorizer {
    fn categorize(
        &self,
        candidates: Vec<RepoCandidate>,
    ) -> Result<Vec<RepoCandidate>, CategorizerError> {
        let categorized = candidates
            .into_iter()
            .map(|mut c| {
                c.category = categorize_candidate(&c);
                c
            })
            .collect();
        Ok(categorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::FeedEntry;
    use chrono::Utc;
    use url::Url;

    fn make_candidate(
        name: &str,
        description: Option<&str>,
        topics: &[&str],
    ) -> RepoCandidate {
        RepoCandidate {
            entry: FeedEntry {
                title: name.into(),
                repo_url: Url::parse(&format!("https://github.com/owner/{name}")).unwrap(),
                description: description.map(String::from),
                published: Some(Utc::now()),
                source_name: "test".into(),
            },
            stars: 100,
            language: Some("Rust".into()),
            topics: topics.iter().map(|s| (*s).to_string()).collect(),
            fork: false,
            archived: false,
            owner: "owner".into(),
            repo_name: name.into(),
            category: RepoCategory::default(),
            semantic_score: 0.0,
            pushed_at: None,
        }
    }

    #[test]
    fn categorizes_ai_agent_by_topic() {
        let c = make_candidate("my-tool", None, &["agent", "llm"]);
        assert_eq!(categorize_candidate(&c), RepoCategory::AiAgents);
    }

    #[test]
    fn categorizes_ai_agent_by_description() {
        let c = make_candidate("cool-lib", Some("An autonomous AI agent framework"), &[]);
        assert_eq!(categorize_candidate(&c), RepoCategory::AiAgents);
    }

    #[test]
    fn categorizes_security_by_topic() {
        let c = make_candidate("scanner", None, &["vulnerability", "sast"]);
        assert_eq!(categorize_candidate(&c), RepoCategory::Security);
    }

    #[test]
    fn categorizes_devops_by_name() {
        let c = make_candidate("kubernetes-operator", None, &[]);
        assert_eq!(categorize_candidate(&c), RepoCategory::DevOps);
    }

    #[test]
    fn categorizes_rag_by_topic() {
        let c = make_candidate("my-lib", None, &["rag", "retrieval"]);
        assert_eq!(categorize_candidate(&c), RepoCategory::RagSearch);
    }

    #[test]
    fn categorizes_testing_by_topic() {
        let c = make_candidate("test-runner", None, &["testing", "e2e"]);
        assert_eq!(categorize_candidate(&c), RepoCategory::Testing);
    }

    #[test]
    fn categorizes_ui_by_topic() {
        let c = make_candidate("component-lib", None, &["design-system", "storybook"]);
        assert_eq!(categorize_candidate(&c), RepoCategory::UiUx);
    }

    #[test]
    fn categorizes_workflow_by_topic() {
        let c = make_candidate("task-tool", None, &["automation", "cli"]);
        assert_eq!(categorize_candidate(&c), RepoCategory::Workflow);
    }

    #[test]
    fn categorizes_memory_by_description() {
        let c = make_candidate("brain", Some("Long-term memory for LLM conversations"), &[]);
        assert_eq!(categorize_candidate(&c), RepoCategory::Memory);
    }

    #[test]
    fn categorizes_documentation_by_topic() {
        let c = make_candidate("doc-gen", None, &["documentation"]);
        assert_eq!(categorize_candidate(&c), RepoCategory::Documentation);
    }

    #[test]
    fn defaults_to_other_when_no_match() {
        let c = make_candidate("random-thing", Some("A random library"), &["misc"]);
        assert_eq!(categorize_candidate(&c), RepoCategory::Other);
    }

    #[test]
    fn batch_categorize_works() {
        let categorizer = KeywordCategorizer::new();
        let candidates = vec![
            make_candidate("agent-kit", None, &["agent"]),
            make_candidate("random", None, &[]),
        ];
        let result = categorizer.categorize(candidates).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].category, RepoCategory::AiAgents);
        assert_eq!(result[1].category, RepoCategory::Other);
    }

    #[test]
    fn priority_order_ai_agents_over_workflow() {
        // "agent" matches AI Agents, "cli" matches Workflow — AI Agents wins (higher priority)
        let c = make_candidate("agent-cli", None, &["agent", "cli"]);
        assert_eq!(categorize_candidate(&c), RepoCategory::AiAgents);
    }
}
