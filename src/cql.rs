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

/// Build a CQL query that matches the page title.
pub fn build_title_cql(spaces: &[String], query: &str) -> String {
    format!(
        "{} AND type = page AND title ~ \"{}\" ORDER BY lastmodified DESC",
        space_clause(spaces),
        escape_cql(query)
    )
}

/// Build a CQL query that searches the full page text.
pub fn build_text_cql(spaces: &[String], query: &str) -> String {
    format!(
        "{} AND type = page AND text ~ \"{}\" ORDER BY lastmodified DESC",
        space_clause(spaces),
        escape_cql(query)
    )
}

/// Return the CQL queries needed for the requested [`SearchIn`] mode.
///
/// `Both` returns two queries (title first, text second).
pub fn build_cql_queries(
    spaces: &[String],
    query: &str,
    search_in: &SearchIn,
) -> Vec<(SearchIn, String)> {
    match search_in {
        SearchIn::Title => vec![(SearchIn::Title, build_title_cql(spaces, query))],
        SearchIn::Text => vec![(SearchIn::Text, build_text_cql(spaces, query))],
        SearchIn::Both => vec![
            (SearchIn::Title, build_title_cql(spaces, query)),
            (SearchIn::Text, build_text_cql(spaces, query)),
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
    let url = Url::parse(trimmed).map_err(|_| ConfluenceError::InvalidPageUrl(input.to_string()))?;

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
        let cql = build_title_cql(&["DEV".to_string()], "Redis");
        assert_eq!(
            cql,
            r#"space = "DEV" AND type = page AND title ~ "Redis" ORDER BY lastmodified DESC"#
        );
    }

    #[test]
    fn test_build_text_cql_multi_space() {
        let cql = build_text_cql(&["DEV".to_string(), "ARCH".to_string()], "Redis");
        assert_eq!(
            cql,
            r#"space in ("DEV", "ARCH") AND type = page AND text ~ "Redis" ORDER BY lastmodified DESC"#
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
        let id =
            extract_page_id("https://confluence.example.local/pages/123456789").unwrap();
        assert_eq!(id, "123456789");
    }

    #[test]
    fn test_extract_page_id_invalid() {
        assert!(extract_page_id("not-a-url-or-id").is_err());
    }

    #[test]
    fn test_build_cql_queries_both() {
        let qs = build_cql_queries(&["DEV".to_string()], "Redis", &SearchIn::Both);
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].0, SearchIn::Title);
        assert_eq!(qs[1].0, SearchIn::Text);
    }
}
