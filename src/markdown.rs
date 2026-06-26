//! Convert Confluence storage-format HTML to Markdown.
//!
//! Confluence storage format uses a mix of standard HTML and custom
//! `ac:` / `ri:` XML namespace elements.  We pre-process those before
//! handing the result to a recursive HTML walker built on `scraper`.

use regex::Regex;
use scraper::node::Node;
use scraper::{ElementRef, Html, Selector};
use std::sync::OnceLock;

// ── Regex helpers (compiled once) ─────────────────────────────────────────────

fn macro_open_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"<ac:structured-macro[^>]*\bac:name="([^"]*)"[^>]*>"#).unwrap())
}

fn image_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<ac:image[^>]*/?>").unwrap())
}

fn ac_link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"</?ac:link[^>]*>").unwrap())
}

fn ri_page_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"<ri:page[^>]*\bri:content-title="([^"]*)"[^>]*/>"#).unwrap())
}

fn ri_attachment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"<ri:attachment[^>]*\bri:filename="([^"]*)"[^>]*/>"#).unwrap())
}

fn ac_plain_text_body_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"</?ac:plain-text-body[^>]*>").unwrap())
}

fn ac_rich_text_body_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"</?ac:rich-text-body[^>]*>").unwrap())
}

fn ac_parameter_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"</?ac:parameter[^>]*>").unwrap())
}

fn ac_default_parameter_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"</?ac:default-parameter[^>]*>").unwrap())
}

fn sv_macro_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"\bac:name\s*=\s*"sv-translation""#).unwrap())
}

fn ac_language_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"\bac:name\s*=\s*"language""#).unwrap())
}

fn escape_html_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn skip_ws(s: &str, mut i: usize, limit: usize) -> usize {
    while i < limit {
        let Some(ch) = s[i..limit].chars().next() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        i += ch.len_utf8();
    }
    i
}

fn find_language_param(
    html: &str,
    mut pos: usize,
    limit: usize,
) -> Option<(std::ops::Range<usize>, &str)> {
    const PARAM_CLOSE: &str = "</ac:parameter>";
    let lang_name_re = ac_language_name_re();

    loop {
        pos = skip_ws(html, pos, limit);

        if pos >= limit || !html[pos..limit].starts_with("<ac:parameter") {
            return None;
        }

        let tag_end = pos + html[pos..limit].find('>')? + 1;
        let close_start = tag_end + html[tag_end..limit].find(PARAM_CLOSE)?;
        let close_end = close_start + PARAM_CLOSE.len();

        if lang_name_re.is_match(&html[pos..tag_end]) {
            return Some((pos..close_end, &html[tag_end..close_start]));
        }

        pos = close_end;
    }
}

fn rewrite_sv_translation_openings(html: &str) -> String {
    let sv_name_re = sv_macro_name_re();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;

    while let Some(rel_start) = html[cursor..].find("<ac:structured-macro") {
        let start = cursor + rel_start;
        let Some(rel_open_end) = html[start..].find('>') else {
            break;
        };
        let open_end = start + rel_open_end + 1;
        let open_tag = &html[start..open_end];

        if !sv_name_re.is_match(open_tag) {
            out.push_str(&html[cursor..open_end]);
            cursor = open_end;
            continue;
        }

        let Some(rel_close) = html[open_end..].find("</ac:structured-macro>") else {
            out.push_str(&html[cursor..open_end]);
            cursor = open_end;
            continue;
        };
        let close_start = open_end + rel_close;

        if let Some((param_range, raw_lang)) = find_language_param(html, open_end, close_start) {
            let lang = escape_html_attr(raw_lang.trim());
            out.push_str(&html[cursor..start]);
            out.push_str(&format!(
                r#"<div class="ac-macro" data-macro-name="sv-translation" data-macro-language="{lang}">"#
            ));
            out.push_str(&html[open_end..param_range.start]);
            cursor = param_range.end;
        } else {
            out.push_str(&html[cursor..open_end]);
            cursor = open_end;
        }
    }

    out.push_str(&html[cursor..]);
    out
}

