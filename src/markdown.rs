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

fn ri_content_title_attr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"ri:content-title="([^"]*)""#).unwrap())
}

fn ri_space_key_attr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"ri:space-key="([^"]*)""#).unwrap())
}

fn ac_plain_text_body_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"</?ac:plain-text-body[^>]*>").unwrap())
}

fn ac_rich_text_body_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"</?ac:rich-text-body[^>]*>").unwrap())
}

/// Matches an `<ac:parameter>` element including its content (non-greedy).
/// Used in the final cleanup pass to remove any parameters not consumed by
/// `rewrite_macros()` (e.g. parameters that appear after the rich-text-body).
fn ac_param_with_content_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<ac:parameter[^>]*>.*?</ac:parameter>").unwrap())
}

fn ac_default_param_with_content_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?s)<ac:default-parameter[^>]*>.*?</ac:default-parameter>").unwrap()
    })
}

/// Matches a CDATA-wrapped `<ac:plain-text-body>` element.
fn plain_text_body_cdata_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?s)<ac:plain-text-body[^>]*>\s*<!\[CDATA\[(.*?)\]\]>\s*</ac:plain-text-body>")
            .unwrap()
    })
}

/// Matches the `ac:name` attribute inside a parameter open-tag.
fn ac_param_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"\bac:name="([^"]*)""#).unwrap())
}

/// Matches any remaining HTML/XML tag (used to strip tags from param values).
fn html_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<[^>]*>").unwrap())
}

// ── Selector helpers (compiled once) ─────────────────────────────────────────

fn body_sel() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| Selector::parse("body").unwrap())
}

fn code_sel() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| Selector::parse("code").unwrap())
}

fn pre_sel() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| Selector::parse("pre").unwrap())
}

fn tr_sel() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| Selector::parse("tr").unwrap())
}

fn th_td_sel() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| Selector::parse("th, td").unwrap())
}

fn first_row_th_sel() -> &'static Selector {
    static SEL: OnceLock<Selector> = OnceLock::new();
    SEL.get_or_init(|| Selector::parse("tr:first-child th").unwrap())
}

// ── Escaping helpers ──────────────────────────────────────────────────────────

/// Full HTML text escaping (`&`, `<`, `>`). Used for raw (CDATA) code content.
fn escape_html_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Minimal escaping for parameter values that are already XML-entity-encoded.
/// Only `"` needs to be escaped so the value is safe inside a double-quoted
/// HTML attribute.  Re-encoding `&` would cause double-escaping.
fn escape_attr_preserving_entities(value: &str) -> String {
    value.replace('"', "&quot;")
}

