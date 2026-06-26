use crate::error::ConfluenceError;
use crate::models::{ErrorDetail, ErrorOutput, PageOutput, SearchOutput};

// ── Search output ─────────────────────────────────────────────────────────────

/// Print search results as pretty JSON.
pub fn print_search_json(output: &SearchOutput) -> Result<(), ConfluenceError> {
    println!("{}", serde_json::to_string_pretty(output)?);
    Ok(())
}

/// Print search results in human-readable form.
pub fn print_search_human(output: &SearchOutput) {
    println!(
        "Search: \"{}\" in [{}] ({})",
        output.query,
        output.spaces.join(", "),
        output.search_in,
    );
    println!("Results: {}", output.results.len());
    println!("{}", "─".repeat(72));

    for (i, r) in output.results.iter().enumerate() {
        println!("{:>3}. {}", i + 1, r.title);
        println!("     Space : {} – {}", r.space_key, r.space_name);
        println!("     URL   : {}", r.url);
        if let Some(ts) = &r.last_modified {
            println!("     Modified : {}", ts);
        }
        println!("     Match  : {}", r.matched_by.join(", "));
        if let Some(ex) = &r.excerpt {
            let ex = ex.trim().replace('\n', " ");
            let truncated = if ex.chars().count() > 120 {
                let end = ex
                    .char_indices()
                    .nth(120)
                    .map(|(i, _)| i)
                    .unwrap_or(ex.len());
                format!("{}…", &ex[..end])
            } else {
                ex.clone()
            };
            println!("     Excerpt: {}", truncated);
        }
        println!();
    }
}

// ── Page output ───────────────────────────────────────────────────────────────

/// Print a page as pretty JSON.
pub fn print_page_json(output: &PageOutput) -> Result<(), ConfluenceError> {
    println!("{}", serde_json::to_string_pretty(output)?);
    Ok(())
}

/// Print a page's Markdown content with a brief metadata header.
pub fn print_page_markdown(output: &PageOutput) {
    println!("<!-- Notice: {} -->", output.notice);
    println!("<!-- Title: {} | Space: {} -->", output.title, output.space_key);
    println!("<!-- URL: {} -->", output.url);
    if let Some(ts) = &output.last_modified {
        println!("<!-- Last modified: {} -->", ts);
    }
    println!();
    println!("{}", output.content_markdown);
}

/// Print the raw storage HTML (for debugging / advanced users).
pub fn print_page_storage_html(html: &str, title: &str, url: &str) {
    println!("<!-- Title: {} -->", title);
    println!("<!-- URL: {} -->", url);
    println!();
    println!("{}", html);
}

/// Print plain text (whitespace-collapsed Markdown without any heading).
pub fn print_page_plain(output: &PageOutput) {
    println!("{}", output.content_markdown);
}

// ── Error output ──────────────────────────────────────────────────────────────

/// Emit an error as JSON (for --json / --format json modes).
pub fn print_error_json(err: &ConfluenceError) {
    let output = ErrorOutput {
        error: ErrorDetail {
            kind: err.kind().to_string(),
            message: err.to_string(),
        },
    };
    // Best-effort; if JSON serialization itself fails, fall back to stderr.
    match serde_json::to_string_pretty(&output) {
        Ok(s) => eprintln!("{}", s),
        Err(_) => eprintln!("error: {}", err),
    }
}

// ── URL helpers ───────────────────────────────────────────────────────────────

/// Build a full page URL from the base_url in config and the webui relative
/// path returned by the Confluence API.
pub fn make_page_url(base_url: &str, api_base: Option<&str>, webui: Option<&str>) -> String {
    let base = api_base.unwrap_or(base_url).trim_end_matches('/');
    match webui {
        Some(rel) => format!("{}{}", base, rel),
        None => base.to_string(),
    }
}