// ── Pre-processing ────────────────────────────────────────────────────────────

fn preprocess(html: &str) -> String {
    let s = rewrite_sv_translation_openings(html);

    let s = macro_open_re()
        .replace_all(&s, |caps: &regex::Captures| {
            format!(r#"<div class="ac-macro" data-macro-name="{}">"#, &caps[1])
        })
        .into_owned();

    let s = s.replace("</ac:structured-macro>", "</div>");

    // ac:image → [image]
    let s = image_re().replace_all(&s, "[image]").into_owned();

    // ri:page → page title as text
    let s = ri_page_re()
        .replace_all(&s, |caps: &regex::Captures| caps[1].to_string())
        .into_owned();

    // ri:attachment → filename
    let s = ri_attachment_re()
        .replace_all(&s, |caps: &regex::Captures| caps[1].to_string())
        .into_owned();

    // ac:link wrappers → pass through content
    let s = ac_link_re().replace_all(&s, "").into_owned();

    // ac:plain-text-body / ac:rich-text-body → pass through content
    let s = ac_plain_text_body_re().replace_all(&s, "").into_owned();
    let s = ac_rich_text_body_re().replace_all(&s, "").into_owned();

    // ac:parameter → pass through content
    let s = ac_parameter_re().replace_all(&s, "").into_owned();
    let s = ac_default_parameter_re().replace_all(&s, "").into_owned();

    s
}

// ── Conversion context ────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum ListKind {
    Unordered,
    Ordered(usize),
}

struct Ctx {
    list_stack: Vec<ListKind>,
    sv_translation_language: Option<String>,
    sv_translation_seen: bool,
}

impl Ctx {
    fn new(language: Option<&str>) -> Self {
        Self {
            list_stack: Vec::new(),
            sv_translation_language: language.map(str::to_owned),
            sv_translation_seen: false,
        }
    }

    fn list_depth(&self) -> usize {
        self.list_stack.len()
    }