/// Normalise a parameter name to a safe HTML attribute name fragment.
/// Empty string (for `<ac:default-parameter>`) becomes `"default"`.
fn sanitize_param_name(name: &str) -> String {
    if name.is_empty() {
        return "default".to_string();
    }
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Clean a raw parameter value for use as an HTML attribute value.
/// Resolves `ri:page`/`ri:attachment` references, strips remaining tags,
/// and escapes the result.
fn clean_param_value(raw: &str) -> String {
    let s = ri_page_re()
        .replace_all(raw, |caps: &regex::Captures| caps[1].to_string())
        .into_owned();
    let s = ri_attachment_re()
        .replace_all(&s, |caps: &regex::Captures| caps[1].to_string())
        .into_owned();
    let s = ac_link_re().replace_all(&s, "").into_owned();
    let s = html_tag_re().replace_all(&s, "").into_owned();
    escape_attr_preserving_entities(s.trim())
}

// ── Whitespace helper (re-used by consume_params) ────────────────────────────

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

// ── Parameter consumer ────────────────────────────────────────────────────────

/// Starting at `pos` in `html`, consume any leading `<ac:parameter>` and
/// `<ac:default-parameter>` elements, returning `(name, value)` pairs and
/// the position immediately after the last consumed element.
fn consume_params(html: &str, pos: usize) -> (Vec<(String, String)>, usize) {
    let mut params = Vec::new();
    let mut cursor = pos;

    loop {
        cursor = skip_ws(html, cursor, html.len());

        let rest = &html[cursor..];
        let (is_param, is_default) = (
            rest.starts_with("<ac:parameter"),
            rest.starts_with("<ac:default-parameter"),
        );
        if !is_param && !is_default {
            break;
        }

        let close_tag = if is_default {
            "</ac:default-parameter>"
        } else {
            "</ac:parameter>"
        };

        // Note: ac:name attribute values are simple identifiers and cannot
        // contain '>'.  If that invariant ever breaks, tag_end would land
        // inside the attribute value, producing a garbage open_tag_content.
        let Some(rel_gt) = rest.find('>') else {
            break;
        };
        let open_tag_content = &rest[..rel_gt]; // everything before '>'
        let tag_end = cursor + rel_gt + 1;

        let Some(rel_close) = html[tag_end..].find(close_tag) else {
            // Malformed: parameter has no matching close tag.  Advance past
            // the open tag to avoid emitting the raw XML tag into output,
            // then stop consuming parameters.
            cursor = tag_end;
            break;
        };
        let value_end = tag_end + rel_close;
        let close_end = value_end + close_tag.len();

        let param_name = if let Some(caps) = ac_param_name_re().captures(open_tag_content) {
            sanitize_param_name(&caps[1])
        } else if is_default {
            "default".to_string()
        } else {
            sanitize_param_name("")
        };

        let raw_value = &html[tag_end..value_end];
        let value = clean_param_value(raw_value);

        params.push((param_name, value));
        cursor = close_end;
    }

    (params, cursor)
}

// ── Pre-processing ────────────────────────────────────────────────────────────

/// Macros that should become `<span>` rather than `<div>` so that they do not
/// break open a surrounding `<p>` element when html5ever parses the result.
const INLINE_MACROS: &[&str] = &["status", "anchor"];

/// Convert `<ac:plain-text-body>` elements to `<pre>` so that the code
/// content survives html5ever parsing.
///
/// html5ever treats `<![CDATA[...]]>` as a bogus comment (content is lost).
/// This function must run **before** `rewrite_macros()` so the protected
/// content is not altered by later regex passes.
///
/// Known limitation: CDATA sequences split with `]]]]><![CDATA[>` are not
/// supported.
fn rewrite_plain_text_bodies(html: &str) -> String {
    // Pass 1: CDATA variant — content is raw text, needs full HTML escaping.
    let s = plain_text_body_cdata_re()
        .replace_all(html, |caps: &regex::Captures| {
            let content = escape_html_text(&caps[1]);
            // The leading '\n' is intentional: html5ever eats the first newline
            // immediately after a <pre> open tag per the HTML5 spec.  Our
            // artificial newline is consumed, leaving the real content intact.
            format!("<pre class=\"ac-plain-text-body\">\n{content}</pre>")
        })
        .into_owned();

    // Pass 2: non-CDATA variant — content is already entity-encoded.
    ac_plain_text_body_re()
        .replace_all(&s, |caps: &regex::Captures| {
            if caps[0].starts_with("</") {
                "</pre>".to_string()
            } else {
                "<pre class=\"ac-plain-text-body\">\n".to_string()
            }
        })
        .into_owned()
}

/// Rewrite every `<ac:structured-macro>` into an HTML `<div>` (or `<span>`
/// for inline macros), lifting `<ac:parameter>` children into `data-macro-param-*`
/// attributes.
///
/// The scanner finds the next opening or closing macro tag from the current
/// cursor position and processes whichever comes first, so nested macros
/// (e.g. expand containing code) are handled correctly via a div/span stack.
///
/// Known limitation: parameter values that themselves contain the literal
/// string `<ac:structured-macro` are not supported (extremely rare in practice).
fn rewrite_macros(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + html.len() / 4);
    let mut cursor = 0;
    // Stack tracks what kind of element (div/span) was opened at each nesting
    // level so the matching close tag can be emitted.
    let mut stack: Vec<&'static str> = Vec::new();

    loop {
        let rest = &html[cursor..];
        let open_pos = rest.find("<ac:structured-macro").map(|p| cursor + p);
        let close_pos = rest.find("</ac:structured-macro>").map(|p| cursor + p);

        match (open_pos, close_pos) {
            (None, None) => {
                out.push_str(&html[cursor..]);
                break;
            }
            (Some(op), None) => {
                // No close tags remain. Process the open tag if it is
                // self-closing; otherwise copy the rest and stop.
                let Some(rel_gt) = html[op..].find('>') else {
                    out.push_str(&html[cursor..]);
                    break;
                };
                let tag_end = op + rel_gt + 1;
                let open_tag = &html[op..tag_end];

                let is_self_closing = open_tag.ends_with("/>")
                    || open_tag
                        .get(..open_tag.len().saturating_sub(1))
                        .is_some_and(|s| s.trim_end().ends_with('/'));

                if !is_self_closing {
                    out.push_str(&html[cursor..]);
                    break;
                }

                out.push_str(&html[cursor..op]);

                let macro_name = macro_open_re()
                    .captures(open_tag)
                    .map(|c| c[1].to_string())
                    .unwrap_or_default();

                // Self-closing macros have no parameter children; calling
                // consume_params here could accidentally eat stray
                // <ac:parameter> tags from malformed documents.
                let kind: &'static str = if INLINE_MACROS.contains(&macro_name.as_str()) {
                    "span"
                } else {
                    "div"
                };

                let tag = format!(
                    r#"<{kind} class="ac-macro" data-macro-name="{}"></{kind}>"#,
                    escape_attr_preserving_entities(&macro_name)
                );
                out.push_str(&tag);
                cursor = tag_end;
                // Continue the loop — more content may follow.
            }
            (None, Some(cp)) => {
                // Only a close tag remains.
                out.push_str(&html[cursor..cp]);
                let kind = stack.pop().unwrap_or("div");
                out.push_str(if kind == "span" { "</span>" } else { "</div>" });
                cursor = cp + "</ac:structured-macro>".len();
            }
            (Some(op), Some(cp)) if cp < op => {
                // Close tag comes before the next open tag.
                out.push_str(&html[cursor..cp]);
                let kind = stack.pop().unwrap_or("div");
                out.push_str(if kind == "span" { "</span>" } else { "</div>" });
                cursor = cp + "</ac:structured-macro>".len();
            }
            (Some(op), _) => {
                // Open tag comes first.
                out.push_str(&html[cursor..op]);

                let Some(rel_gt) = html[op..].find('>') else {
                    // Malformed: no closing '>'; copy rest and stop.
                    out.push_str(&html[cursor..]);
                    break;
                };
                let tag_end = op + rel_gt + 1;
                let open_tag = &html[op..tag_end];

                let macro_name = macro_open_re()
                    .captures(open_tag)
                    .map(|c| c[1].to_string())
                    .unwrap_or_default();

                let is_self_closing = open_tag.ends_with("/>")
                    || open_tag
                        .get(..open_tag.len().saturating_sub(1))
                        .is_some_and(|s| s.trim_end().ends_with('/'));

                let kind: &'static str = if INLINE_MACROS.contains(&macro_name.as_str()) {
                    "span"
                } else {
                    "div"
                };

                let mut tag = format!(
                    r#"<{kind} class="ac-macro" data-macro-name="{}""#,
                    escape_attr_preserving_entities(&macro_name)
                );

                if is_self_closing {
                    // Self-closing macros have no parameter children; skip
                    // consume_params to avoid eating stray tags from malformed
                    // documents.
                    tag.push_str(&format!("></{kind}>"));
                    out.push_str(&tag);
                    cursor = tag_end;
                } else {
                    let (params, after_params) = consume_params(html, tag_end);
                    for (k, v) in &params {
                        tag.push_str(&format!(r#" data-macro-param-{k}="{v}""#));
                    }
                    tag.push('>');
                    out.push_str(&tag);
                    stack.push(kind);
                    cursor = after_params;
                }
            }
        }
    }

    out
}

