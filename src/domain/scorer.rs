use std::collections::HashSet;

use crate::domain::model::OwnRepoSummary;

/// Common English stop words to filter from keyword overlap.
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "is", "of", "for", "in", "to", "with", "and", "or",
    "be", "at", "by", "from", "this", "that", "on", "are", "as", "it",
    "its", "was", "were", "has", "have", "had", "not", "but", "if", "so",
];

/// Strip HTML tags from text, returning plain text content.
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

/// Tokenize text into lowercase words, filtering stop words and single-char tokens.
fn tokenize(text: &str) -> HashSet<String> {
    let clean = strip_html(text);
    clean.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| t.len() > 1 && !STOP_WORDS.contains(&t.as_str()))
        .collect()
}

/// Jaccard similarity between two slices of strings (case-insensitive).
fn jaccard_topics(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }

    let set_a: HashSet<String> = a.iter().map(|s| s.to_ascii_lowercase()).collect();
    let set_b: HashSet<String> = b.iter().map(|s| s.to_ascii_lowercase()).collect();

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Keyword overlap between two optional descriptions.
/// Score = |common words| / max(|words_a|, |words_b|)
fn keyword_overlap(desc_a: Option<&str>, desc_b: Option<&str>) -> f64 {
    let words_a = match desc_a {
        Some(t) if !t.trim().is_empty() => tokenize(t),
        _ => return 0.0,
    };
    let words_b = match desc_b {
        Some(t) if !t.trim().is_empty() => tokenize(t),
        _ => return 0.0,
    };

    let max_len = words_a.len().max(words_b.len());
    if max_len == 0 {
        return 0.0;
    }

    let common = words_a.intersection(&words_b).count();
    common as f64 / max_len as f64
}

/// Compute semantic similarity score between a discovered repo and a single own repo.
///
/// Uses adaptive weights depending on whether the own repo has topics:
/// - With topics:    0.50 * jaccard_topics + 0.35 * desc_overlap + 0.15 * name_overlap
/// - Without topics: 0.00 * jaccard_topics + 0.70 * desc_overlap + 0.30 * name_overlap
///
/// Also checks for partial topic containment (e.g. "multi-agent" ⊇ "agent").
fn score_against_own(
    description: Option<&str>,
    topics: &[String],
    own: &OwnRepoSummary,
) -> f64 {
    let jt = if !topics.is_empty() && !own.topics.is_empty() {
        // Full Jaccard + partial containment bonus
        let base = jaccard_topics(topics, &own.topics);
        let partial = partial_topic_overlap(topics, &own.topics);
        (base + partial * 0.3).min(1.0)
    } else {
        0.0
    };

    // Compare discovered description vs own description
    let ko = keyword_overlap(description, own.description.as_deref());

    // Compare discovered description vs own repo name (tokenized: "mcp-llm-bridge" → tokens)
    let own_name_as_text = own.name.replace(['-', '_'], " ");
    let name_score = keyword_overlap(description, Some(&own_name_as_text));

    let (w_topics, w_desc, w_name) = if !own.topics.is_empty() {
        (0.50, 0.35, 0.15)
    } else {
        (0.00, 0.70, 0.30)
    };

    (w_topics * jt + w_desc * ko + w_name * name_score).clamp(0.0, 1.0)
}

/// Partial topic overlap: counts topics that are substrings of each other.
/// E.g. "multi-agent" ⊇ "agent" → counts as 0.5 match per pair found.
/// Returns a value in [0.0, 1.0].
fn partial_topic_overlap(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut matches = 0usize;
    for ta in a {
        let ta_lower = ta.to_ascii_lowercase();
        for tb in b {
            let tb_lower = tb.to_ascii_lowercase();
            if ta_lower != tb_lower && (ta_lower.contains(&*tb_lower) || tb_lower.contains(&*ta_lower)) {
                matches += 1;
            }
        }
    }
    let max_possible = a.len().max(b.len());
    (matches as f64 / max_possible as f64).min(1.0)
}

