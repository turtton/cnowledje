use crate::error::ConfluenceError;
use crate::models::{
    ErrorDetail, ErrorOutput, JiraIssueOutput, JiraSearchOutput, PageOutput, SearchOutput,
    UnifiedSearchOutput,
};

// ── Search output ─────────────────────────────────────────────────────────────

/// Print search results as pretty JSON.
pub fn print_search_json(output: &SearchOutput) -> Result<(), ConfluenceError> {
    println!("{}", serde_json::to_string_pretty(output)?);
    Ok(())
}

/// Print search results in human-readable form.
pub fn print_search_human(output: &SearchOutput) {
    match (&output.query, &output.search_in) {
        (Some(query), Some(search_in)) => println!(
            "Search: \"{}\" in [{}] ({})",
            query,
            output.spaces.join(", "),
            search_in,
        ),
        _ => println!("Search: labels only in [{}]", output.spaces.join(", ")),
    }
    if !output.labels.is_empty() {
        println!("Labels: {}", output.labels.join(", "));
    }
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
        if !r.labels.is_empty() {
            println!("     Labels : {}", r.labels.join(", "));
        }
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

/// Print combined search results as pretty JSON.
pub fn print_unified_search_json(output: &UnifiedSearchOutput) -> Result<(), ConfluenceError> {
    println!("{}", serde_json::to_string_pretty(output)?);
    Ok(())
}

/// Print combined search results in human-readable form.
pub fn print_unified_search_human(output: &UnifiedSearchOutput) {
    let confluence_needs_separator = output
        .confluence
        .as_ref()
        .is_some_and(|confluence| confluence.results.is_empty());

    if let Some(confluence) = &output.confluence {
        println!("=== Confluence ===");
        print_search_human(confluence);
    }

    if let Some(jira) = &output.jira {
        if confluence_needs_separator {
            println!();
        }
        println!("=== Jira ===");
        print_jira_search_human(jira);
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
    println!(
        "<!-- Title: {} | Space: {} -->",
        output.title, output.space_key
    );
    println!("<!-- URL: {} -->", output.url);
    if let Some(ts) = &output.last_modified {
        println!("<!-- Last modified: {} -->", ts);
    }
    if !output.labels.is_empty() {
        println!("<!-- Labels: {} -->", output.labels.join(", "));
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

// ── Jira search output ────────────────────────────────────────────────────────

/// Print Jira search results as pretty JSON.
pub fn print_jira_search_json(output: &JiraSearchOutput) -> Result<(), ConfluenceError> {
    println!("{}", serde_json::to_string_pretty(output)?);
    Ok(())
}

/// Print Jira search results in human-readable form.
pub fn print_jira_search_human(output: &JiraSearchOutput) {
    match &output.query {
        Some(q) => println!(
            "Search: \"{}\" in projects [{}]",
            q,
            output.projects.join(", ")
        ),
        None => println!(
            "Search: (filters only) in projects [{}]",
            output.projects.join(", ")
        ),
    }
    println!("JQL   : {}", output.jql);
    println!("Results: {} (total {})", output.results.len(), output.total);
    println!("{}", "─".repeat(72));

    for (i, r) in output.results.iter().enumerate() {
        println!("{:>3}. {} — {}", i + 1, r.key, r.summary);
        if let Some(status) = &r.status {
            println!("     Status  : {}", status);
        }
        if let Some(issue_type) = &r.issue_type {
            println!("     Type    : {}", issue_type);
        }
        if let Some(assignee) = &r.assignee {
            println!("     Assignee: {}", assignee);
        }
        if let Some(updated) = &r.updated {
            println!("     Updated : {}", updated);
        }
        println!("     URL     : {}", r.url);
        println!();
    }
}

// ── Jira issue output ─────────────────────────────────────────────────────────

/// Print a Jira issue as pretty JSON.
pub fn print_jira_issue_json(output: &JiraIssueOutput) -> Result<(), ConfluenceError> {
    println!("{}", serde_json::to_string_pretty(output)?);
    Ok(())
}

/// Print a Jira issue's Markdown content (description + comments) with a
/// brief metadata header.
pub fn print_jira_issue_markdown(output: &JiraIssueOutput) {
    println!("<!-- Notice: {} -->", output.notice);
    println!(
        "<!-- Key: {} | Project: {} -->",
        output.key,
        output.project_key.as_deref().unwrap_or("-")
    );
    println!("<!-- URL: {} -->", output.url);
    if let Some(ts) = &output.updated {
        println!("<!-- Last updated: {} -->", ts);
    }
    println!();
    println!("# {}: {}", output.key, output.summary);
    println!();
    if let Some(status) = &output.status {
        println!("- Status: {}", status);
    }
    if let Some(issue_type) = &output.issue_type {
        println!("- Type: {}", issue_type);
    }
    if let Some(priority) = &output.priority {
        println!("- Priority: {}", priority);
    }
    if let Some(assignee) = &output.assignee {
        println!("- Assignee: {}", assignee);
    }
    if let Some(reporter) = &output.reporter {
        println!("- Reporter: {}", reporter);
    }
    if !output.labels.is_empty() {
        println!("- Labels: {}", output.labels.join(", "));
    }
    println!();
    println!("## Description");
    println!();
    println!("{}", output.description_markdown);

    if !output.comments.is_empty() {
        println!();
        println!("## Comments ({})", output.comments.len());
        for c in &output.comments {
            println!();
            println!(
                "### {} ({})",
                c.author.as_deref().unwrap_or("(unknown)"),
                c.created.as_deref().unwrap_or("(unknown)")
            );
            println!();
            println!("{}", c.body_markdown);
        }
    }

    if output.omitted_comments > 0 {
        println!();
        println!("[{} more comments truncated]", output.omitted_comments);
    }
}

/// Print plain text: description followed by comment bodies, no header.
pub fn print_jira_issue_plain(output: &JiraIssueOutput) {
    println!("{}", output.description_markdown);
    for c in &output.comments {
        println!();
        println!("{}", c.body_markdown);
    }
    if output.omitted_comments > 0 {
        println!();
        println!("[{} more comments truncated]", output.omitted_comments);
    }
}

// ── Jira URL helpers ──────────────────────────────────────────────────────────

/// Build a full issue URL from the Jira base_url and issue key:
/// `{base_url}/browse/{key}`.
pub fn make_issue_url(base_url: &str, key: &str) -> String {
    format!("{}/browse/{}", base_url.trim_end_matches('/'), key)
}