fn preprocess(html: &str) -> String {
    // ① CDATA / plain-text-body → <pre> (must be first to protect code from
    //   later regex passes).
    let s = rewrite_plain_text_bodies(html);

    // ② <ac:structured-macro> → <div|span class="ac-macro" ...>
    let s = rewrite_macros(&s);

    // ③ Inline ac: / ri: elements.
    let s = image_re().replace_all(&s, "[image]").into_owned();
    let s = ri_page_re()
        .replace_all(&s, |caps: &regex::Captures| caps[1].to_string())
        .into_owned();
    let s = ri_attachment_re()
        .replace_all(&s, |caps: &regex::Captures| caps[1].to_string())
        .into_owned();
    let s = ac_link_re().replace_all(&s, "").into_owned();

    // ④ Strip remaining wrapper tags and any parameters not consumed by
    //   rewrite_macros (e.g. parameters that appear after the rich-text-body).
    let s = ac_rich_text_body_re().replace_all(&s, "").into_owned();
    let s = ac_param_with_content_re().replace_all(&s, "").into_owned();
    let s = ac_default_param_with_content_re()
        .replace_all(&s, "")
        .into_owned();

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
    /// Resolved page IDs for `excerpt-include`/`excerpt-includeplus` macros,
    /// one per occurrence in document order (see [`extract_excerpt_refs`]).
    excerpt_ids: Vec<Option<String>>,
    excerpt_index: usize,
}

impl Ctx {
    fn new(language: Option<&str>, excerpt_ids: Vec<Option<String>>) -> Self {
        Self {
            list_stack: Vec::new(),
            sv_translation_language: language.map(str::to_owned),
            sv_translation_seen: false,
            excerpt_ids,
            excerpt_index: 0,
        }
    }

