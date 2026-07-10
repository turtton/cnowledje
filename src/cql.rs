use url::Url;

use crate::error::ConfluenceError;
use crate::types::SearchIn;

// ── CQL generation ────────────────────────────────────────────────────────────

/// Escape a value for use inside a CQL double-quoted string.
pub fn escape_cql(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn space_clause(spaces: &[String]) -> String {
    if spaces.len() == 1 {
        format!("space = \"{}\"", escape_cql(&spaces[0]))
    } else {
        let keys: Vec<String> = spaces
            .iter()
            .map(|s| format!("\"{}\"", escape_cql(s)))
            .collect();
        format!("space in ({})", keys.join(", "))
    }
}

fn label_clause(labels: &[String]) -> String {
    if labels.len() == 1 {
        format!("label = \"{}\"", escape_cql(&labels[0]))
    } else {
        let names: Vec<String> = labels
            .iter()
            .map(|label| format!("\"{}\"", escape_cql(label)))
            .collect();
        format!("label in ({})", names.join(", "))
    }
}

/// Build a CQL query that matches the page title.
pub fn build_title_cql(spaces: &[String], query: &str, labels: &[String]) -> String {
    let label = if labels.is_empty() {
        String::new()
    } else {
        format!(" AND {}", label_clause(labels))
    };
    format!(
        "{} AND type = page AND title ~ \"{}\"{} ORDER BY lastmodified DESC",
        space_clause(spaces),
        escape_cql(query),
        label
    )
}

/// Build a CQL query that searches the full page text.
pub fn build_text_cql(spaces: &[String], query: &str, labels: &[String]) -> String {
    let label = if labels.is_empty() {
        String::new()
    } else {
        format!(" AND {}", label_clause(labels))
    };
    format!(
        "{} AND type = page AND text ~ \"{}\"{} ORDER BY lastmodified DESC",
        space_clause(spaces),
        escape_cql(query),
        label
    )
}

/// Build a CQL query that matches pages by label without a text query.
pub fn build_label_cql(spaces: &[String], labels: &[String]) -> String {
    format!(
        "{} AND type = page AND {} ORDER BY lastmodified DESC",
        space_clause(spaces),
        label_clause(labels)
    )
}

/// Build a CQL query that matches a page by exact title within one space.
///
/// Used to resolve an `excerpt-include` page reference (title + optional
/// space) to an ID, since the storage format never carries one directly.
pub fn build_exact_title_cql(space: &str, title: &str) -> String {
    format!(
        "space = \"{}\" AND type = page AND title = \"{}\"",
        escape_cql(space),
        escape_cql(title)
    )
}

/// Return the CQL queries needed for the requested [`SearchIn`] mode.
///
/// `Both` returns two queries (title first, text second).
pub fn build_cql_queries(
    spaces: &[String],
    query: &str,
    search_in: &SearchIn,
    labels: &[String],
) -> Vec<(SearchIn, String)> {
    match search_in {
        SearchIn::Title => vec![(SearchIn::Title, build_title_cql(spaces, query, labels))],
        SearchIn::Text => vec![(SearchIn::Text, build_text_cql(spaces, query, labels))],
        SearchIn::Both => vec![
            (SearchIn::Title, build_title_cql(spaces, query, labels)),
            (SearchIn::Text, build_text_cql(spaces, query, labels)),
        ],
    }
}

// ── Page ID extraction ────────────────────────────────────────────────────────

/// Accept a raw numeric ID or a Confluence page URL and return the page ID.
pub fn extract_page_id(input: &str) -> Result<String, ConfluenceError> {
    let trimmed = input.trim();

    // Pure numeric string → use directly.
    if trimmed.chars().all(|c| c.is_ascii_digit()) && !trimmed.is_empty() {
        return Ok(trimmed.to_string());
    }

    // Try URL parsing.
    let url =
        Url::parse(trimmed).map_err(|_| ConfluenceError::InvalidPageUrl(input.to_string()))?;

    // ?pageId=<id>
    for (key, val) in url.query_pairs() {
        if key == "pageId" && val.chars().all(|c| c.is_ascii_digit()) {
            return Ok(val.into_owned());
        }
    }

    // /pages/<numeric_segment>
    let path = url.path();
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    while let Some(seg) = segments.next() {
        if seg.eq_ignore_ascii_case("pages") {
            if let Some(next) = segments.next() {
                if !next.is_empty() && next.chars().all(|c| c.is_ascii_digit()) {
                    return Ok(next.to_string());
                }
            }
        }
    }

    Err(ConfluenceError::InvalidPageUrl(input.to_string()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_cql_no_special() {
        assert_eq!(escape_cql("Redis 設計"), "Redis 設計");
    }

    #[test]
    fn test_escape_cql_quotes() {
        assert_eq!(escape_cql(r#"he said "hi""#), r#"he said \"hi\""#);
    }

    #[test]
    fn test_escape_cql_backslash() {
        assert_eq!(escape_cql(r"back\slash"), r"back\\slash");
    }

    #[test]
    fn test_build_title_cql_single_space() {
        let cql = build_title_cql(&["DEV".to_string()], "Redis", &[]);
        assert_eq!(
            cql,
            r#"space = "DEV" AND type = page AND title ~ "Redis" ORDER BY lastmodified DESC"#
        );
    }

    #[test]
    fn test_build_exact_title_cql() {
        let cql = build_exact_title_cql("DEV", "Source Page");
        assert_eq!(
            cql,
            r#"space = "DEV" AND type = page AND title = "Source Page""#
        );
    }

    #[test]
    fn test_build_text_cql_multi_space() {
        let cql = build_text_cql(&["DEV".to_string(), "ARCH".to_string()], "Redis", &[]);
        assert_eq!(
            cql,
            r#"space in ("DEV", "ARCH") AND type = page AND text ~ "Redis" ORDER BY lastmodified DESC"#
        );
    }

    #[test]
    fn test_build_title_cql_single_label() {
        let cql = build_title_cql(&["DEV".to_string()], "Redis", &["api".to_string()]);
        assert_eq!(
            cql,
            r#"space = "DEV" AND type = page AND title ~ "Redis" AND label = "api" ORDER BY lastmodified DESC"#
        );
    }

    #[test]
    fn test_build_label_cql_multiple_labels() {
        let cql = build_label_cql(
            &["DEV".to_string()],
            &["api".to_string(), "設計".to_string()],
        );
        assert_eq!(
            cql,
            r#"space = "DEV" AND type = page AND label in ("api", "設計") ORDER BY lastmodified DESC"#
        );
    }

    #[test]
    fn test_build_label_cql_escapes_label() {
        let cql = build_label_cql(&["DEV".to_string()], &[r#"a"b\c"#.to_string()]);
        assert_eq!(
            cql,
            r#"space = "DEV" AND type = page AND label = "a\"b\\c" ORDER BY lastmodified DESC"#
        );
    }

    #[test]
    fn test_extract_page_id_numeric() {
        assert_eq!(extract_page_id("123456789").unwrap(), "123456789");
    }

    #[test]
    fn test_extract_page_id_query_param() {
        let id = extract_page_id(
            "https://confluence.example.local/pages/viewpage.action?pageId=123456789",
        )
        .unwrap();
        assert_eq!(id, "123456789");
    }

    #[test]
    fn test_extract_page_id_path_segment() {
        let id = extract_page_id("https://confluence.example.local/pages/123456789").unwrap();
        assert_eq!(id, "123456789");
    }

    #[test]
    fn test_extract_page_id_invalid() {
        assert!(extract_page_id("not-a-url-or-id").is_err());
    }

    #[test]
    fn test_build_cql_queries_both() {
        let qs = build_cql_queries(&["DEV".to_string()], "Redis", &SearchIn::Both, &[]);
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].0, SearchIn::Title);
        assert_eq!(qs[1].0, SearchIn::Text);
    }
}
