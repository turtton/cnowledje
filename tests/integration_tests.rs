//! Integration tests for the cnowledje library.
//!
//! These tests cover the core logic without making live HTTP requests.
//! HTTP client behavior (Authorization headers, status code handling) is
//! verified by unit tests inside each module.

use cnowledje::cql;
use cnowledje::error::ConfluenceError;
use cnowledje::jql;
use cnowledje::markdown;
use cnowledje::models;
use cnowledje::types::SearchIn;

// ── CQL generation ─────────────────────────────────────────────────────────────

#[test]
fn cql_title_single_space() {
    let q = cql::build_title_cql(&["DEV".to_string()], "Redis 設計", &[]);
    assert!(q.starts_with(r#"space = "DEV""#));
    assert!(q.contains("title ~"));
    assert!(q.contains("Redis 設計"));
    assert!(q.contains("ORDER BY lastmodified DESC"));
}

#[test]
fn cql_text_multiple_spaces() {
    let q = cql::build_text_cql(&["DEV".to_string(), "ARCH".to_string()], "Redis", &[]);
    assert!(q.starts_with(r#"space in ("DEV", "ARCH")"#));
    assert!(q.contains("text ~"));
}

#[test]
fn cql_escape_double_quotes_in_query() {
    let q = cql::build_title_cql(&["DEV".to_string()], r#"say "hello""#, &[]);
    assert!(q.contains(r#"say \"hello\""#));
}

#[test]
fn cql_escape_backslash_in_query() {
    let q = cql::build_title_cql(&["DEV".to_string()], r"back\slash", &[]);
    assert!(q.contains(r"back\\slash"));
}

#[test]
fn cql_both_returns_two_queries() {
    let qs = cql::build_cql_queries(&["DEV".to_string()], "test", &SearchIn::Both, &[]);
    assert_eq!(qs.len(), 2);
    assert!(matches!(qs[0].0, SearchIn::Title));
    assert!(matches!(qs[1].0, SearchIn::Text));
}

#[test]
fn cql_title_only_returns_one_query() {
    let qs = cql::build_cql_queries(&["DEV".to_string()], "test", &SearchIn::Title, &[]);
    assert_eq!(qs.len(), 1);
    assert!(matches!(qs[0].0, SearchIn::Title));
}

// ── Page ID extraction ─────────────────────────────────────────────────────────

#[test]
fn page_id_from_numeric_string() {
    assert_eq!(cql::extract_page_id("123456789").unwrap(), "123456789");
}

#[test]
fn page_id_from_viewpage_url() {
    let id = cql::extract_page_id(
        "https://confluence.example.local/pages/viewpage.action?pageId=987654321",
    )
    .unwrap();
    assert_eq!(id, "987654321");
}

#[test]
fn page_id_from_path_url() {
    let id = cql::extract_page_id("https://confluence.example.local/pages/111222333").unwrap();
    assert_eq!(id, "111222333");
}

#[test]
fn page_id_invalid_returns_err() {
    assert!(cql::extract_page_id("not-a-page").is_err());
    assert!(cql::extract_page_id("https://example.com/wiki/display/DEV/Redis").is_err());
}

// ── JQL generation ─────────────────────────────────────────────────────────────

#[test]
fn jql_project_single() {
    let filters = jql::JqlFilters {
        statuses: &[],
        assignee: None,
        reporter: None,
        issue_types: &[],
        labels: &[],
    };
    let q = jql::build_search_jql(&["DEV".to_string()], None, &filters);
    assert_eq!(q, r#"project = "DEV" ORDER BY updated DESC"#);
}

#[test]
fn jql_project_multiple() {
    let filters = jql::JqlFilters {
        statuses: &[],
        assignee: None,
        reporter: None,
        issue_types: &[],
        labels: &[],
    };
    let q = jql::build_search_jql(&["DEV".to_string(), "OPS".to_string()], None, &filters);
    assert_eq!(q, r#"project in ("DEV", "OPS") ORDER BY updated DESC"#);
}

#[test]
fn jql_query_text_clause_present_when_query_given() {
    let filters = jql::JqlFilters {
        statuses: &[],
        assignee: None,
        reporter: None,
        issue_types: &[],
        labels: &[],
    };
    let q = jql::build_search_jql(&["DEV".to_string()], Some("redis"), &filters);
    assert!(q.contains(r#"text ~ "redis""#));
}

#[test]
fn jql_query_text_clause_absent_when_none_or_blank() {
    let filters = jql::JqlFilters {
        statuses: &[],
        assignee: None,
        reporter: None,
        issue_types: &[],
        labels: &[],
    };
    let none_q = jql::build_search_jql(&["DEV".to_string()], None, &filters);
    assert!(!none_q.contains("text ~"));

    let blank_q = jql::build_search_jql(&["DEV".to_string()], Some("   "), &filters);
    assert!(!blank_q.contains("text ~"));
}

#[test]
fn jql_filter_status_single_and_multiple() {
    let single = jql::JqlFilters {
        statuses: &["Open".to_string()],
        assignee: None,
        reporter: None,
        issue_types: &[],
        labels: &[],
    };
    let q = jql::build_search_jql(&["DEV".to_string()], None, &single);
    assert!(q.contains(r#"status = "Open""#));

    let multi = jql::JqlFilters {
        statuses: &["Open".to_string(), "In Progress".to_string()],
        assignee: None,
        reporter: None,
        issue_types: &[],
        labels: &[],
    };
    let q = jql::build_search_jql(&["DEV".to_string()], None, &multi);
    assert!(q.contains(r#"status in ("Open", "In Progress")"#));
}

#[test]
fn jql_filter_assignee() {
    let filters = jql::JqlFilters {
        statuses: &[],
        assignee: Some("alice"),
        reporter: None,
        issue_types: &[],
        labels: &[],
    };
    let q = jql::build_search_jql(&["DEV".to_string()], None, &filters);
    assert!(q.contains(r#"assignee = "alice""#));
}

#[test]
fn jql_filter_reporter() {
    let filters = jql::JqlFilters {
        statuses: &[],
        assignee: None,
        reporter: Some("bob"),
        issue_types: &[],
        labels: &[],
    };
    let q = jql::build_search_jql(&["DEV".to_string()], None, &filters);
    assert!(q.contains(r#"reporter = "bob""#));
}

#[test]
fn jql_filter_issue_type_single_and_multiple() {
    let single = jql::JqlFilters {
        statuses: &[],
        assignee: None,
        reporter: None,
        issue_types: &["Bug".to_string()],
        labels: &[],
    };
    let q = jql::build_search_jql(&["DEV".to_string()], None, &single);
    assert!(q.contains(r#"issuetype = "Bug""#));

    let multi = jql::JqlFilters {
        statuses: &[],
        assignee: None,
        reporter: None,
        issue_types: &["Bug".to_string(), "Task".to_string()],
        labels: &[],
    };
    let q = jql::build_search_jql(&["DEV".to_string()], None, &multi);
    assert!(q.contains(r#"issuetype in ("Bug", "Task")"#));
}

#[test]
fn jql_filter_label_single_and_multiple() {
    let single = jql::JqlFilters {
        statuses: &[],
        assignee: None,
        reporter: None,
        issue_types: &[],
        labels: &["urgent".to_string()],
    };
    let q = jql::build_search_jql(&["DEV".to_string()], None, &single);
    assert!(q.contains(r#"labels = "urgent""#));

    let multi = jql::JqlFilters {
        statuses: &[],
        assignee: None,
        reporter: None,
        issue_types: &[],
        labels: &["urgent".to_string(), "backend".to_string()],
    };
    let q = jql::build_search_jql(&["DEV".to_string()], None, &multi);
    assert!(q.contains(r#"labels in ("urgent", "backend")"#));
}

#[test]
fn jql_combined_filters_clause_order_and_trailing_sort() {
    let filters = jql::JqlFilters {
        statuses: &["Open".to_string(), "In Progress".to_string()],
        assignee: Some("alice"),
        reporter: Some("bob"),
        issue_types: &["Bug".to_string()],
        labels: &["urgent".to_string(), "backend".to_string()],
    };
    let q = jql::build_search_jql(&["DEV".to_string()], Some("redis"), &filters);
    assert_eq!(
        q,
        r#"project = "DEV" AND text ~ "redis" AND status in ("Open", "In Progress") AND assignee = "alice" AND reporter = "bob" AND issuetype = "Bug" AND labels in ("urgent", "backend") ORDER BY updated DESC"#
    );
}

#[test]
fn jql_filters_only_no_query_still_produces_jql() {
    let filters = jql::JqlFilters {
        statuses: &["Open".to_string()],
        assignee: None,
        reporter: None,
        issue_types: &[],
        labels: &[],
    };
    assert!(!filters.is_empty());
    let q = jql::build_search_jql(&["DEV".to_string()], None, &filters);
    assert!(!q.contains("text ~"));
    assert_eq!(
        q,
        r#"project = "DEV" AND status = "Open" ORDER BY updated DESC"#
    );
}

#[test]
fn jql_filters_is_empty_true_when_all_unset() {
    let filters = jql::JqlFilters {
        statuses: &[],
        assignee: None,
        reporter: None,
        issue_types: &[],
        labels: &[],
    };
    assert!(filters.is_empty());
}

#[test]
fn jql_filters_is_empty_false_when_status_set() {
    let filters = jql::JqlFilters {
        statuses: &["Open".to_string()],
        assignee: None,
        reporter: None,
        issue_types: &[],
        labels: &[],
    };
    assert!(!filters.is_empty());
}

#[test]
fn jql_filters_is_empty_false_when_assignee_set() {
    let filters = jql::JqlFilters {
        statuses: &[],
        assignee: Some("alice"),
        reporter: None,
        issue_types: &[],
        labels: &[],
    };
    assert!(!filters.is_empty());
}

// ── Issue key extraction ───────────────────────────────────────────────────────

#[test]
fn jql_issue_key_from_bare_key_uppercased() {
    assert_eq!(jql::extract_issue_key("PROJ-123").unwrap(), "PROJ-123");
}

#[test]
fn jql_issue_key_from_lowercase_key_normalized() {
    assert_eq!(jql::extract_issue_key("proj-123").unwrap(), "PROJ-123");
}

#[test]
fn jql_issue_key_from_browse_url() {
    let key = jql::extract_issue_key("https://jira.example.com/browse/PROJ-42").unwrap();
    assert_eq!(key, "PROJ-42");
}

#[test]
fn jql_issue_key_from_browse_url_with_query_string() {
    let key = jql::extract_issue_key("https://jira.example.com/browse/proj-42?focusedCommentId=1")
        .unwrap();
    assert_eq!(key, "PROJ-42");
}

#[test]
fn jql_issue_key_invalid_bare_word_is_err() {
    let err = jql::extract_issue_key("not-a-key").unwrap_err();
    assert!(matches!(err, ConfluenceError::InvalidIssueKey(_)));
}

#[test]
fn jql_issue_key_invalid_unrelated_url_is_err() {
    let err =
        jql::extract_issue_key("https://jira.example.com/issues/?selectedIssue=42").unwrap_err();
    assert!(matches!(err, ConfluenceError::InvalidIssueKey(_)));
}

// ── Markdown conversion ────────────────────────────────────────────────────────

#[test]
fn markdown_heading_levels() {
    let md = markdown::html_to_markdown("<h1>H1</h1><h2>H2</h2><h3>H3</h3>", 50_000, None);
    assert!(md.contains("# H1"));
    assert!(md.contains("## H2"));
    assert!(md.contains("### H3"));
}

#[test]
fn markdown_bold_and_italic() {
    let md = markdown::html_to_markdown("<strong>bold</strong> and <em>italic</em>", 50_000, None);
    assert!(md.contains("**bold**"));
    assert!(md.contains("_italic_"));
}

#[test]
fn markdown_link_with_href() {
    let md =
        markdown::html_to_markdown(r#"<a href="https://example.com">Example</a>"#, 50_000, None);
    assert!(md.contains("[Example](https://example.com)"));
}

#[test]
fn markdown_unordered_list() {
    let md = markdown::html_to_markdown(
        "<ul><li>Alpha</li><li>Beta</li><li>Gamma</li></ul>",
        50_000,
        None,
    );
    assert!(md.contains("- Alpha"));
    assert!(md.contains("- Beta"));
    assert!(md.contains("- Gamma"));
}

#[test]
fn markdown_ordered_list() {
    let md = markdown::html_to_markdown(
        "<ol><li>First</li><li>Second</li><li>Third</li></ol>",
        50_000,
        None,
    );
    assert!(md.contains("1. First"));
    assert!(md.contains("2. Second"));
    assert!(md.contains("3. Third"));
}

#[test]
fn markdown_inline_code() {
    let md = markdown::html_to_markdown("<code>let x = 42;</code>", 50_000, None);
    assert!(md.contains("`let x = 42;`"));
}

#[test]
fn markdown_preformatted_block() {
    let md = markdown::html_to_markdown("<pre><code>fn main() {}</code></pre>", 50_000, None);
    assert!(md.contains("```"));
    assert!(md.contains("fn main()"));
}

#[test]
fn markdown_table_with_header() {
    let html = "<table>\
        <tr><th>Name</th><th>Value</th></tr>\
        <tr><td>foo</td><td>1</td></tr>\
        <tr><td>bar</td><td>2</td></tr>\
        </table>";
    let md = markdown::html_to_markdown(html, 50_000, None);
    assert!(md.contains("| Name | Value |"));
    assert!(md.contains("| --- | --- |"));
    assert!(md.contains("| foo | 1 |"));
    assert!(md.contains("| bar | 2 |"));
}

#[test]
fn markdown_confluence_macro_placeholder() {
    let html = r#"<ac:structured-macro ac:name="jira"><ac:parameter ac:name="key">PROJ-1</ac:parameter></ac:structured-macro>"#;
    let md = markdown::html_to_markdown(html, 50_000, None);
    // Parameter values are surfaced in the placeholder so ticket keys remain readable.
    assert!(
        md.contains("[unsupported confluence macro: jira"),
        "placeholder missing: {}",
        md
    );
    assert!(md.contains("PROJ-1"), "param value should appear: {}", md);
}

#[test]
fn markdown_confluence_info_macro_blockquote() {
    let html = r#"<ac:structured-macro ac:name="info"><ac:rich-text-body><p>Note here</p></ac:rich-text-body></ac:structured-macro>"#;
    let md = markdown::html_to_markdown(html, 50_000, None);
    assert!(
        md.contains("> **Info:**"),
        "Info label should appear: {}",
        md
    );
    assert!(md.contains("Note here"), "body should appear: {}", md);
}

#[test]
fn markdown_japanese_utf8() {
    let md = markdown::html_to_markdown("<p>Redisの利用方針について説明します。</p>", 50_000, None);
    assert!(md.contains("Redisの利用方針について説明します。"));
}

#[test]
fn markdown_truncation_adds_notice() {
    let html = "<p>".to_string() + &"あ".repeat(300) + "</p>";
    let md = markdown::html_to_markdown(&html, 50, None);
    assert!(md.contains("[content truncated]"), "got: {}", md);
}

#[test]
fn markdown_no_truncation_when_short() {
    let md = markdown::html_to_markdown("<p>Short content.</p>", 50_000, None);
    assert!(!md.contains("[content truncated]"));
    assert!(md.contains("Short content."));
}

#[test]
fn markdown_sv_translation_expand_by_language() {
    let html = r#"
<ac:structured-macro ac:name="sv-translation">
  <ac:parameter ac:name="language">ja</ac:parameter>
  <ac:rich-text-body><p>日本語コンテンツ</p></ac:rich-text-body>
</ac:structured-macro>
<ac:structured-macro ac:name="sv-translation">
  <ac:parameter ac:name="language">en</ac:parameter>
  <ac:rich-text-body><p>English content</p></ac:rich-text-body>
</ac:structured-macro>"#;
    let md = markdown::html_to_markdown(html, 50_000, Some("ja"));
    assert!(
        md.contains("日本語コンテンツ"),
        "ja block should be expanded"
    );
    assert!(
        !md.contains("English content"),
        "en block should not be expanded"
    );
}

#[test]
fn markdown_sv_translation_expand_first_when_no_language() {
    let html = r#"
<ac:structured-macro ac:name="sv-translation">
  <ac:parameter ac:name="language">ja</ac:parameter>
  <ac:rich-text-body><p>日本語コンテンツ</p></ac:rich-text-body>
</ac:structured-macro>
<ac:structured-macro ac:name="sv-translation">
  <ac:parameter ac:name="language">en</ac:parameter>
  <ac:rich-text-body><p>English content</p></ac:rich-text-body>
</ac:structured-macro>"#;
    let md = markdown::html_to_markdown(html, 50_000, None);
    assert!(
        md.contains("日本語コンテンツ"),
        "first block should be expanded when language unspecified"
    );
    assert!(
        !md.contains("English content"),
        "second block should not be expanded"
    );
}

// ── Macro conversion integration tests ────────────────────────────────────────

#[test]
fn markdown_confluence_code_macro() {
    let html = r#"<ac:structured-macro ac:name="code">
  <ac:parameter ac:name="language">rust</ac:parameter>
  <ac:plain-text-body><![CDATA[fn main() {
    println!("hello");
}]]></ac:plain-text-body>
</ac:structured-macro>"#;
    let md = markdown::html_to_markdown(html, 50_000, None);
    assert!(md.contains("```rust"), "language fence: {}", md);
    assert!(md.contains("fn main()"), "code body: {}", md);
    assert!(
        md.contains(r#"println!("hello")"#),
        "special chars in code: {}",
        md
    );
}

#[test]
fn markdown_confluence_expand_macro_has_explicit_boundary() {
    let html = r#"<ac:structured-macro ac:name="expand">
  <ac:parameter ac:name="title">Details</ac:parameter>
  <ac:rich-text-body><p>Expanded body content.</p></ac:rich-text-body>
</ac:structured-macro>"#;
    let md = markdown::html_to_markdown(html, 50_000, None);
    assert_eq!(
        md, "<details open>\n<summary>▸ Details</summary>\n\nExpanded body content.\n\n</details>",
        "expand output must delimit the rendered body with an explicit closing tag"
    );
}

#[test]
fn markdown_confluence_expand_macro_default_title_and_empty_body() {
    let html = r#"<ac:structured-macro ac:name="expand">
  <ac:rich-text-body></ac:rich-text-body>
</ac:structured-macro>"#;
    let md = markdown::html_to_markdown(html, 50_000, None);
    assert_eq!(
        md, "<details open>\n<summary>▸ Expand</summary>\n\n</details>",
        "an empty expand still needs a visible range and the default title"
    );
}

#[test]
fn markdown_confluence_expand_macro_nesting_keeps_body_inside_range() {
    let html = r#"<ac:structured-macro ac:name="expand">
  <ac:parameter ac:name="title">Outer</ac:parameter>
  <ac:rich-text-body>
    <p>Outer before.</p>
    <ac:structured-macro ac:name="expand">
      <ac:parameter ac:name="title">Inner</ac:parameter>
      <ac:rich-text-body><p>Inner body.</p></ac:rich-text-body>
    </ac:structured-macro>
    <p>Outer after.</p>
  </ac:rich-text-body>
</ac:structured-macro>
<p>Outside after.</p>"#;
    let md = markdown::html_to_markdown(html, 50_000, None);

    assert_eq!(md.matches("<details open>").count(), 2);
    assert_eq!(md.matches("</details>").count(), 2);

    let outer_open = md.find("<details open>").expect("outer expand opening tag");
    let outer_close = md.rfind("</details>").expect("outer expand closing tag");
    let inner_open = md[outer_open + "<details open>".len()..]
        .find("<details open>")
        .map(|offset| outer_open + "<details open>".len() + offset)
        .expect("nested expand opening tag");
    let inner_close = md[inner_open + "<details open>".len()..]
        .find("</details>")
        .map(|offset| inner_open + "<details open>".len() + offset)
        .expect("nested expand closing tag");

    assert!(
        outer_open < inner_open,
        "nested expand must be inside outer expand"
    );
    assert!(
        inner_open < inner_close,
        "nested expand must close after opening"
    );
    assert!(
        inner_close < outer_close,
        "outer expand must close after nested expand"
    );
    assert!(
        md[outer_open..outer_close].contains("Outer before.")
            && md[outer_open..outer_close].contains("Outer after."),
        "outer body must remain inside the outer range: {md}"
    );
    assert!(
        md[inner_open..inner_close].contains("Inner body."),
        "nested body must remain inside the nested range: {md}"
    );
    assert_eq!(
        md[outer_close + "</details>".len()..].trim(),
        "Outside after.",
        "content following an expand must come after its closing tag"
    );
}

#[test]
fn markdown_confluence_status_inline() {
    let html = r#"<p>Task is <ac:structured-macro ac:name="status"><ac:parameter ac:name="title">IN PROGRESS</ac:parameter></ac:structured-macro> now.</p>"#;
    let md = markdown::html_to_markdown(html, 50_000, None);
    assert!(md.contains("[IN PROGRESS]"), "status badge: {}", md);
    // All text should appear together, not split across separate lines
    let lines: Vec<&str> = md.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("Task is") && l.contains("[IN PROGRESS]") && l.contains("now.")),
        "status must not break paragraph: {md}",
    );
}

// ── Output model constants ─────────────────────────────────────────────────────

#[test]
fn notice_string_present() {
    assert!(models::NOTICE.contains("reference material"));
    assert!(models::NOTICE.contains("instructions"));
}

// ── format helpers ─────────────────────────────────────────────────────────────

use cnowledje::format::make_issue_url;
use cnowledje::format::make_page_url;

#[test]
fn make_page_url_with_api_base() {
    let url = make_page_url(
        "https://config-base.example.com",
        Some("https://api-base.example.com"),
        Some("/pages/viewpage.action?pageId=1"),
    );
    assert_eq!(
        url,
        "https://api-base.example.com/pages/viewpage.action?pageId=1"
    );
}

#[test]
fn make_page_url_fallback_to_config_base() {
    let url = make_page_url(
        "https://confluence.example.local",
        None,
        Some("/pages/viewpage.action?pageId=2"),
    );
    assert_eq!(
        url,
        "https://confluence.example.local/pages/viewpage.action?pageId=2"
    );
}

#[test]
fn make_page_url_no_webui() {
    let url = make_page_url("https://confluence.example.local", None, None);
    assert_eq!(url, "https://confluence.example.local");
}

#[test]
fn make_issue_url_without_trailing_slash() {
    let url = make_issue_url("https://jira.example.com", "PROJ-123");
    assert_eq!(url, "https://jira.example.com/browse/PROJ-123");
}

#[test]
fn make_issue_url_strips_trailing_slash() {
    let url = make_issue_url("https://jira.example.com/", "PROJ-123");
    assert_eq!(url, "https://jira.example.com/browse/PROJ-123");
}

use cnowledje::config::{
    load_profile_config_at_path, profile_exists_at_path, save_profile_to_path, ProfileConfig,
};

#[test]
fn config_save_creates_new_file_with_profile() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let profile = ProfileConfig {
        base_url: Some("https://confluence.example.com".to_string()),
        api_path: Some("/rest/api".to_string()),
        ..Default::default()
    };

    save_profile_to_path("default", &profile, &path).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("[default]"),
        "should contain profile header"
    );
    assert!(content.contains("https://confluence.example.com"));
    assert!(content.contains("/rest/api"));
}

#[test]
fn config_save_preserves_other_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    std::fs::write(
        &path,
        "[staging]\nbase_url = \"https://staging.example.com\"\n",
    )
    .unwrap();

    let profile = ProfileConfig {
        base_url: Some("https://prod.example.com".to_string()),
        ..Default::default()
    };

    save_profile_to_path("default", &profile, &path).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("[staging]"),
        "staging profile should be preserved"
    );
    assert!(content.contains("https://staging.example.com"));
    assert!(content.contains("[default]"));
    assert!(content.contains("https://prod.example.com"));
}

#[test]
fn config_save_omits_none_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let profile = ProfileConfig {
        base_url: Some("https://example.com".to_string()),
        ..Default::default()
    };

    save_profile_to_path("default", &profile, &path).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        !content.contains("default_space"),
        "None fields should not appear"
    );
    assert!(!content.contains("allowed_spaces"));
    assert!(!content.contains("default_limit"));
}

#[test]
fn config_save_overwrites_existing_profile() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    std::fs::write(&path, "[default]\nbase_url = \"https://old.example.com\"\n").unwrap();

    let profile = ProfileConfig {
        base_url: Some("https://new.example.com".to_string()),
        ..Default::default()
    };

    save_profile_to_path("default", &profile, &path).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        !content.contains("old.example.com"),
        "old URL should be replaced"
    );
    assert!(content.contains("new.example.com"));
}

#[test]
fn config_profile_exists_checks_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    assert!(
        !profile_exists_at_path("default", &path).unwrap(),
        "should return false for missing file"
    );

    std::fs::write(&path, "[default]\nbase_url = \"https://example.com\"\n").unwrap();

    assert!(profile_exists_at_path("default", &path).unwrap());
    assert!(
        !profile_exists_at_path("staging", &path).unwrap(),
        "absent profile should return false"
    );
}

#[test]
fn config_save_round_trips_jira_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let profile = ProfileConfig {
        base_url: Some("https://confluence.example.com".to_string()),
        api_path: Some("/rest/api".to_string()),
        jira_base_url: Some("https://jira.example.com".to_string()),
        jira_api_path: Some("/rest/api/2".to_string()),
        jira_allowed_projects: Some(vec!["DEV".into(), "OPS".into()]),
        jira_default_project: Some("DEV".into()),
        ..Default::default()
    };

    save_profile_to_path("default", &profile, &path).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("[default]"),
        "should contain profile header"
    );
    assert!(content.contains("https://confluence.example.com"));
    assert!(content.contains("jira_base_url"));
    assert!(content.contains("https://jira.example.com"));
    assert!(content.contains("jira_api_path"));
    assert!(content.contains("/rest/api/2"));
    assert!(content.contains("jira_allowed_projects"));
    assert!(content.contains("DEV"));
    assert!(content.contains("OPS"));
    assert!(content.contains("jira_default_project"));
}

#[test]
fn config_save_omits_none_jira_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let profile = ProfileConfig {
        base_url: Some("https://example.com".to_string()),
        ..Default::default()
    };

    save_profile_to_path("default", &profile, &path).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        !content.contains("jira_"),
        "no jira_* keys should appear when all jira fields are None"
    );
}

#[test]
fn unified_search_output_preserves_legacy_fields_and_adds_pagination_fields() {
    let output = models::UnifiedSearchOutput {
        query: Some("release readiness".into()),
        confluence: Some(models::SearchOutput {
            query: Some("release readiness".into()),
            spaces: vec!["ENG".into()],
            labels: vec!["public".into()],
            search_in: Some("both".into()),
            returned: 1,
            has_more: false,
            results: vec![models::SearchResultOutput {
                id: "123".into(),
                title: "Release readiness".into(),
                space_key: "ENG".into(),
                space_name: "Engineering".into(),
                url: "https://confluence.example.com/pages/123".into(),
                last_modified: Some("2026-07-10T00:00:00Z".into()),
                matched_by: vec!["title".into()],
                labels: vec!["release".into()],
                excerpt: Some("Ready for release".into()),
            }],
        }),
        jira: Some(models::JiraSearchOutput {
            query: Some("release readiness".into()),
            projects: vec!["OPS".into()],
            jql: "project = OPS ORDER BY updated DESC".into(),
            total: 4,
            returned: 1,
            has_more: true,
            results: vec![models::JiraSearchResultOutput {
                key: "OPS-42".into(),
                summary: "Release readiness".into(),
                status: Some("Open".into()),
                issue_type: Some("Task".into()),
                priority: Some("High".into()),
                assignee: Some("Ada Lovelace".into()),
                project_key: Some("OPS".into()),
                project_name: Some("Operations".into()),
                labels: vec!["release".into()],
                url: "https://jira.example.com/browse/OPS-42".into(),
                updated: Some("2026-07-11T00:00:00.000+0000".into()),
            }],
        }),
    };

    let json = serde_json::to_value(output).unwrap();
    let expected = serde_json::json!({
        "query": "release readiness",
        "confluence": {
            "query": "release readiness",
            "spaces": ["ENG"],
            "labels": ["public"],
            "search_in": "both",
            "returned": 1,
            "has_more": false,
            "results": [{
                "id": "123",
                "title": "Release readiness",
                "space_key": "ENG",
                "space_name": "Engineering",
                "url": "https://confluence.example.com/pages/123",
                "last_modified": "2026-07-10T00:00:00Z",
                "matched_by": ["title"],
                "labels": ["release"],
                "excerpt": "Ready for release"
            }]
        },
        "jira": {
            "query": "release readiness",
            "projects": ["OPS"],
            "jql": "project = OPS ORDER BY updated DESC",
            "total": 4,
            "returned": 1,
            "has_more": true,
            "results": [{
                "key": "OPS-42",
                "summary": "Release readiness",
                "status": "Open",
                "issue_type": "Task",
                "priority": "High",
                "assignee": "Ada Lovelace",
                "project_key": "OPS",
                "project_name": "Operations",
                "labels": ["release"],
                "url": "https://jira.example.com/browse/OPS-42",
                "updated": "2026-07-11T00:00:00.000+0000"
            }]
        }
    });
    assert_eq!(json, expected);
}

#[test]
fn unified_search_output_keeps_null_jira_and_nested_confluence_shape() {
    let output = models::UnifiedSearchOutput {
        query: Some("release readiness".into()),
        confluence: Some(models::SearchOutput {
            query: Some("release readiness".into()),
            spaces: vec!["ENG".into()],
            labels: vec![],
            search_in: Some("both".into()),
            returned: 0,
            has_more: false,
            results: vec![],
        }),
        jira: None,
    };

    let json = serde_json::to_value(output).unwrap();
    assert_eq!(json.get("jira"), Some(&serde_json::Value::Null));
    assert_eq!(json["confluence"]["returned"], serde_json::json!(0));
    assert_eq!(json["confluence"]["has_more"], serde_json::json!(false));
}

#[test]
fn label_only_search_output_serializes_null_query_and_search_in() {
    let output = models::SearchOutput {
        query: None,
        spaces: vec!["DEV".into()],
        labels: vec![],
        search_in: None,
        returned: 0,
        has_more: false,
        results: vec![],
    };

    let json = serde_json::to_value(output).unwrap();
    assert_eq!(json.get("query"), Some(&serde_json::Value::Null));
    assert_eq!(json.get("search_in"), Some(&serde_json::Value::Null));
    assert_eq!(json.get("labels"), Some(&serde_json::json!([])));
    assert_eq!(json.get("returned"), Some(&serde_json::json!(0)));
    assert_eq!(json.get("has_more"), Some(&serde_json::json!(false)));
}

#[test]
fn raw_profile_round_trip_preserves_confluence_when_jira_is_added() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let confluence_profile = ProfileConfig {
        base_url: Some("https://confluence.example.com".into()),
        api_path: Some("/wiki/rest/api".into()),
        allowed_spaces: Some(vec!["ENG".into(), "OPS".into()]),
        default_space: Some("ENG".into()),
        ..Default::default()
    };
    save_profile_to_path("team", &confluence_profile, &path).unwrap();

    let mut merged = load_profile_config_at_path("team", &path).unwrap();
    merged.jira_base_url = Some("https://jira.example.com".into());
    merged.jira_api_path = Some("/rest/api/2".into());
    merged.jira_allowed_projects = Some(vec!["ENG".into(), "OPS".into()]);
    merged.jira_default_project = Some("ENG".into());
    save_profile_to_path("team", &merged, &path).unwrap();

    let reread = load_profile_config_at_path("team", &path).unwrap();
    assert_eq!(reread.base_url, confluence_profile.base_url);
    assert_eq!(reread.api_path, confluence_profile.api_path);
    assert_eq!(reread.allowed_spaces, confluence_profile.allowed_spaces);
    assert_eq!(reread.default_space, confluence_profile.default_space);
    assert_eq!(
        reread.jira_base_url.as_deref(),
        Some("https://jira.example.com")
    );
    assert_eq!(reread.jira_api_path.as_deref(), Some("/rest/api/2"));
    assert_eq!(
        reread.jira_allowed_projects,
        Some(vec!["ENG".into(), "OPS".into()])
    );
    assert_eq!(reread.jira_default_project.as_deref(), Some("ENG"));
}

#[test]
fn raw_profile_loader_returns_empty_profile_for_missing_path_or_profile() {
    let dir = tempfile::tempdir().unwrap();
    let missing_path = dir.path().join("missing.toml");

    let missing_file = load_profile_config_at_path("team", &missing_path).unwrap();
    assert_eq!(
        serde_json::to_value(missing_file).unwrap(),
        serde_json::to_value(ProfileConfig::default()).unwrap()
    );

    let path = dir.path().join("config.toml");
    save_profile_to_path(
        "other-team",
        &ProfileConfig {
            base_url: Some("https://confluence.example.com".into()),
            ..Default::default()
        },
        &path,
    )
    .unwrap();
    let missing_profile = load_profile_config_at_path("team", &path).unwrap();
    assert_eq!(
        serde_json::to_value(missing_profile).unwrap(),
        serde_json::to_value(ProfileConfig::default()).unwrap()
    );
}

#[test]
fn confluence_search_response_deserializes_links_next() {
    let response: models::SearchResponse = serde_json::from_value(serde_json::json!({
        "results": [],
        "size": 0,
        "_links": {
            "base": "https://confluence.example.com",
            "next": "/rest/api/content/search?cursor=abc"
        }
    }))
    .unwrap();

    let links = response.links.expect("response links should be decoded");
    assert_eq!(
        links.base.as_deref(),
        Some("https://confluence.example.com")
    );
    assert_eq!(
        links.next.as_deref(),
        Some("/rest/api/content/search?cursor=abc")
    );
}

#[test]
fn search_outputs_returned_matches_results_length_for_both_backends() {
    let confluence = models::SearchOutput {
        query: Some("query".into()),
        spaces: vec![],
        labels: vec![],
        search_in: Some("title".into()),
        returned: 1,
        has_more: false,
        results: vec![models::SearchResultOutput {
            id: "123".into(),
            title: "A page".into(),
            space_key: "ENG".into(),
            space_name: "Engineering".into(),
            url: "https://confluence.example.com/pages/123".into(),
            last_modified: None,
            matched_by: vec!["title".into()],
            labels: vec![],
            excerpt: None,
        }],
    };
    let jira = models::JiraSearchOutput {
        query: Some("query".into()),
        projects: vec!["OPS".into()],
        jql: "project = OPS".into(),
        total: 1,
        returned: 1,
        has_more: false,
        results: vec![models::JiraSearchResultOutput {
            key: "OPS-1".into(),
            summary: "An issue".into(),
            status: None,
            issue_type: None,
            priority: None,
            assignee: None,
            project_key: Some("OPS".into()),
            project_name: None,
            labels: vec![],
            url: "https://jira.example.com/browse/OPS-1".into(),
            updated: None,
        }],
    };

    assert_eq!(confluence.returned as usize, confluence.results.len());
    assert_eq!(jira.returned as usize, jira.results.len());
}

#[test]
fn jira_search_result_fixture_decodes_labels_and_optional_project_name() {
    let response: models::JiraSearchResponse = serde_json::from_value(serde_json::json!({
        "total": 2,
        "issues": [
            {
                "key": "OPS-42",
                "fields": {
                    "project": {"key": "OPS", "name": "Operations"},
                    "labels": ["release", "public"]
                }
            },
            {
                "key": "OPS-43",
                "fields": {"project": {"key": "OPS"}}
            }
        ]
    }))
    .unwrap();

    assert_eq!(
        response.issues[0].fields.labels.as_ref().unwrap(),
        &vec!["release".to_string(), "public".to_string()]
    );
    assert_eq!(
        response.issues[0]
            .fields
            .project
            .as_ref()
            .and_then(|project| project.name.as_deref()),
        Some("Operations")
    );
    assert_eq!(response.issues[1].fields.labels, None);
    assert_eq!(
        response.issues[1]
            .fields
            .project
            .as_ref()
            .and_then(|project| project.name.as_deref()),
        None
    );
}

// ── Skill bundle ─────────────────────────────────────────────────────────────

use cnowledje::skill::BUNDLED_SKILLS;

#[test]
fn bundled_skills_contains_exactly_confluence_and_jira() {
    assert_eq!(BUNDLED_SKILLS.len(), 2);

    let mut names: Vec<&str> = BUNDLED_SKILLS.iter().map(|s| s.name).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["confluence-lookup", "jira-lookup"]);
}

#[test]
fn confluence_search_result_metadata_labels_are_extracted_in_api_order() {
    let json = serde_json::json!({
        "id": "123",
        "title": "Release readiness",
        "space": {"key": "ENG", "name": "Engineering"},
        "version": {"when": "2026-07-10T00:00:00Z"},
        "excerpt": null,
        "metadata": {
            "labels": {
                "results": [{"name": "api"}, {"name": "release-readiness"}]
            }
        },
        "_links": {"webui": "/pages/123"}
    });

    let result: models::SearchResult = serde_json::from_value(json).unwrap();
    assert_eq!(
        result.metadata.label_names(),
        vec!["api".to_string(), "release-readiness".to_string()]
    );
}

#[test]
fn confluence_metadata_defaults_when_response_omits_metadata() {
    let result_json = serde_json::json!({
        "id": "123",
        "title": "Release readiness",
        "space": {"key": "ENG", "name": "Engineering"},
        "version": {"when": null},
        "excerpt": null,
        "_links": {"webui": "/pages/123"}
    });
    let result: models::SearchResult = serde_json::from_value(result_json).unwrap();
    assert!(result.metadata.label_names().is_empty());

    let page_json = serde_json::json!({
        "id": "123",
        "title": "Release readiness",
        "space": {"key": "ENG", "name": "Engineering"},
        "version": {"when": null},
        "body": null,
        "_links": {"webui": "/pages/123", "base": "https://confluence.example.com"}
    });
    let page: models::PageResponse = serde_json::from_value(page_json).unwrap();
    assert!(page.metadata.label_names().is_empty());
}
#[test]
fn jira_remote_link_fixture_decodes_all_reference_fields() {
    let link: models::JiraRemoteLink = serde_json::from_value(serde_json::json!({
        "id": 42,
        "globalId": "system=confluence&object=page-123",
        "application": {
            "type": "com.atlassian.confluence",
            "name": "Confluence"
        },
        "relationship": "Wiki Page",
        "object": {
            "url": "https://confluence.example.com/wiki/spaces/ENG/pages/123",
            "title": "Release readiness",
            "summary": "Deployment checklist"
        }
    }))
    .unwrap();

    assert_eq!(link.id, 42);
    assert_eq!(
        link.global_id.as_deref(),
        Some("system=confluence&object=page-123")
    );
    let application = link.application.as_ref().unwrap();
    assert_eq!(
        application.application_type.as_deref(),
        Some("com.atlassian.confluence")
    );
    assert_eq!(application.name.as_deref(), Some("Confluence"));
    let object = link.object.as_ref().unwrap();
    assert_eq!(
        object.url,
        "https://confluence.example.com/wiki/spaces/ENG/pages/123"
    );
    assert_eq!(object.title, "Release readiness");
    assert_eq!(object.summary.as_deref(), Some("Deployment checklist"));
}

#[test]
fn jira_issue_output_serializes_confluence_references_with_expected_shape() {
    let output = models::JiraIssueOutput {
        key: "ENG-42".into(),
        summary: "Release readiness".into(),
        project_key: Some("ENG".into()),
        status: Some("Open".into()),
        issue_type: Some("Task".into()),
        priority: None,
        assignee: None,
        reporter: None,
        labels: vec![],
        created: None,
        updated: None,
        url: "https://jira.example.com/browse/ENG-42".into(),
        description_markdown: "Ready".into(),
        comments: vec![],
        omitted_comments: 0,
        confluence_references: vec![
            models::ConfluenceReferenceOutput {
                id: 123,
                title: "Release readiness".into(),
                url: "https://confluence.example.com/wiki/spaces/ENG/pages/123".into(),
                summary: Some("Deployment checklist".into()),
            },
            models::ConfluenceReferenceOutput {
                id: 124,
                title: "Runbook".into(),
                url: "https://confluence.example.com/wiki/spaces/ENG/pages/124".into(),
                summary: None,
            },
        ],
        notice: models::JIRA_NOTICE,
    };

    let json = serde_json::to_value(output).unwrap();
    assert_eq!(
        json["confluence_references"],
        serde_json::json!([
            {
                "id": 123,
                "title": "Release readiness",
                "url": "https://confluence.example.com/wiki/spaces/ENG/pages/123",
                "summary": "Deployment checklist"
            },
            {
                "id": 124,
                "title": "Runbook",
                "url": "https://confluence.example.com/wiki/spaces/ENG/pages/124",
                "summary": null
            }
        ])
    );
}