    /// Consume the next resolved excerpt-include page ID, in document order.
    fn next_excerpt_id(&mut self) -> Option<String> {
        let id = self.excerpt_ids.get(self.excerpt_index).cloned().flatten();
        self.excerpt_index += 1;
        id
    }

    fn list_depth(&self) -> usize {
        self.list_stack.len()
    }

    fn list_indent(&self) -> String {
        "  ".repeat(self.list_depth().saturating_sub(1))
    }
}

// ── Cross-page excerpt reference extraction ────────────────────────────────────

/// A page referenced by an `excerpt-include`/`excerpt-includeplus` macro.
///
/// The Confluence storage format never carries a page ID for `ri:page`
/// references (only `ri:content-title` and, for cross-space references,
/// `ri:space-key`), so resolving one requires a separate API lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptRef {
    pub title: String,
    pub space_key: Option<String>,
}

/// Scan raw (pre-conversion) storage HTML for `excerpt-include`/
/// `excerpt-includeplus` macros and return their referenced page in
/// document order. Callers can resolve each to a page ID (e.g. via a CQL
/// title search) and feed the results back into
/// [`html_to_markdown_with_excerpt_ids`].
pub fn extract_excerpt_refs(html: &str) -> Vec<ExcerptRef> {
    let mut refs = Vec::new();
    let mut cursor = 0;

    while let Some(rel) = html[cursor..].find("<ac:structured-macro") {
        let open_pos = cursor + rel;
        let Some(rel_gt) = html[open_pos..].find('>') else {
            break;
        };
        let tag_end = open_pos + rel_gt + 1;
        let open_tag = &html[open_pos..tag_end];

        let is_excerpt = macro_open_re()
            .captures(open_tag)
            .is_some_and(|c| matches!(&c[1], "excerpt-include" | "excerpt-includeplus"));

        let close_tag = "</ac:structured-macro>";
        let (body, next_cursor) = match html[tag_end..].find(close_tag) {
            Some(r) => (&html[tag_end..tag_end + r], tag_end + r + close_tag.len()),
            None => (&html[tag_end..], html.len()),
        };

        if is_excerpt {
            // Always push exactly one entry per excerpt-include occurrence
            // (even on extraction failure) so this list stays positionally
            // aligned with the `next_excerpt_id()` calls made while walking
            // the converted DOM in `convert_macro`.
            let page_tag = body
                .find("<ri:page")
                .and_then(|page_rel| body[page_rel..].find('>').map(|g| (page_rel, g)))
                .map(|(page_rel, page_gt)| &body[page_rel..page_rel + page_gt + 1]);

            let title = page_tag
                .and_then(|t| ri_content_title_attr_re().captures(t))
                .map(|c| c[1].to_string())
                .unwrap_or_default();
            let space_key = page_tag
                .and_then(|t| ri_space_key_attr_re().captures(t))
                .map(|c| c[1].to_string());

            refs.push(ExcerptRef { title, space_key });
        }

        cursor = next_cursor;
    }

    refs
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn html_to_markdown(html: &str, max_chars: usize, language: Option<&str>) -> String {
    html_to_markdown_with_excerpt_ids(html, max_chars, language, &[])
}

/// Like [`html_to_markdown`], but injects already-resolved page IDs into
/// `excerpt-include`/`excerpt-includeplus` placeholders. `excerpt_ids` must
/// line up positionally with [`extract_excerpt_refs`] run over the same
/// `html`; `None` (or a short slice) falls back to the title-only placeholder.
pub fn html_to_markdown_with_excerpt_ids(
    html: &str,
    max_chars: usize,
    language: Option<&str>,
    excerpt_ids: &[Option<String>],
) -> String {
    let processed = preprocess(html);
    let document = Html::parse_fragment(&processed);

    let root = match document.select(body_sel()).next() {
        Some(b) => b,
        None => document.root_element(),
    };

    let mut out = String::new();
    let mut ctx = Ctx::new(language, excerpt_ids.to_vec());
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
        // Block wrappers – check for macro attributes first, then recurse.
        "html" | "head" | "body" | "div" | "section" | "article" | "main" | "aside" | "header"
        | "footer" | "nav" | "figure" => {
            if let Some(name) = elem.value().attr("data-macro-name") {
                convert_macro(elem, name, out, ctx);
                return;
            }
            convert_children(elem, out, ctx);
        }

        // Inline elements that may carry macro attributes (status, anchor).
        "span" => {
            if let Some(name) = elem.value().attr("data-macro-name") {
                convert_macro(elem, name, out, ctx);
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
                .select(code_sel())
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

// ── Macro dispatcher ──────────────────────────────────────────────────────────

/// Emit a blockquote block from `text`, one `> ` prefix per line.
fn quote_lines(text: &str, out: &mut String) {
    for line in text.lines() {
        out.push_str(&format!("> {}\n", line));
    }
}

fn convert_macro(elem: ElementRef<'_>, name: &str, out: &mut String, ctx: &mut Ctx) {
    match name {
        // ── Scroll Versions translation block ─────────────────────────────────
        "sv-translation" => {
            let macro_lang = elem.value().attr("data-macro-param-language").unwrap_or("");
            let should_expand = match &ctx.sv_translation_language {
                Some(wanted) => wanted == macro_lang,
                None => !ctx.sv_translation_seen,
            };
            ctx.sv_translation_seen = true;
            if should_expand {
                convert_children(elem, out, ctx);
            }
        }

        // ── Expand (collapsible section) ──────────────────────────────────────
        "expand" => {
            let title = elem
                .value()
                .attr("data-macro-param-title")
                .filter(|t| !t.is_empty())
                .unwrap_or("Expand");
            out.push('\n');
            out.push_str(&format!("**▸ {}**", title));
            out.push('\n');
            convert_children(elem, out, ctx);
        }

        // ── Code block ────────────────────────────────────────────────────────
        "code" => {
            let lang = elem.value().attr("data-macro-param-language").unwrap_or("");
            let code = elem
                .select(pre_sel())
                .next()
                .map(|p| p.text().collect::<String>())
                .unwrap_or_else(|| elem.text().collect::<String>());
            out.push('\n');
            out.push_str(&format!("```{}\n", lang));
            out.push_str(&code);
            if !code.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("```\n");
        }

        // ── No-format block ───────────────────────────────────────────────────
        "noformat" => {
            let code = elem
                .select(pre_sel())
                .next()
                .map(|p| p.text().collect::<String>())
                .unwrap_or_else(|| elem.text().collect::<String>());
            out.push('\n');
            out.push_str("```\n");
            out.push_str(&code);
            if !code.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("```\n");
        }

        // ── Info / Note / Warning / Tip ───────────────────────────────────────
        "info" | "note" | "warning" | "tip" => {
            let label = {
                let mut l = name[..1].to_uppercase();
                l.push_str(&name[1..]);
                l
            };
            let mut inner = String::new();
            convert_children(elem, &mut inner, ctx);
            out.push('\n');
            out.push_str(&format!("> **{}:**\n", label));
            quote_lines(inner.trim(), out);
            out.push('\n');
        }

        // ── Panel ─────────────────────────────────────────────────────────────
        "panel" => {
            let title = elem.value().attr("data-macro-param-title").unwrap_or("");
            let mut inner = String::new();
            convert_children(elem, &mut inner, ctx);
            out.push('\n');
            if !title.is_empty() {
                out.push_str(&format!("> **{}**\n", title));
            }
            quote_lines(inner.trim(), out);
            out.push('\n');
        }

        // ── Status badge (inline) ─────────────────────────────────────────────
        "status" => {
            let title = elem.value().attr("data-macro-param-title").unwrap_or("");
            if !title.is_empty() {
                out.push_str(&format!("[{}]", title));
            }
        }

        // ── Table of contents ─────────────────────────────────────────────────
        "toc" => {
            out.push_str("\n[TOC]\n");
        }

        // ── Anchor (page anchor — no visible output) ──────────────────────────
        "anchor" => {
            // Intentionally empty.
        }

        // ── Cross-page excerpt include ────────────────────────────────────────
        // We can only fetch the current page; cross-page inclusion is not
        // possible in this read-only, single-page design.
        "excerpt-include" | "excerpt-includeplus" => {
            let page = elem
                .value()
                .attr("data-macro-param-default")
                .or_else(|| elem.value().attr("data-macro-param-page"))
                .or_else(|| elem.value().attr("data-macro-param-name"))
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown page");
            match ctx.next_excerpt_id() {
                Some(id) => out.push_str(&format!("\n> [excerpt from: {} (id: {})]\n", page, id)),
                None => out.push_str(&format!("\n> [excerpt from: {}]\n", page)),
            }
        }

        // ── Unsupported ───────────────────────────────────────────────────────
        _ => {
            // Show any hoisted parameter values as context so that useful
            // data (e.g. a JIRA ticket key) is not silently dropped.
            let params: Vec<String> = elem
                .value()
                .attrs()
                .filter(|(k, _)| k.starts_with("data-macro-param-"))
                .map(|(k, v)| format!("{}: {}", k.trim_start_matches("data-macro-param-"), v))
                .collect();

            if params.is_empty() {
                out.push_str(&format!("\n> [unsupported confluence macro: {}]\n", name));
            } else {
                out.push_str(&format!(
                    "\n> [unsupported confluence macro: {} ({})]\n",
                    name,
                    params.join(", ")
                ));
            }

            // Also render body content if the macro has a rich-text-body.
            let mut inner = String::new();
            convert_children(elem, &mut inner, ctx);
            let inner = inner.trim();
            if !inner.is_empty() {
                out.push('\n');
                quote_lines(inner, out);
                out.push('\n');
            }
        }
    }
}

// ── Table renderer ────────────────────────────────────────────────────────────

fn convert_table(table: ElementRef<'_>, out: &mut String, ctx: &mut Ctx) {
    let rows: Vec<Vec<String>> = table
        .select(tr_sel())
        .map(|row| {
            row.select(th_td_sel())
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
    let has_header = table.select(first_row_th_sel()).next().is_some();

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
        // Parameter values are now shown in the placeholder for visibility.
        assert!(
            md.contains("[unsupported confluence macro: jira"),
            "placeholder: {md}"
        );
        assert!(md.contains("PROJ-1"), "param value should appear: {md}");
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

    // ── Expand macro ──────────────────────────────────────────────────────────

    #[test]
    fn test_expand_macro_with_title() {
        let html = r#"<ac:structured-macro ac:name="expand">
  <ac:parameter ac:name="title">Click to expand</ac:parameter>
  <ac:rich-text-body><p>Hidden content here.</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("**▸ Click to expand**"), "title should appear");
        assert!(
            md.contains("Hidden content here."),
            "body should be expanded"
        );
        assert!(!md.contains("[unsupported confluence macro: expand]"));
    }

    #[test]
    fn test_expand_macro_no_title_uses_default() {
        let html = r#"<ac:structured-macro ac:name="expand">
  <ac:rich-text-body><p>Body text.</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(
            md.contains("**▸ Expand**"),
            "default title 'Expand' should be used"
        );
        assert!(md.contains("Body text."));
    }

    // ── Code macro ────────────────────────────────────────────────────────────

    #[test]
    fn test_code_macro_with_language() {
        let html = r#"<ac:structured-macro ac:name="code">
  <ac:parameter ac:name="language">rust</ac:parameter>
  <ac:plain-text-body><![CDATA[fn main() {}]]></ac:plain-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("```rust"), "language fence should appear");
        assert!(md.contains("fn main() {}"), "code body should appear");
        assert!(md.contains("```"), "closing fence should appear");
    }

    #[test]
    fn test_code_macro_cdata_special_chars() {
        let html = r#"<ac:structured-macro ac:name="code">
  <ac:parameter ac:name="language">html</ac:parameter>
  <ac:plain-text-body><![CDATA[<div class="a" & b>text</div>]]></ac:plain-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(
            md.contains(r#"<div class="a" & b>text</div>"#),
            "HTML special chars in CDATA should be preserved: {md}"
        );
    }

    #[test]
    fn test_code_macro_extra_params_do_not_leak() {
        let html = r#"<ac:structured-macro ac:name="code">
  <ac:parameter ac:name="language">java</ac:parameter>
  <ac:parameter ac:name="title">My Snippet</ac:parameter>
  <ac:parameter ac:name="linenumbers">true</ac:parameter>
  <ac:plain-text-body><![CDATA[System.out.println("hi");]]></ac:plain-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(
            !md.contains("My Snippet"),
            "extra params must not appear in output"
        );
        assert!(
            !md.contains("true"),
            "linenumbers param must not appear in output"
        );
        assert!(md.contains("```java"));
        assert!(md.contains(r#"System.out.println("hi");"#));
    }

    // ── Noformat macro ────────────────────────────────────────────────────────

    #[test]
    fn test_noformat_macro() {
        let html = r#"<ac:structured-macro ac:name="noformat">
  <ac:plain-text-body><![CDATA[plain text block
second line]]></ac:plain-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("```\n"), "opening fence without language");
        assert!(md.contains("plain text block"));
        assert!(md.contains("second line"));
    }

    // ── Info / note / warning / tip ───────────────────────────────────────────

    #[test]
    fn test_info_macro() {
        let html = r#"<ac:structured-macro ac:name="info">
  <ac:rich-text-body><p>Important note here.</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("> **Info:**"), "Info label should appear");
        assert!(md.contains("Important note here."), "body should appear");
        assert!(!md.contains("[unsupported confluence macro: info]"));
    }

    #[test]
    fn test_warning_macro() {
        let html = r#"<ac:structured-macro ac:name="warning">
  <ac:rich-text-body><p>Danger!</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("> **Warning:**"));
        assert!(md.contains("Danger!"));
    }

    #[test]
    fn test_note_and_tip_macros() {
        let note_md = html_to_markdown(
            r#"<ac:structured-macro ac:name="note"><ac:rich-text-body><p>A note.</p></ac:rich-text-body></ac:structured-macro>"#,
            50_000,
            None,
        );
        assert!(note_md.contains("> **Note:**"));
        let tip_md = html_to_markdown(
            r#"<ac:structured-macro ac:name="tip"><ac:rich-text-body><p>A tip.</p></ac:rich-text-body></ac:structured-macro>"#,
            50_000,
            None,
        );
        assert!(tip_md.contains("> **Tip:**"));
    }

    // ── Panel macro ───────────────────────────────────────────────────────────

    #[test]
    fn test_panel_macro_with_title() {
        let html = r#"<ac:structured-macro ac:name="panel">
  <ac:parameter ac:name="title">Panel Title</ac:parameter>
  <ac:rich-text-body><p>Panel body.</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("> **Panel Title**"));
        assert!(md.contains("Panel body."));
    }

    #[test]
    fn test_panel_macro_without_title() {
        let html = r#"<ac:structured-macro ac:name="panel">
  <ac:rich-text-body><p>No title panel.</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("No title panel."));
        // No title line should appear
        assert!(!md.contains("> **"));
    }

    // ── Status macro (inline) ─────────────────────────────────────────────────

    #[test]
    fn test_status_macro_inline_no_paragraph_break() {
        // status is a <span> so it must not break the surrounding <p>
        let html = r#"<p>Status is <ac:structured-macro ac:name="status"><ac:parameter ac:name="title">DONE</ac:parameter></ac:structured-macro> today.</p>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("[DONE]"), "status badge should appear");
        // The whole paragraph should be on a single (logical) line
        let lines: Vec<&str> = md.lines().filter(|l| !l.trim().is_empty()).collect();
        let para_line = lines
            .iter()
            .find(|l| l.contains("[DONE]"))
            .expect("no line with [DONE]");
        assert!(
            para_line.contains("Status is"),
            "surrounding text should be on same line: {para_line}"
        );
        assert!(
            para_line.contains("today."),
            "trailing text should be on same line: {para_line}"
        );
    }

    // ── TOC macro ─────────────────────────────────────────────────────────────

    #[test]
    fn test_toc_macro() {
        let html = r#"<ac:structured-macro ac:name="toc" ac:schema-version="1"/>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("[TOC]"));
        assert!(!md.contains("[unsupported confluence macro: toc]"));
    }

    // ── Anchor macro ──────────────────────────────────────────────────────────

    #[test]
    fn test_anchor_macro_silent() {
        let html = r#"<p>Before</p><ac:structured-macro ac:name="anchor"><ac:parameter ac:name="default">my-anchor</ac:parameter></ac:structured-macro><p>After</p>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("Before"), "before text should appear");
        assert!(md.contains("After"), "after text should appear");
        assert!(!md.contains("my-anchor"), "anchor name should not appear");
        assert!(!md.contains("[unsupported confluence macro: anchor]"));
    }

    // ── Excerpt-include macro ─────────────────────────────────────────────────

    #[test]
    fn test_excerpt_includeplus_placeholder() {
        // The page reference is in a default parameter containing ac:link/ri:page
        let html = r#"<ac:structured-macro ac:name="excerpt-includeplus">
  <ac:default-parameter><ac:link><ri:page ri:content-title="Source Page"/></ac:link></ac:default-parameter>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(
            md.contains("[excerpt from: Source Page]"),
            "page name should appear in placeholder: {md}"
        );
    }

    #[test]
    fn test_extract_excerpt_refs_title_and_space() {
        let html = r#"<ac:structured-macro ac:name="excerpt-include">
  <ac:default-parameter><ac:link><ri:page ri:space-key="DEV" ri:content-title="Source Page"/></ac:link></ac:default-parameter>
</ac:structured-macro>"#;
        let refs = extract_excerpt_refs(html);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].title, "Source Page");
        assert_eq!(refs[0].space_key.as_deref(), Some("DEV"));
    }

    #[test]
    fn test_extract_excerpt_refs_no_space_key() {
        let html = r#"<ac:structured-macro ac:name="excerpt-includeplus">
  <ac:default-parameter><ac:link><ri:page ri:content-title="Source Page"/></ac:link></ac:default-parameter>
</ac:structured-macro>"#;
        let refs = extract_excerpt_refs(html);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].title, "Source Page");
        assert_eq!(refs[0].space_key, None);
    }

    #[test]
    fn test_extract_excerpt_refs_ignores_other_macros() {
        let html = r#"<ac:structured-macro ac:name="info"><ac:rich-text-body><p>hi</p></ac:rich-text-body></ac:structured-macro>"#;
        assert!(extract_excerpt_refs(html).is_empty());
    }

    #[test]
    fn test_excerpt_include_with_resolved_id() {
        let html = r#"<ac:structured-macro ac:name="excerpt-include">
  <ac:default-parameter><ac:link><ri:page ri:content-title="Source Page"/></ac:link></ac:default-parameter>
</ac:structured-macro>"#;
        let md =
            html_to_markdown_with_excerpt_ids(html, 50_000, None, &[Some("123456".to_string())]);
        assert!(
            md.contains("[excerpt from: Source Page (id: 123456)]"),
            "resolved id should appear in placeholder: {md}"
        );
    }

    #[test]
    fn test_excerpt_include_falls_back_to_title_when_unresolved() {
        let html = r#"<ac:structured-macro ac:name="excerpt-include">
  <ac:default-parameter><ac:link><ri:page ri:content-title="Source Page"/></ac:link></ac:default-parameter>
</ac:structured-macro>"#;
        let md = html_to_markdown_with_excerpt_ids(html, 50_000, None, &[None]);
        assert!(
            md.contains("[excerpt from: Source Page]"),
            "unresolved lookup should fall back to title-only: {md}"
        );
        assert!(!md.contains("id:"));
    }

    #[test]
    fn test_excerpt_include_id_index_survives_missing_ref() {
        // Two excerpt-include macros; excerpt_ids has one entry per
        // occurrence, so the second one's id must line up correctly.
        let html = r#"<ac:structured-macro ac:name="excerpt-include">
  <ac:default-parameter><ac:link><ri:page ri:content-title="First"/></ac:link></ac:default-parameter>
</ac:structured-macro>
<ac:structured-macro ac:name="excerpt-includeplus">
  <ac:default-parameter><ac:link><ri:page ri:content-title="Second"/></ac:link></ac:default-parameter>
</ac:structured-macro>"#;
        let md =
            html_to_markdown_with_excerpt_ids(html, 50_000, None, &[None, Some("999".to_string())]);
        assert!(md.contains("[excerpt from: First]"));
        assert!(md.contains("[excerpt from: Second (id: 999)]"));
    }

    // ── Nested macros ─────────────────────────────────────────────────────────

    #[test]
    fn test_nested_macro_expand_contains_code() {
        let html = r#"<ac:structured-macro ac:name="expand">
  <ac:parameter ac:name="title">Show Code</ac:parameter>
  <ac:rich-text-body>
    <ac:structured-macro ac:name="code">
      <ac:parameter ac:name="language">python</ac:parameter>
      <ac:plain-text-body><![CDATA[print("hello")]]></ac:plain-text-body>
    </ac:structured-macro>
  </ac:rich-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(md.contains("**▸ Show Code**"), "expand title should appear");
        assert!(md.contains("```python"), "code fence with language");
        assert!(md.contains(r#"print("hello")"#), "code body should appear");
    }

    // ── Self-closing macro tag ────────────────────────────────────────────────

    #[test]
    fn test_self_closing_macro_does_not_swallow_content() {
        let html = r#"<ac:structured-macro ac:name="toc" ac:schema-version="1"/><p>After TOC</p>"#;
        let md = html_to_markdown(html, 50_000, None);
        assert!(
            md.contains("After TOC"),
            "content after self-closing macro must appear"
        );
        assert!(md.contains("[TOC]"));
    }

    // ── Parameter with special chars ──────────────────────────────────────────

    #[test]
    fn test_param_value_with_double_quotes() {
        // title contains a double-quote character (XML-entity-encoded in storage)
        let html = r#"<ac:structured-macro ac:name="expand">
  <ac:parameter ac:name="title">Say &quot;hello&quot;</ac:parameter>
  <ac:rich-text-body><p>Body.</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = html_to_markdown(html, 50_000, None);
        // html5ever decodes &quot; back to " when reading the attribute value
        assert!(
            md.contains(r#"**▸ Say "hello"**"#) || md.contains("**▸ Say"),
            "title should appear: {md}"
        );
        assert!(md.contains("Body."));
    }
}