/// Compute the semantic relevance of a discovered repo against the user's own repos.
///
/// Returns the maximum score across all own repos (0.0 if `own_repos` is empty).
/// Score is between 0.0 and 1.0.
pub fn semantic_score(
    description: Option<&str>,
    topics: &[String],
    own_repos: &[OwnRepoSummary],
) -> f64 {
    own_repos
        .iter()
        .map(|own| score_against_own(description, topics, own))
        .fold(0.0_f64, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own(name: &str, desc: Option<&str>, topics: &[&str]) -> OwnRepoSummary {
        OwnRepoSummary {
            name: name.into(),
            description: desc.map(String::from),
            topics: topics.iter().map(|t| (*t).to_string()).collect(),
        }
    }

    #[test]
    fn no_own_repos_returns_zero() {
        let score = semantic_score(Some("a great cli tool"), &["cli".into()], &[]);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn perfect_topic_match_dominates() {
        let own_repos = vec![own("my-tool", None, &["cli", "rust", "async"])];
        let score = semantic_score(None, &["cli".into(), "rust".into(), "async".into()], &own_repos);
        // jaccard = 3/3 = 1.0, keyword_overlap = 0, name_overlap = 0
        // w_topics=0.50, w_desc=0.35, w_name=0.15 → 0.50 * 1.0 = 0.50
        assert!((score - 0.5).abs() < 1e-9, "score = {score}");
    }

    #[test]
    fn no_overlap_returns_low_score() {
        let own_repos = vec![own("my-web-app", Some("web framework for javascript"), &["javascript", "web"])];
        let score = semantic_score(
            Some("a rust command line tool"),
            &["rust".into(), "cli".into()],
            &own_repos,
        );
        // No topic overlap at all → jaccard = 0
        // Keyword overlap: web/framework/javascript vs rust/command/line/tool → no common
        assert!(score < 0.05, "expected near zero, got {score}");
    }

    #[test]
    fn description_overlap_contributes() {
        let own_repos = vec![own("my-analyzer", Some("static analysis tool for code"), &[])];
        let score = semantic_score(
            Some("a static analysis tool for rust code"),
            &[],
            &own_repos,
        );
        // jaccard = 0, keyword_overlap > 0 (static, analysis, tool, code)
        assert!(score > 0.0, "description overlap should contribute: {score}");
        assert!(score <= 1.0, "score must not exceed 1.0: {score}");
    }

    #[test]
    fn combined_topics_and_description() {
        let own_repos = vec![own(
            "my-cli",
            Some("command line tool for developers"),
            &["cli", "rust"],
        )];
        let score = semantic_score(
            Some("fast command line tool written in rust"),
            &["cli".into(), "rust".into(), "async".into()],
            &own_repos,
        );
        // jaccard: {cli, rust} ∩ {cli, rust, async} = 2, union = 3 → 2/3 ≈ 0.667
        // keyword_overlap: command/line/tool vs fast/command/line/tool/written/rust → 3 common / max
        assert!(score > 0.3, "combined score should be significant: {score}");
        assert!(score <= 1.0);
    }

    #[test]
    fn max_over_multiple_own_repos() {
        let own_repos = vec![
            own("unrelated", Some("something else entirely"), &["python", "ml"]),
            own("very-relevant", Some("cli tool in rust"), &["cli", "rust"]),
        ];
        let score_multi = semantic_score(
            Some("a cli tool"),
            &["cli".into(), "rust".into()],
            &own_repos,
        );
        let score_single = semantic_score(
            Some("a cli tool"),
            &["cli".into(), "rust".into()],
            &[own("very-relevant", Some("cli tool in rust"), &["cli", "rust"])],
        );
        // Multi should equal single (max picks the higher one)
        assert!((score_multi - score_single).abs() < 1e-9,
            "multi={score_multi}, single={score_single}");
    }

    #[test]
    fn stop_words_excluded_from_keyword_overlap() {
        let own_repos = vec![own("proj", Some("the best tool for the developers"), &[])];
        let score = semantic_score(
            Some("the a an is of for"),
            &[],
            &own_repos,
        );
        // After filtering stop words, no meaningful words remain → low/zero overlap
        assert!(score < 0.1, "stop words should not drive score up: {score}");
    }

    #[test]
    fn score_clamped_to_zero_one() {
        let own_repos = vec![own("exact", Some("identical description here"), &["a", "b", "c"])];
        let score = semantic_score(
            Some("identical description here"),
            &["a".into(), "b".into(), "c".into()],
            &own_repos,
        );
        assert!(score >= 0.0 && score <= 1.0, "score out of range: {score}");
    }

    #[test]
    fn html_tags_stripped_before_scoring() {
        // Description wrapped in HTML like real RSS feeds
        let own_repos = vec![own("memory-agent", Some("persistent memory for AI agents"), &["ai", "agents"])];
        let html_desc = Some(r#"<p><img src="banner.png"/></p><h1>persistent memory for AI agents via knowledge graphs</h1>"#);
        let plain_desc = Some("persistent memory for AI agents via knowledge graphs");

        let score_html = semantic_score(html_desc, &["ai".into(), "agents".into()], &own_repos);
        let score_plain = semantic_score(plain_desc, &["ai".into(), "agents".into()], &own_repos);

        // Both should produce similar non-zero scores (HTML stripped before tokenizing)
        assert!(score_html > 0.1, "HTML description should score meaningfully after stripping: {score_html}");
        assert!((score_html - score_plain).abs() < 0.1, "HTML vs plain should be close: html={score_html}, plain={score_plain}");
    }
}
