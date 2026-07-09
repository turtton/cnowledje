use url::Url;

use crate::error::ConfluenceError;

// ── JQL generation ────────────────────────────────────────────────────────────

/// Escape a value for use inside a JQL double-quoted string.
pub fn escape_jql(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn project_clause(projects: &[String]) -> String {
    if projects.len() == 1 {
        format!("project = \"{}\"", escape_jql(&projects[0]))
    } else {
        let keys: Vec<String> = projects
            .iter()
            .map(|p| format!("\"{}\"", escape_jql(p)))
            .collect();
        format!("project in ({})", keys.join(", "))
    }
}

/// Build a JQL `field = "..."` / `field in (...)` clause depending on arity.
fn multi_value_clause(field: &str, values: &[String]) -> String {
    if values.len() == 1 {
        format!("{} = \"{}\"", field, escape_jql(&values[0]))
    } else {
        let vals: Vec<String> = values
            .iter()
            .map(|v| format!("\"{}\"", escape_jql(v)))
            .collect();
        format!("{} in ({})", field, vals.join(", "))
    }
}

/// Filters applied to a Jira issue search, on top of the project scope and
/// free-text query.
pub struct JqlFilters<'a> {
    pub statuses: &'a [String],
    pub assignee: Option<&'a str>,
    pub reporter: Option<&'a str>,
    pub issue_types: &'a [String],
    pub labels: &'a [String],
}

impl JqlFilters<'_> {
    /// True when no filter field is set.
    pub fn is_empty(&self) -> bool {
        self.statuses.is_empty()
            && self.assignee.is_none()
            && self.reporter.is_none()
            && self.issue_types.is_empty()
            && self.labels.is_empty()
    }
}

/// Build a JQL query for `jira search`.
///
/// Clauses are AND-joined in a fixed order (project, text, status, assignee,
/// reporter, issuetype, labels) so generated JQL is stable and testable, with
/// a trailing `ORDER BY updated DESC`.
pub fn build_search_jql(projects: &[String], query: Option<&str>, filters: &JqlFilters) -> String {
    let mut clauses = vec![project_clause(projects)];

    if let Some(q) = query {
        let q = q.trim();
        if !q.is_empty() {
            clauses.push(format!("text ~ \"{}\"", escape_jql(q)));
        }
    }

    if !filters.statuses.is_empty() {
        clauses.push(multi_value_clause("status", filters.statuses));
    }
    if let Some(assignee) = filters.assignee {
        clauses.push(format!("assignee = \"{}\"", escape_jql(assignee)));
    }
    if let Some(reporter) = filters.reporter {
        clauses.push(format!("reporter = \"{}\"", escape_jql(reporter)));
    }
    if !filters.issue_types.is_empty() {
        clauses.push(multi_value_clause("issuetype", filters.issue_types));
    }
    if !filters.labels.is_empty() {
        clauses.push(multi_value_clause("labels", filters.labels));
    }

    format!("{} ORDER BY updated DESC", clauses.join(" AND "))
}

// ── Issue key extraction ──────────────────────────────────────────────────────

