//! Cross-source semantic discovery using TF-IDF keyword scoring.
//!
//! Enables searching across repo-radar's data sources with a single query.
//! Ranks results by relevance using term frequency-inverse document frequency.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A document indexed for semantic search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchDocument {
    /// Display name (e.g. "owner/repo").
    pub name: String,
    /// URL to the resource.
    pub url: String,
    /// Source that produced this document (e.g. "github-trending", "hackernews").
    pub source: String,
    /// Combined text content for searching (description + topics + README excerpt).
    pub content: String,
}

/// A search result with relevance score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub document: SearchDocument,
    /// Relevance score (higher = more relevant). Not normalized to [0,1].
    pub score: f64,
    /// Matched terms that contributed to the score.
    pub matched_terms: Vec<String>,
}

/// Common English stop words filtered during tokenization.
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "is", "of", "for", "in", "to", "with", "and", "or", "be",
    "at", "by", "from", "this", "that", "on", "are", "as", "it", "its", "was",
    "were", "has", "have", "had", "not", "but", "if", "so", "can", "will", "do",
    "does", "did", "been", "being", "would", "could", "should", "may", "might",
    "shall", "must", "need", "use", "used", "using", "which", "what", "when",
    "where", "who", "how", "than", "then", "also", "just", "like", "into",
    "about", "up", "out", "no", "only", "very", "more", "most", "some", "any",
    "all", "each", "every", "both", "few", "many", "much", "own", "other",
    "such", "our", "your", "they", "them", "their", "we", "you", "he", "she",
];

/// In-memory semantic search index using TF-IDF scoring.
///
/// Documents are added via `add_document`, then queried with `search`.
/// The index recomputes IDF weights on each search call (suitable for
/// small-to-medium corpora typical of repo-radar scans).
pub struct SemanticIndex {
    documents: Vec<SearchDocument>,
    /// Per-document term frequency maps: doc_index -> (term -> count).
    tf_maps: Vec<HashMap<String, usize>>,
    /// Total number of tokens per document (for TF normalization).
    doc_lengths: Vec<usize>,
}

