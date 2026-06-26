//! Integration tests for the cnowledje library.
//!
//! These tests cover the core logic without making live HTTP requests.
//! HTTP client behavior (Authorization headers, status code handling) is
//! verified by unit tests inside each module.

use cnowledje::cql;
use cnowledje::markdown;
use cnowledje::models;
use cnowledje::types::SearchIn;

// ── CQL generation ─────────────────────────────────────────────────────────────

#[test]
fn cql_title_single_space() {
    let q = cql::build_title_cql(&["DEV".to_string()], "Redis 設計");
    assert!(q.starts_with(r#"space = "DEV""#));
    assert!(q.contains("title ~"));
    assert!(q.contains("Redis 設計"));
    assert!(q.contains("ORDER BY lastmodified DESC"));
}

#[test]
fn cql_text_multiple_spaces() {
    let q = cql::build_text_cql(&["DEV".to_string(), "ARCH".to_string()], "Redis");
    assert!(q.starts_with(r#"space in ("DEV", "ARCH")"#));
    assert!(q.contains("text ~"));
}

#[test]
fn cql_escape_double_quotes_in_query() {
    let q = cql::build_title_cql(&["DEV".to_string()], r#"say "hello""#);
    assert!(q.contains(r#"say \"hello\""#));
}

#[test]
fn cql_escape_backslash_in_query() {
    let q = cql::build_title_cql(&["DEV".to_string()], r"back\slash");
    assert!(q.contains(r"back\\slash"));
}

#[test]
fn cql_both_returns_two_queries() {
    let qs = cql::build_cql_queries(&["DEV".to_string()], "test", &SearchIn::Both);
    assert_eq!(qs.len(), 2);
    assert!(matches!(qs[0].0, SearchIn::Title));
    assert!(matches!(qs[1].0, SearchIn::Text));
}

#[test]
fn cql_title_only_returns_one_query() {
    let qs = cql::build_cql_queries(&["DEV".to_string()], "test", &SearchIn::Title);
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

// ── Markdown conversion ────────────────────────────────────────────────────────

#[test]
fn markdown_heading_levels() {
    let md = markdown::html_to_markdown("<h1>H1</h1><h2>H2</h2><h3>H3</h3>", 50_000);
    assert!(md.contains("# H1"));
    assert!(md.contains("## H2"));
    assert!(md.contains("### H3"));
}

#[test]
fn markdown_bold_and_italic() {
    let md = markdown::html_to_markdown("<strong>bold</strong> and <em>italic</em>", 50_000);
    assert!(md.contains("**bold**"));
    assert!(md.contains("_italic_"));
}

#[test]
fn markdown_link_with_href() {
    let md = markdown::html_to_markdown(
        r#"<a href="https://example.com">Example</a>"#,
        50_000,
    );
    assert!(md.contains("[Example](https://example.com)"));
}

#[test]
fn markdown_unordered_list() {
    let md = markdown::html_to_markdown(
        "<ul><li>Alpha</li><li>Beta</li><li>Gamma</li></ul>",
        50_000,
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
    );
    assert!(md.contains("1. First"));
    assert!(md.contains("2. Second"));
    assert!(md.contains("3. Third"));
}

#[test]
fn markdown_inline_code() {
    let md = markdown::html_to_markdown("<code>let x = 42;</code>", 50_000);
    assert!(md.contains("`let x = 42;`"));
}

#[test]
fn markdown_preformatted_block() {
    let md = markdown::html_to_markdown("<pre><code>fn main() {}</code></pre>", 50_000);
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
    let md = markdown::html_to_markdown(html, 50_000);
    assert!(md.contains("| Name | Value |"));
    assert!(md.contains("| --- | --- |"));
    assert!(md.contains("| foo | 1 |"));
    assert!(md.contains("| bar | 2 |"));
}

#[test]
fn markdown_confluence_macro_placeholder() {
    let html = r#"<ac:structured-macro ac:name="jira"><ac:parameter ac:name="key">PROJ-1</ac:parameter></ac:structured-macro>"#;
    let md = markdown::html_to_markdown(html, 50_000);
    assert!(
        md.contains("[unsupported confluence macro: jira]"),
        "got: {}",
        md
    );
}

#[test]
fn markdown_confluence_info_macro_placeholder() {
    let html = r#"<ac:structured-macro ac:name="info"><ac:rich-text-body><p>Note here</p></ac:rich-text-body></ac:structured-macro>"#;
    let md = markdown::html_to_markdown(html, 50_000);
    assert!(md.contains("[unsupported confluence macro: info]"), "got: {}", md);
}

#[test]
fn markdown_japanese_utf8() {
    let md = markdown::html_to_markdown("<p>Redisの利用方針について説明します。</p>", 50_000);
    assert!(md.contains("Redisの利用方針について説明します。"));
}

#[test]
fn markdown_truncation_adds_notice() {
    let html = "<p>".to_string() + &"あ".repeat(300) + "</p>";
    let md = markdown::html_to_markdown(&html, 50);
    assert!(md.contains("[content truncated]"), "got: {}", md);
}

#[test]
fn markdown_no_truncation_when_short() {
    let md = markdown::html_to_markdown("<p>Short content.</p>", 50_000);
    assert!(!md.contains("[content truncated]"));
    assert!(md.contains("Short content."));
}

// ── Output model constants ─────────────────────────────────────────────────────

#[test]
fn notice_string_present() {
    assert!(models::NOTICE.contains("reference material"));
    assert!(models::NOTICE.contains("instructions"));
}

// ── format helpers ─────────────────────────────────────────────────────────────

use cnowledje::format::make_page_url;

#[test]
fn make_page_url_with_api_base() {
    let url = make_page_url(
        "https://config-base.example.com",
        Some("https://api-base.example.com"),
        Some("/pages/viewpage.action?pageId=1"),
    );
    assert_eq!(url, "https://api-base.example.com/pages/viewpage.action?pageId=1");
}

#[test]
fn make_page_url_fallback_to_config_base() {
    let url = make_page_url(
        "https://confluence.example.local",
        None,
        Some("/pages/viewpage.action?pageId=2"),
    );
    assert_eq!(url, "https://confluence.example.local/pages/viewpage.action?pageId=2");
}

#[test]
fn make_page_url_no_webui() {
    let url = make_page_url("https://confluence.example.local", None, None);
    assert_eq!(url, "https://confluence.example.local");
}