    fn list_indent(&self) -> String {
        "  ".repeat(self.list_depth().saturating_sub(1))
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn html_to_markdown(html: &str, max_chars: usize, language: Option<&str>) -> String {
    let processed = preprocess(html);
    let document = Html::parse_fragment(&processed);

    let body_sel = Selector::parse("body").unwrap();
    let root = match document.select(&body_sel).next() {
        Some(b) => b,
        None => document.root_element(),
    };

    let mut out = String::new();
    let mut ctx = Ctx::new(language);
    convert_children(root, &mut out, &mut ctx);

    let trimmed = collapse_blank_lines(out.trim());

    if trimmed.chars().count() > max_chars {
        let mut byte_pos = 0;
        for (char_count, c) in trimmed.chars().enumerate() {
            if char_count >= max_chars {
                break;
            }
            byte_pos += c.len_utf8();
        }
        format!("{}\n\n[content truncated]", &trimmed[..byte_pos])
    } else {
        trimmed
    }
}

// ── Recursive converter ───────────────────────────────────────────────────────

fn convert_children(elem: ElementRef<'_>, out: &mut String, ctx: &mut Ctx) {
    for child in elem.children() {
        match child.value() {
            Node::Text(t) => {
                let text: &str = t.as_ref();
                // Skip whitespace-only text between block elements
                if !text.trim().is_empty() {
                    out.push_str(text);
                }
            }
            Node::Element(_) => {
                if let Some(child_elem) = ElementRef::wrap(child) {
                    convert_element(child_elem, out, ctx);
                }
            }
            _ => {}
        }
    }
}

fn convert_element(elem: ElementRef<'_>, out: &mut String, ctx: &mut Ctx) {
    let tag = elem.value().name();

    match tag {
        // Block wrappers – just recurse
        "html" | "head" | "body" | "div" | "section" | "article" | "main" | "aside" | "header"
        | "footer" | "nav" | "span" | "figure" => {
            if let Some(name) = elem.value().attr("data-macro-name") {
                if name == "sv-translation" {
                    let macro_lang = elem.value().attr("data-macro-language").unwrap_or("");
                    let should_expand = match &ctx.sv_translation_language {
                        Some(wanted) => wanted == macro_lang,
                        None => !ctx.sv_translation_seen,
                    };
                    ctx.sv_translation_seen = true;
                    if should_expand {
                        convert_children(elem, out, ctx);
                    }
                    return;
                }
                out.push_str(&format!("\n> [unsupported confluence macro: {}]\n", name));
                return;
            }
            convert_children(elem, out, ctx);
        }

        // Headings
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = tag.chars().nth(1).unwrap().to_digit(10).unwrap() as usize;
            let text = elem.text().collect::<String>();
            let text = text.trim();
            if !text.is_empty() {
                out.push('\n');
                out.push_str(&"#".repeat(level));
                out.push(' ');
                out.push_str(text);
                out.push('\n');
            }
        }

        // Paragraph
        "p" => {
            out.push('\n');
            let mut inner = String::new();
            convert_children(elem, &mut inner, ctx);
            out.push_str(inner.trim());
            out.push('\n');
        }

        // Horizontal rule
        "hr" => {
            out.push_str("\n---\n");
        }

        // Line break
        "br" => {
            out.push_str("  \n");
        }

        // Inline emphasis
        "strong" | "b" => {
            let mut inner = String::new();
            convert_children(elem, &mut inner, ctx);
            let inner = inner.trim().to_string();
            if !inner.is_empty() {
                out.push_str(&format!("**{}**", inner));
            }
        }
        "em" | "i" => {
            let mut inner = String::new();
            convert_children(elem, &mut inner, ctx);
            let inner = inner.trim().to_string();
            if !inner.is_empty() {
                out.push_str(&format!("_{}_", inner));
            }
        }
        "del" | "s" | "strike" => {
            let mut inner = String::new();
            convert_children(elem, &mut inner, ctx);
            let inner = inner.trim().to_string();
            if !inner.is_empty() {
                out.push_str(&format!("~~{}~~", inner));
            }
        }

        // Inline code
        "code" | "tt" => {
            let text = elem.text().collect::<String>();
            if !text.is_empty() {
                out.push_str(&format!("`{}`", text));
            }
        }

        // Pre / code block
        "pre" => {
            let inner_code = elem
                .select(&Selector::parse("code").unwrap())
                .next()
                .map(|c| c.text().collect::<String>())
                .unwrap_or_else(|| elem.text().collect::<String>());
            out.push_str("\n```\n");
            out.push_str(&inner_code);
            if !inner_code.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("```\n");
        }

        // Blockquote
        "blockquote" => {
            let mut inner = String::new();
            convert_children(elem, &mut inner, ctx);
            for line in inner.trim().lines() {
                out.push_str(&format!("> {}\n", line));
            }
        }

        // Anchor / link
        "a" => {
            let href = elem.value().attr("href").unwrap_or("");
            let mut inner = String::new();
            convert_children(elem, &mut inner, ctx);
            let text = inner.trim();
            if text.is_empty() && href.is_empty() {
                // nothing
            } else if text.is_empty() {
                out.push_str(&format!("<{}>", href));
            } else if href.is_empty() {
                out.push_str(text);
            } else {
                out.push_str(&format!("[{}]({})", text, href));
            }
        }

        // Images
        "img" => {
            let alt = elem.value().attr("alt").unwrap_or("");
            let src = elem.value().attr("src").unwrap_or("");
            if !src.is_empty() {
                out.push_str(&format!("![{}]({})", alt, src));
            } else if !alt.is_empty() {
                out.push_str(&format!("[{}]", alt));
            } else {
                out.push_str("[image]");
            }
        }

        // Unordered list
        "ul" => {
            out.push('\n');
            ctx.list_stack.push(ListKind::Unordered);
            convert_children(elem, out, ctx);
            ctx.list_stack.pop();
            out.push('\n');
        }

        // Ordered list
        "ol" => {
            out.push('\n');
            ctx.list_stack.push(ListKind::Ordered(0));
            convert_children(elem, out, ctx);
            ctx.list_stack.pop();
            out.push('\n');
        }

        // List item
        "li" => {
            let indent = ctx.list_indent();
            let bullet = match ctx.list_stack.last_mut() {
                Some(ListKind::Unordered) => "- ".to_string(),
                Some(ListKind::Ordered(n)) => {
                    *n += 1;
                    format!("{}. ", n)
                }
                None => "- ".to_string(),
            };
            let mut inner = String::new();
            convert_children(elem, &mut inner, ctx);
            let first_line = inner.trim().lines().next().unwrap_or("").to_string();
            out.push_str(&format!("{}{}{}\n", indent, bullet, first_line));
            // Sub-lists are handled by the nested ul/ol elements inside li.
        }

        // Definition list
        "dl" => {
            out.push('\n');
            convert_children(elem, out, ctx);
            out.push('\n');
        }
        "dt" => {
            let text = elem.text().collect::<String>();
            out.push_str(&format!("\n**{}**\n", text.trim()));
        }
        "dd" => {
            let mut inner = String::new();
            convert_children(elem, &mut inner, ctx);
            out.push_str(&format!(": {}\n", inner.trim()));
        }

        // Tables
        "table" => {
            out.push('\n');
            convert_table(elem, out, ctx);
            out.push('\n');
        }

        // Skip these silently
        "thead" | "tbody" | "tfoot" => {
            convert_children(elem, out, ctx);
        }
        "tr" => {
            // handled inside convert_table
            convert_children(elem, out, ctx);
        }
        "th" | "td" => {
            // handled inside convert_table
            convert_children(elem, out, ctx);
        }

        // sup / sub
        "sup" => {
            let text = elem.text().collect::<String>();
            out.push_str(&format!("^{}^", text.trim()));
        }
        "sub" => {
            let text = elem.text().collect::<String>();
            out.push_str(&format!("~{}~", text.trim()));
        }

        // Skip script, style, noscript
        "script" | "style" | "noscript" => {}

        // Unknown → recurse (best-effort)
        _ => {
            convert_children(elem, out, ctx);
        }
    }
}

// ── Table renderer ────────────────────────────────────────────────────────────

fn convert_table(table: ElementRef<'_>, out: &mut String, ctx: &mut Ctx) {
    let row_sel = Selector::parse("tr").unwrap();
    let cell_sel = Selector::parse("th, td").unwrap();

    let rows: Vec<Vec<String>> = table
        .select(&row_sel)
        .map(|row| {
            row.select(&cell_sel)
                .map(|cell| {
                    let mut inner = String::new();
                    convert_children(cell, &mut inner, ctx);
                    inner.trim().replace('\n', " ")
                })
                .collect()
        })
        .filter(|r: &Vec<String>| !r.is_empty())
        .collect();

    if rows.is_empty() {
        return;
    }

    // Column widths for alignment
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);