/// True when `s` looks like a Jira issue key: an ASCII-letter-led project
/// prefix (letters/digits/`_`), a single trailing `-`, and a non-empty
/// all-digit issue number (e.g. `PROJ-123`).
fn is_issue_key(s: &str) -> bool {
    match s.rsplit_once('-') {
        Some((left, right)) => {
            !left.is_empty()
                && left.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && left.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !right.is_empty()
                && right.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Accept a raw issue key (e.g. `PROJ-123`) or a Jira issue URL containing
/// `/browse/<KEY>` and return the normalized (uppercased) key.
///
/// Other Jira URL shapes (e.g. Cloud's `?selectedIssue=`) are intentionally
/// unsupported and return [`ConfluenceError::InvalidIssueKey`].
pub fn extract_issue_key(input: &str) -> Result<String, ConfluenceError> {
    let trimmed = input.trim();

    if is_issue_key(trimmed) {
        return Ok(trimmed.to_ascii_uppercase());
    }

    if let Ok(url) = Url::parse(trimmed) {
        let path = url.path();
        let mut segments = path.split('/').filter(|s| !s.is_empty());
        while let Some(seg) = segments.next() {
            if seg.eq_ignore_ascii_case("browse") {
                if let Some(next) = segments.next() {
                    if is_issue_key(next) {
                        return Ok(next.to_ascii_uppercase());
                    }
                }
            }
        }
    }

    Err(ConfluenceError::InvalidIssueKey(input.to_string()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_jql_no_special() {
        assert_eq!(escape_jql("Redis 設計"), "Redis 設計");
    }

    #[test]
    fn test_escape_jql_quotes() {
        assert_eq!(escape_jql(r#"he said "hi""#), r#"he said \"hi\""#);
    }

    #[test]
    fn test_escape_jql_backslash() {
        assert_eq!(escape_jql(r"back\slash"), r"back\\slash");
    }

    fn empty_filters() -> JqlFilters<'static> {
        JqlFilters {
            statuses: &[],
            assignee: None,
            reporter: None,
            issue_types: &[],
            labels: &[],
        }
    }

    #[test]
    fn test_build_search_jql_single_project() {
        let jql = build_search_jql(&["DEV".to_string()], None, &empty_filters());
        assert_eq!(jql, r#"project = "DEV" ORDER BY updated DESC"#);
    }

    #[test]
    fn test_build_search_jql_multiple_projects() {
        let jql = build_search_jql(
            &["DEV".to_string(), "OPS".to_string()],
            None,
            &empty_filters(),
        );
        assert_eq!(jql, r#"project in ("DEV", "OPS") ORDER BY updated DESC"#);
    }

    #[test]
    fn test_build_search_jql_query_present() {
        let jql = build_search_jql(&["DEV".to_string()], Some("redis"), &empty_filters());
        assert_eq!(
            jql,
            r#"project = "DEV" AND text ~ "redis" ORDER BY updated DESC"#
        );
    }

    #[test]
    fn test_build_search_jql_query_none_omits_text_clause() {
        let jql = build_search_jql(&["DEV".to_string()], None, &empty_filters());
        assert!(!jql.contains("text ~"));
    }

    #[test]
    fn test_build_search_jql_query_whitespace_only_omits_text_clause() {
        let jql = build_search_jql(&["DEV".to_string()], Some("   "), &empty_filters());
        assert!(!jql.contains("text ~"));
    }

    #[test]
    fn test_build_search_jql_single_status() {
        let filters = JqlFilters {
            statuses: &["Open".to_string()],
            ..empty_filters()
        };
        let jql = build_search_jql(&["DEV".to_string()], None, &filters);
        assert_eq!(
            jql,
            r#"project = "DEV" AND status = "Open" ORDER BY updated DESC"#
        );
    }

    #[test]
    fn test_build_search_jql_multiple_statuses() {
        let filters = JqlFilters {
            statuses: &["Open".to_string(), "In Progress".to_string()],
            ..empty_filters()
        };
        let jql = build_search_jql(&["DEV".to_string()], None, &filters);
        assert_eq!(
            jql,
            r#"project = "DEV" AND status in ("Open", "In Progress") ORDER BY updated DESC"#
        );
    }

    #[test]
    fn test_build_search_jql_assignee() {
        let filters = JqlFilters {
            assignee: Some("alice"),
            ..empty_filters()
        };
        let jql = build_search_jql(&["DEV".to_string()], None, &filters);
        assert_eq!(
            jql,
            r#"project = "DEV" AND assignee = "alice" ORDER BY updated DESC"#
        );
    }

    #[test]
    fn test_build_search_jql_reporter() {
        let filters = JqlFilters {
            reporter: Some("bob"),
            ..empty_filters()
        };
        let jql = build_search_jql(&["DEV".to_string()], None, &filters);
        assert_eq!(
            jql,
            r#"project = "DEV" AND reporter = "bob" ORDER BY updated DESC"#
        );
    }

    #[test]
    fn test_build_search_jql_single_issue_type() {
        let filters = JqlFilters {
            issue_types: &["Bug".to_string()],
            ..empty_filters()
        };
        let jql = build_search_jql(&["DEV".to_string()], None, &filters);
        assert_eq!(
            jql,
            r#"project = "DEV" AND issuetype = "Bug" ORDER BY updated DESC"#
        );
    }

    #[test]
    fn test_build_search_jql_multiple_issue_types() {
        let filters = JqlFilters {
            issue_types: &["Bug".to_string(), "Task".to_string()],
            ..empty_filters()
        };
        let jql = build_search_jql(&["DEV".to_string()], None, &filters);
        assert_eq!(
            jql,
            r#"project = "DEV" AND issuetype in ("Bug", "Task") ORDER BY updated DESC"#
        );
    }

    #[test]
    fn test_build_search_jql_single_label() {
        let filters = JqlFilters {
            labels: &["urgent".to_string()],
            ..empty_filters()
        };
        let jql = build_search_jql(&["DEV".to_string()], None, &filters);
        assert_eq!(
            jql,
            r#"project = "DEV" AND labels = "urgent" ORDER BY updated DESC"#
        );
    }

    #[test]
    fn test_build_search_jql_multiple_labels() {
        let filters = JqlFilters {
            labels: &["urgent".to_string(), "backend".to_string()],
            ..empty_filters()
        };
        let jql = build_search_jql(&["DEV".to_string()], None, &filters);
        assert_eq!(
            jql,
            r#"project = "DEV" AND labels in ("urgent", "backend") ORDER BY updated DESC"#
        );
    }

    #[test]
    fn test_build_search_jql_combined_filters_clause_order() {
        let filters = JqlFilters {
            statuses: &["Open".to_string(), "In Progress".to_string()],
            assignee: Some("alice"),
            reporter: Some("bob"),
            issue_types: &["Bug".to_string()],
            labels: &["urgent".to_string(), "backend".to_string()],
        };
        let jql = build_search_jql(&["DEV".to_string()], Some("redis"), &filters);
        assert_eq!(
            jql,
            r#"project = "DEV" AND text ~ "redis" AND status in ("Open", "In Progress") AND assignee = "alice" AND reporter = "bob" AND issuetype = "Bug" AND labels in ("urgent", "backend") ORDER BY updated DESC"#
        );
    }

    #[test]
    fn test_build_search_jql_filters_only_no_query() {
        let filters = JqlFilters {
            statuses: &["Open".to_string()],
            ..empty_filters()
        };
        assert!(!filters.is_empty());
        let jql = build_search_jql(&["DEV".to_string()], None, &filters);
        assert!(!jql.contains("text ~"));
        assert_eq!(
            jql,
            r#"project = "DEV" AND status = "Open" ORDER BY updated DESC"#
        );
    }

    #[test]
    fn test_jql_filters_is_empty_true_when_all_unset() {
        assert!(empty_filters().is_empty());
    }

    #[test]
    fn test_jql_filters_is_empty_false_when_statuses_set() {
        let filters = JqlFilters {
            statuses: &["Open".to_string()],
            ..empty_filters()
        };
        assert!(!filters.is_empty());
    }

    #[test]
    fn test_jql_filters_is_empty_false_when_assignee_set() {
        let filters = JqlFilters {
            assignee: Some("alice"),
            ..empty_filters()
        };
        assert!(!filters.is_empty());
    }

    #[test]
    fn test_extract_issue_key_bare_key_uppercase_passthrough() {
        assert_eq!(extract_issue_key("PROJ-123").unwrap(), "PROJ-123");
    }

    #[test]
    fn test_extract_issue_key_lowercase_normalized() {
        assert_eq!(extract_issue_key("proj-123").unwrap(), "PROJ-123");
    }

    #[test]
    fn test_extract_issue_key_from_browse_url() {
        let key = extract_issue_key("https://jira.example.com/browse/PROJ-42").unwrap();
        assert_eq!(key, "PROJ-42");
    }

    #[test]
    fn test_extract_issue_key_from_browse_url_with_query_string() {
        let key = extract_issue_key("https://jira.example.com/browse/proj-42?focusedCommentId=1")
            .unwrap();
        assert_eq!(key, "PROJ-42");
    }

    #[test]
    fn test_extract_issue_key_invalid_bare_word_returns_invalid_issue_key_err() {
        let err = extract_issue_key("not-a-key").unwrap_err();
        assert!(matches!(err, ConfluenceError::InvalidIssueKey(_)));
    }

    #[test]
    fn test_extract_issue_key_unrelated_url_returns_invalid_issue_key_err() {
        let err =
            extract_issue_key("https://jira.example.com/issues/?selectedIssue=42").unwrap_err();
        assert!(matches!(err, ConfluenceError::InvalidIssueKey(_)));
    }
}