impl SemanticIndex {
    /// Create a new empty index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            documents: Vec::new(),
            tf_maps: Vec::new(),
            doc_lengths: Vec::new(),
        }
    }

    /// Add a document to the index.
    pub fn add_document(&mut self, doc: SearchDocument) {
        let tokens = tokenize(&doc.content);
        let total = tokens.len();

        let mut tf: HashMap<String, usize> = HashMap::new();
        for token in &tokens {
            *tf.entry(token.clone()).or_insert(0) += 1;
        }

        // Also index the name (split on / and -)
        let name_tokens = tokenize(&doc.name.replace(['/', '-', '_'], " "));
        for token in &name_tokens {
            *tf.entry(token.clone()).or_insert(0) += 1;
        }

        self.documents.push(doc);
        self.tf_maps.push(tf);
        self.doc_lengths.push(total + name_tokens.len());
    }

    /// Search the index with a natural language query.
    ///
    /// Returns up to `limit` results sorted by TF-IDF relevance score (descending).
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() || self.documents.is_empty() {
            return Vec::new();
        }

        // Compute IDF for each query term
        let n = self.documents.len() as f64;
        let mut idf: HashMap<&str, f64> = HashMap::new();
        for token in &query_tokens {
            let df = self
                .tf_maps
                .iter()
                .filter(|tf| tf.contains_key(token.as_str()))
                .count() as f64;
            // Smooth IDF: ln(1 + N / (1 + df))
            idf.insert(token, (1.0 + n / (1.0 + df)).ln());
        }

        // Score each document
        let mut scored: Vec<(usize, f64, Vec<String>)> = self
            .documents
            .iter()
            .enumerate()
            .filter_map(|(i, _doc)| {
                let tf = &self.tf_maps[i];
                let doc_len = self.doc_lengths[i] as f64;
                if doc_len == 0.0 {
                    return None;
                }

                let mut score = 0.0;
                let mut matched = Vec::new();

                for token in &query_tokens {
                    if let Some(&count) = tf.get(token.as_str()) {
                        // Normalized TF: count / doc_length
                        let tf_score = count as f64 / doc_len;
                        let idf_score = idf.get(token.as_str()).copied().unwrap_or(0.0);
                        score += tf_score * idf_score;
                        if !matched.contains(token) {
                            matched.push(token.clone());
                        }
                    }
                }

                if score > 0.0 {
                    Some((i, score, matched))
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        scored
            .into_iter()
            .map(|(i, score, matched_terms)| SearchResult {
                document: self.documents[i].clone(),
                score,
                matched_terms,
            })
            .collect()
    }

    /// Return the number of indexed documents.
    #[must_use]
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    /// Return `true` if the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}

impl Default for SemanticIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Tokenize text into lowercase alphanumeric tokens, filtering stop words.
fn tokenize(text: &str) -> Vec<String> {
    // Strip HTML tags first
    let clean = strip_html(text);
    clean
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| t.len() > 1 && !STOP_WORDS.contains(&t.as_str()))
        .collect()
}

/// Strip HTML tags from text.
fn strip_html(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                result.push(' ');
            }
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn make_doc(name: &str, source: &str, content: &str) -> SearchDocument {
        SearchDocument {
            name: name.into(),
            url: format!("https://github.com/{name}"),
            source: source.into(),
            content: content.into(),
        }
    }

    #[test]
    fn empty_index_returns_empty() {
        let index = SemanticIndex::new();
        let results = index.search("anything", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn empty_query_returns_empty() {
        let mut index = SemanticIndex::new();
        index.add_document(make_doc("owner/repo", "test", "some content here"));
        let results = index.search("", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn stop_words_only_query_returns_empty() {
        let mut index = SemanticIndex::new();
        index.add_document(make_doc("owner/repo", "test", "efficient graph traversal algorithm"));
        let results = index.search("the a an is", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn exact_match_scores_higher_than_partial() {
        let mut index = SemanticIndex::new();
        index.add_document(make_doc(
            "owner/graph-algo",
            "github-trending",
            "efficient graph traversal algorithm with Dijkstra implementation",
        ));
        index.add_document(make_doc(
            "owner/web-app",
            "hackernews",
            "web application framework with efficient routing",
        ));
        index.add_document(make_doc(
            "owner/unrelated",
            "reddit",
            "machine learning model training pipeline",
        ));

        let results = index.search("efficient graph traversal", 10);

        assert!(!results.is_empty());
        assert_eq!(results[0].document.name, "owner/graph-algo");
        // The graph-algo doc should score higher since it matches more terms
        if results.len() > 1 {
            assert!(results[0].score > results[1].score);
        }
    }

    #[test]
    fn cross_source_results_returned() {
        let mut index = SemanticIndex::new();
        index.add_document(make_doc(
            "owner/rust-graph",
            "github-trending",
            "graph algorithms in Rust with BFS and DFS",
        ));
        index.add_document(make_doc(
            "owner/py-graph",
            "hackernews",
            "Python graph library with pathfinding algorithms",
        ));
        index.add_document(make_doc(
            "owner/js-graph",
            "reddit",
            "JavaScript graph visualization and traversal",
        ));

        let results = index.search("graph algorithms", 10);

        // All three should match since they all contain "graph"
        assert!(results.len() >= 2);

        // Verify results come from different sources
        let sources: HashSet<&str> = results.iter().map(|r| r.document.source.as_str()).collect();
        assert!(sources.len() >= 2, "results should span multiple sources");
    }

    #[test]
    fn limit_is_respected() {
        let mut index = SemanticIndex::new();
        for i in 0..10 {
            index.add_document(make_doc(
                &format!("owner/repo-{i}"),
                "test",
                &format!("rust tool number {i} for developers"),
            ));
        }

        let results = index.search("rust tool", 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn matched_terms_are_populated() {
        let mut index = SemanticIndex::new();
        index.add_document(make_doc(
            "owner/repo",
            "test",
            "graph traversal with BFS and DFS algorithms",
        ));

        let results = index.search("graph traversal", 10);
        assert_eq!(results.len(), 1);
        assert!(results[0].matched_terms.contains(&"graph".to_string()));
        assert!(results[0].matched_terms.contains(&"traversal".to_string()));
    }

    #[test]
    fn html_content_is_stripped() {
        let mut index = SemanticIndex::new();
        index.add_document(make_doc(
            "owner/repo",
            "test",
            "<h1>Graph Algorithms</h1><p>Efficient <b>graph traversal</b> in Rust</p>",
        ));

        let results = index.search("graph traversal rust", 10);
        assert_eq!(results.len(), 1);
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn name_tokens_contribute_to_search() {
        let mut index = SemanticIndex::new();
        index.add_document(make_doc(
            "tokio-rs/tokio",
            "github-trending",
            "An async runtime",
        ));

        // Search for "tokio" — should match via the name
        let results = index.search("tokio", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].document.name, "tokio-rs/tokio");
    }

    #[test]
    fn idf_weights_rare_terms_higher() {
        let mut index = SemanticIndex::new();
        // "rust" appears in all docs (low IDF), "quantum" appears in one (high IDF)
        index.add_document(make_doc("owner/quantum", "test", "quantum computing in rust"));
        index.add_document(make_doc("owner/web", "test", "web framework in rust"));
        index.add_document(make_doc("owner/cli", "test", "command line tool in rust"));

        let results = index.search("quantum rust", 10);
        assert!(!results.is_empty());
        // The quantum repo should be ranked first because "quantum" has high IDF
        assert_eq!(results[0].document.name, "owner/quantum");
    }

    #[test]
    fn no_match_returns_empty() {
        let mut index = SemanticIndex::new();
        index.add_document(make_doc("owner/repo", "test", "web framework for JavaScript"));
        let results = index.search("quantum computing", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn search_result_serde_round_trip() {
        let result = SearchResult {
            document: make_doc("owner/repo", "test", "content"),
            score: 0.75,
            matched_terms: vec!["graph".into(), "traversal".into()],
        };
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: SearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, deserialized);
    }

    #[test]
    fn index_len_and_is_empty() {
        let mut index = SemanticIndex::new();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);

        index.add_document(make_doc("owner/repo", "test", "content"));
        assert!(!index.is_empty());
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn tokenize_filters_short_tokens() {
        let tokens = tokenize("I a am go to x");
        // "I" (len 1), "a" (stop), "am" (len 2 but not stop — kept),
        // "go" (2 chars, kept), "to" (stop), "x" (len 1)
        assert!(!tokens.contains(&"a".to_string()));
        assert!(!tokens.contains(&"x".to_string()));
        assert!(!tokens.contains(&"to".to_string()));
        assert!(tokens.contains(&"go".to_string()));
    }

    #[test]
    fn duplicate_query_terms_handled() {
        let mut index = SemanticIndex::new();
        index.add_document(make_doc("owner/repo", "test", "graph algorithms graph theory"));

        let results = index.search("graph graph graph", 10);
        assert_eq!(results.len(), 1);
        // "graph" should appear once in matched_terms (deduped)
        assert_eq!(
            results[0]
                .matched_terms
                .iter()
                .filter(|t| *t == "graph")
                .count(),
            1
        );
    }
}