    // Detect header row (first row uses <th> cells)
    let header_row_sel = Selector::parse("tr:first-child th").unwrap();
    let has_header = table.select(&header_row_sel).next().is_some();

    for (i, row) in rows.iter().enumerate() {
        let padded: Vec<String> = (0..ncols)
            .map(|c| row.get(c).cloned().unwrap_or_default())
            .collect();
        out.push_str(&format!("| {} |\n", padded.join(" | ")));

        if i == 0 && has_header {
            // Separator row
            let sep: Vec<&str> = (0..ncols).map(|_| "---").collect();
            out.push_str(&format!("| {} |\n", sep.join(" | ")));
        }
    }
}

// ── Utility ───────────────────────────────────────────────────────────────────

fn collapse_blank_lines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut blank_count = 0usize;

    for line in s.lines() {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            result.push_str(line);
            result.push('\n');
        }
    }

    result.trim_end().to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heading() {
        let md = html_to_markdown("<h1>Hello</h1>", 50_000, None);
        assert!(md.contains("# Hello"));
    }

    #[test]
    fn test_paragraph() {
        let md = html_to_markdown("<p>Some text</p>", 50_000, None);
        assert!(md.contains("Some text"));
    }

    #[test]
    fn test_strong() {
        let md = html_to_markdown("<p><strong>bold</strong></p>", 50_000, None);
        assert!(md.contains("**bold**"));
    }

    #[test]
    fn test_link() {
        let md = html_to_markdown(r#"<a href="https://example.com">Example</a>"#, 50_000, None);
        assert!(md.contains("[Example](https://example.com)"));
    }

    #[test]
    fn test_unordered_list() {
        let md = html_to_markdown("<ul><li>A</li><li>B</li></ul>", 50_000, None);
        assert!(md.contains("- A"));
        assert!(md.contains("- B"));
    }

    #[test]
    fn test_ordered_list() {
        let md = html_to_markdown("<ol><li>First</li><li>Second</li></ol>", 50_000, None);
        assert!(md.contains("1. First"));
        assert!(md.contains("2. Second"));
    }

    #[test]
    fn test_code_inline() {
        let md = html_to_markdown("<code>let x = 1;</code>", 50_000, None);
        assert!(md.contains("`let x = 1;`"));
    }

    #[test]
    fn test_macro_placeholder() {
        let html = r#"<ac:structured-macro ac:name="jira"><ac:parameter ac:name="key">PROJ-1</ac:parameter></ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("[unsupported confluence macro: jira]"));
    }

    #[test]
    fn test_truncation() {
        let html = "<p>".to_string() + &"x".repeat(200) + "</p>";
        let md = html_to_markdown(&html, 50, None);
        assert!(md.contains("[content truncated]"));
        assert!(md.chars().count() <= 50 + "[content truncated]\n\n".len());
    }

    #[test]
    fn test_japanese_utf8() {
        let md = html_to_markdown("<p>Redisの利用方針について</p>", 50_000, None);
        assert!(md.contains("Redisの利用方針について"));
    }

    #[test]
    fn test_table() {
        let html = "<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>";
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("| A | B |"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("| 1 | 2 |"));
    }

    #[test]
    fn test_sv_translation_expand_first_when_no_language() {
        let html = r#"
<ac:structured-macro ac:name="sv-translation">
  <ac:parameter ac:name="language">ja</ac:parameter>
  <ac:rich-text-body><p>日本語コンテンツ</p></ac:rich-text-body>
</ac:structured-macro>
<ac:structured-macro ac:name="sv-translation">
  <ac:parameter ac:name="language">en</ac:parameter>
  <ac:rich-text-body><p>English content</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(
            md.contains("日本語コンテンツ"),
            "first block should be expanded"
        );
        assert!(
            !md.contains("English content"),
            "second block should not be expanded"
        );
        assert!(!md.contains("[unsupported confluence macro: sv-translation]"));
    }

    #[test]
    fn test_sv_translation_expand_by_language() {
        let html = r#"
<ac:structured-macro ac:name="sv-translation">
  <ac:parameter ac:name="language">ja</ac:parameter>
  <ac:rich-text-body><p>日本語コンテンツ</p></ac:rich-text-body>
</ac:structured-macro>
<ac:structured-macro ac:name="sv-translation">
  <ac:parameter ac:name="language">en</ac:parameter>
  <ac:rich-text-body><p>English content</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, Some("en"));
        assert!(
            !md.contains("日本語コンテンツ"),
            "ja block should not be expanded"
        );
        assert!(
            md.contains("English content"),
            "en block should be expanded"
        );
        assert!(!md.contains("[unsupported confluence macro: sv-translation]"));
    }

    #[test]
    fn test_sv_translation_no_match_language_produces_empty() {
        let html = r#"
<ac:structured-macro ac:name="sv-translation">
  <ac:parameter ac:name="language">ja</ac:parameter>
  <ac:rich-text-body><p>日本語コンテンツ</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, Some("fr"));
        assert!(
            !md.contains("日本語コンテンツ"),
            "non-matching language should not be expanded"
        );
        assert!(!md.contains("[unsupported confluence macro: sv-translation]"));
    }
}
