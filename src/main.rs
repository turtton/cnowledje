mod cli;

use clap::Parser;
use std::collections::{HashMap, HashSet};

use cli::{Cli, Commands, ConfigSubcommand, SkillSubcommand};
use cnowledje::client::ConfluenceClient;
use cnowledje::config::{
    default_config_path, delete_jira_token_from_keyring, delete_token_from_keyring, load_config,
    load_jira_config, load_profile_config, profile_exists, resolve_projects, resolve_spaces,
    save_profile_to_path, store_jira_token_in_keyring, store_token_in_keyring, validate_projects,
    validate_spaces, Config, JiraConfig, TokenSource,
};
use cnowledje::cql::{
    build_exact_title_cql, build_label_cql, build_text_cql, build_title_cql, extract_page_id,
};
use cnowledje::error::ConfluenceError;
use cnowledje::format::{
    make_issue_url, make_page_url, print_error_json, print_jira_issue_json,
    print_jira_issue_markdown, print_jira_issue_plain, print_page_json, print_page_markdown,
    print_page_plain, print_page_storage_html, print_unified_search_human,
    print_unified_search_json,
};
use cnowledje::jira_client::JiraClient;
use cnowledje::jql::{build_search_jql, extract_issue_key, JqlFilters};
use cnowledje::markdown;
use cnowledje::models::SearchResult;
use cnowledje::models::{
    JiraIssueOutput, JiraSearchOutput, JiraSearchResultOutput, PageOutput, SearchOutput,
    SearchResultOutput, UnifiedSearchOutput, JIRA_NOTICE, NOTICE,
};
use cnowledje::skill;
use cnowledje::types::{IssueFormat, PageFormat, SearchIn, SearchSource};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    let exit_code = match cli.command {
        Commands::Search(args) => {
            let json = args.json;
            match run_search(args).await {
                Ok(()) => 0,
                Err(e) => {
                    if json {
                        print_error_json(&e);
                    } else {
                        eprintln!("error: {}", e);
                    }
                    1
                }
            }
        }
        Commands::Page(args) => {
            let json = args.json || args.format == PageFormat::Json;
            match run_page(args).await {
                Ok(()) => 0,
                Err(e) => {
                    if json {
                        print_error_json(&e);
                    } else {
                        eprintln!("error: {}", e);
                    }
                    1
                }
            }
        }
        Commands::Config(args) => match run_config(args).await {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("error: {}", e);
                1
            }
        },
        Commands::Skill(args) => match run_skill(args) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("error: {}", e);
                1
            }
        },
        Commands::Issue(args) => {
            let json = args.json || args.format == IssueFormat::Json;
            match run_issue(args).await {
                Ok(()) => 0,
                Err(e) => {
                    if json {
                        print_error_json(&e);
                    } else {
                        eprintln!("error: {}", e);
                    }
                    1
                }
            }
        }
    };

    std::process::exit(exit_code);
}

// ── search command ────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
struct SearchPlan {
    confluence: bool,
    jira: bool,
}

fn plan_search_sources(
    source: Option<SearchSource>,
    query_present: bool,
    labels_present: bool,
    confluence_flags: bool,
    jira_flags: bool,
) -> Result<SearchPlan, ConfluenceError> {
    let (mut confluence, jira) = match source {
        None | Some(SearchSource::All) => (true, true),
        Some(SearchSource::Confluence) => (true, false),
        Some(SearchSource::Jira) => (false, true),
    };

    if !confluence && confluence_flags {
        return Err(ConfluenceError::InvalidArguments(
            "--space/--in apply to Confluence but --source jira excludes it".into(),
        ));
    }
    if !jira && jira_flags {
        return Err(ConfluenceError::InvalidArguments(
            "--project/--status/--assignee/--reporter/--type apply to Jira but --source confluence excludes it".into(),
        ));
    }

    if !query_present && !labels_present {
        if !jira_flags {
            return Err(ConfluenceError::NoSearchCriteria);
        }
        if confluence && (source.is_some() || confluence_flags) {
            return Err(ConfluenceError::QueryRequiredForConfluence);
        }
        confluence = false;
    }

    Ok(SearchPlan { confluence, jira })
}

async fn run_search(args: cli::SearchArgs) -> Result<(), ConfluenceError> {
    let query = args
        .query
        .as_deref()
        .filter(|query| !query.trim().is_empty());
    let query_present = query.is_some();
    let labels_present = !args.labels.is_empty();
    let confluence_flags = !args.spaces.is_empty() || args.search_in.is_some();
    let jira_flags = !args.projects.is_empty()
        || !args.status.is_empty()
        || args.assignee.is_some()
        || args.reporter.is_some()
        || !args.issue_types.is_empty();
    if args.search_in.is_some() && !query_present {
        return Err(ConfluenceError::InvalidArguments(
            "--in requires a search query".into(),
        ));
    }
    let plan = plan_search_sources(
        args.source,
        query_present,
        labels_present,
        confluence_flags,
        jira_flags,
    )?;

    let confluence_pinned = matches!(
        args.source,
        Some(SearchSource::Confluence | SearchSource::All)
    ) || confluence_flags;
    let jira_pinned =
        matches!(args.source, Some(SearchSource::Jira | SearchSource::All)) || jira_flags;
    let mut first_skipped_error = None;

    let confluence_leg = if plan.confluence {
        match load_config(args.profile.as_deref()) {
            Ok(config) => {
                if args.limit > config.max_limit {
                    return Err(ConfluenceError::LimitExceeded {
                        requested: args.limit,
                        max: config.max_limit,
                    });
                }
                match resolve_spaces(args.spaces.clone(), &config) {
                    Ok(spaces) => {
                        validate_spaces(&spaces, &config)?;
                        Some((
                            config,
                            spaces,
                            args.search_in.clone().unwrap_or(SearchIn::Both),
                        ))
                    }
                    Err(error @ ConfluenceError::NoSpaceSpecified) if !confluence_pinned => {
                        eprintln!(
                            "warning: no space specified and no default_space configured; skipping Confluence"
                        );
                        first_skipped_error = Some(error);
                        None
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error @ ConfluenceError::MissingBaseUrl) if !confluence_pinned => {
                eprintln!("warning: Confluence is not configured; skipping");
                first_skipped_error = Some(error);
                None
            }
            Err(error) => return Err(error),
        }
    } else {
        None
    };

    let jira_leg = if plan.jira {
        match load_jira_config(args.profile.as_deref()) {
            Ok(config) => {
                if args.limit > config.max_limit {
                    return Err(ConfluenceError::LimitExceeded {
                        requested: args.limit,
                        max: config.max_limit,
                    });
                }
                match resolve_projects(args.projects.clone(), &config) {
                    Ok(projects) => {
                        validate_projects(&projects, &config)?;
                        Some((config, projects))
                    }
                    Err(error @ ConfluenceError::NoProjectSpecified) if !jira_pinned => {
                        eprintln!(
                            "warning: no project specified and no jira_default_project configured; skipping Jira"
                        );
                        if first_skipped_error.is_none() {
                            first_skipped_error = Some(error);
                        }
                        None
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error @ ConfluenceError::MissingJiraBaseUrl) if !jira_pinned => {
                eprintln!("warning: Jira is not configured; skipping");
                if first_skipped_error.is_none() {
                    first_skipped_error = Some(error);
                }
                None
            }
            Err(error) => return Err(error),
        }
    } else {
        None
    };

    let filters = JqlFilters {
        statuses: &args.status,
        assignee: args.assignee.as_deref(),
        reporter: args.reporter.as_deref(),
        issue_types: &args.issue_types,
        labels: &args.labels,
    };
    let (confluence, jira) = match (confluence_leg, jira_leg) {
        (Some((confluence_config, spaces, search_in)), Some((jira_config, projects))) => {
            let (confluence, jira) = tokio::try_join!(
                search_confluence(
                    &confluence_config,
                    query,
                    spaces,
                    &search_in,
                    &args.labels,
                    args.limit,
                ),
                search_jira(&jira_config, query, projects, &filters, args.limit,),
            )?;
            (Some(confluence), Some(jira))
        }
        (Some((config, spaces, search_in)), None) => {
            let output =
                search_confluence(&config, query, spaces, &search_in, &args.labels, args.limit)
                    .await?;
            (Some(output), None)
        }
        (None, Some((config, projects))) => {
            let output = search_jira(&config, query, projects, &filters, args.limit).await?;
            (None, Some(output))
        }
        (None, None) => {
            return Err(first_skipped_error.expect("a skipped backend records its error"))
        }
    };

    let output = UnifiedSearchOutput {
        query: query.map(str::to_string),
        confluence,
        jira,
    };
    if args.json {
        print_unified_search_json(&output)?;
    } else {
        print_unified_search_human(&output);
    }

    Ok(())
}

async fn search_confluence(
    config: &Config,
    query: Option<&str>,
    spaces: Vec<String>,
    search_in: &SearchIn,
    labels: &[String],
    limit: u32,
) -> Result<SearchOutput, ConfluenceError> {
    let client = ConfluenceClient::new(&config.base_url, &config.api_path, &config.token)?;
    let internal_limit = std::cmp::min(limit * 2, config.max_limit);
    let groups: Vec<(&'static str, Vec<SearchResult>)> = match query {
        Some(query) => match search_in {
            SearchIn::Title => {
                let cql = build_title_cql(&spaces, query, labels);
                let response = client.search(&cql, internal_limit).await?;
                vec![("title", response.results)]
            }
            SearchIn::Text => {
                let cql = build_text_cql(&spaces, query, labels);
                let response = client.search(&cql, internal_limit).await?;
                vec![("text", response.results)]
            }
            SearchIn::Both => {
                let title_cql = build_title_cql(&spaces, query, labels);
                let text_cql = build_text_cql(&spaces, query, labels);
                let (title_response, text_response) = tokio::try_join!(
                    client.search(&title_cql, internal_limit),
                    client.search(&text_cql, internal_limit)
                )?;
                vec![
                    ("title", title_response.results),
                    ("text", text_response.results),
                ]
            }
        },
        None => {
            let cql = build_label_cql(&spaces, labels);
            let response = client.search(&cql, internal_limit).await?;
            vec![("label", response.results)]
        }
    };

    let mut seen = HashSet::new();
    let mut matched_by: HashMap<String, Vec<String>> = HashMap::new();
    let mut order = Vec::new();
    for (matched, group) in &groups {
        for result in group {
            if seen.insert(result.id.clone()) {
                order.push(result.id.clone());
            }
            matched_by
                .entry(result.id.clone())
                .or_default()
                .push((*matched).to_string());
        }
    }

    let all_results: HashMap<String, &SearchResult> = groups
        .iter()
        .flat_map(|(_, group)| group.iter())
        .map(|result| (result.id.clone(), result))
        .collect();
    let results = order
        .iter()
        .take(limit as usize)
        .filter_map(|id| all_results.get(id).copied())
        .map(|result| SearchResultOutput {
            id: result.id.clone(),
            title: result.title.clone(),
            space_key: result.space.key.clone(),
            space_name: result.space.name.clone(),
            url: make_page_url(&config.base_url, None, result.links.webui.as_deref()),
            last_modified: result.version.when.clone(),
            matched_by: matched_by.get(&result.id).cloned().unwrap_or_default(),
            labels: result.metadata.label_names(),
            excerpt: result.excerpt.clone(),
        })
        .collect();

    Ok(SearchOutput {
        query: query.map(str::to_string),
        spaces,
        labels: labels.to_vec(),
        search_in: query.is_some().then(|| search_in.to_string()),
        results,
    })
}

// ── page command ──────────────────────────────────────────────────────────────

async fn run_page(args: cli::PageArgs) -> Result<(), ConfluenceError> {
    let config = load_config(args.profile.as_deref())?;
    let page_id = extract_page_id(&args.page_id_or_url)?;

    let client = ConfluenceClient::new(&config.base_url, &config.api_path, &config.token)?;
    let page = client.get_page(&page_id).await?;

    let url = make_page_url(
        &config.base_url,
        page.links.base.as_deref(),
        page.links.webui.as_deref(),
    );

    let effective_format = args.effective_format();

    // storage-html: return raw body without conversion
    if effective_format == PageFormat::StorageHtml {
        let html = page
            .body
            .as_ref()
            .and_then(|b| b.storage.as_ref())
            .map(|s| s.value.as_str())
            .unwrap_or("");
        print_page_storage_html(html, &page.title, &url);
        return Ok(());
    }

    // Convert storage HTML → Markdown for all other formats
    let html = page
        .body
        .as_ref()
        .and_then(|b| b.storage.as_ref())
        .map(|s| s.value.as_str())
        .unwrap_or("");

    let effective_max = std::cmp::min(args.max_chars, config.max_page_chars);

    // Resolve excerpt-include/-includeplus page references (title [+ space])
    // to page IDs via a CQL title search, since the storage format never
    // carries an ID for these. Falls back to title-only on any lookup miss.
    let excerpt_refs = markdown::extract_excerpt_refs(html);
    let mut resolved_excerpt_ids: HashMap<(String, String), Option<String>> = HashMap::new();
    for r in &excerpt_refs {
        if r.title.is_empty() {
            continue;
        }
        let space = r
            .space_key
            .clone()
            .unwrap_or_else(|| page.space.key.clone());
        let key = (r.title.clone(), space);
        if resolved_excerpt_ids.contains_key(&key) {
            continue;
        }
        let cql = build_exact_title_cql(&key.1, &key.0);
        let resolved_id = client
            .search(&cql, 1)
            .await
            .ok()
            .and_then(|resp| resp.results.into_iter().next().map(|res| res.id));
        resolved_excerpt_ids.insert(key, resolved_id);
    }
    let excerpt_ids: Vec<Option<String>> = excerpt_refs
        .iter()
        .map(|r| {
            if r.title.is_empty() {
                return None;
            }
            let space = r
                .space_key
                .clone()
                .unwrap_or_else(|| page.space.key.clone());
            resolved_excerpt_ids
                .get(&(r.title.clone(), space))
                .cloned()
                .flatten()
        })
        .collect();

    let content_markdown = markdown::html_to_markdown_with_excerpt_ids(
        html,
        effective_max,
        args.language.as_deref(),
        &excerpt_ids,
    );

    let output = PageOutput {
        id: page.id.clone(),
        title: page.title.clone(),
        space_key: page.space.key.clone(),
        url: url.clone(),
        last_modified: page.version.when.clone(),
        labels: page.metadata.label_names(),
        content_markdown,
        notice: NOTICE,
    };

    match effective_format {
        PageFormat::Json => print_page_json(&output)?,
        PageFormat::Markdown => print_page_markdown(&output),
        PageFormat::Plain => print_page_plain(&output),
        PageFormat::StorageHtml => unreachable!(), // handled above
    }

    Ok(())
}

// ── Jira search helper ──────────────────────────────────────────────────────

async fn search_jira(
    config: &JiraConfig,
    query: Option<&str>,
    projects: Vec<String>,
    filters: &JqlFilters<'_>,
    limit: u32,
) -> Result<JiraSearchOutput, ConfluenceError> {
    let jql = build_search_jql(&projects, query, filters);
    let client = JiraClient::new(&config.base_url, &config.api_path, &config.token)?;
    let response = client.search(&jql, limit).await?;
    let results = response
        .issues
        .iter()
        .map(|issue| {
            let fields = &issue.fields;
            JiraSearchResultOutput {
                key: issue.key.clone(),
                summary: fields.summary.clone().unwrap_or_default(),
                status: fields.status.as_ref().map(|status| status.name.clone()),
                issue_type: fields
                    .issuetype
                    .as_ref()
                    .map(|issue_type| issue_type.name.clone()),
                priority: fields
                    .priority
                    .as_ref()
                    .map(|priority| priority.name.clone()),
                assignee: fields
                    .assignee
                    .as_ref()
                    .and_then(|user| user.display_name.clone().or_else(|| user.name.clone())),
                project_key: fields.project.as_ref().map(|project| project.key.clone()),
                url: make_issue_url(&config.base_url, &issue.key),
                updated: fields.updated.clone(),
            }
        })
        .collect();

    Ok(JiraSearchOutput {
        query: query.map(str::to_string),
        projects,
        jql,
        total: response.total,
        results,
    })
}

// ── issue command ───────────────────────────────────────────────────────────

async fn run_issue(args: cli::IssueArgs) -> Result<(), ConfluenceError> {
    let config = load_jira_config(args.profile.as_deref())?;
    let key = extract_issue_key(&args.issue_key_or_url)?;

    let client = JiraClient::new(&config.base_url, &config.api_path, &config.token)?;
    let issue = client.get_issue(&key).await?;

    let f = &issue.fields;

    // Comment metadata (author/created) comes from `fields.comment.comments`;
    // the HTML body comes from `rendered_fields.comment.comments` at the same
    // index. A missing/out-of-range rendered entry falls back to the raw body.
    let raw_comments = f
        .comment
        .as_ref()
        .map(|c| c.comments.as_slice())
        .unwrap_or(&[]);
    let rendered_comments = issue
        .rendered_fields
        .as_ref()
        .and_then(|rf| rf.comment.as_ref())
        .map(|c| c.comments.as_slice())
        .unwrap_or(&[]);

    let comment_sources: Vec<markdown::IssueCommentSource> = raw_comments
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let author = c
                .author
                .as_ref()
                .and_then(|u| u.display_name.clone().or_else(|| u.name.clone()));
            let body_html = rendered_comments.get(i).and_then(|rc| rc.body.clone());
            markdown::IssueCommentSource {
                author,
                created: c.created.clone(),
                body_html,
                body_raw: c.body.clone(),
            }
        })
        .collect();

    let effective_max = std::cmp::min(args.max_chars, config.max_issue_chars);
    let rendered_desc_html = issue
        .rendered_fields
        .as_ref()
        .and_then(|rf| rf.description.as_deref());
    let raw_desc = f.description.as_deref();

    let rendered = markdown::render_issue_content(
        rendered_desc_html,
        raw_desc,
        &comment_sources,
        effective_max,
    );

    let output = JiraIssueOutput {
        key: issue.key.clone(),
        summary: f.summary.clone().unwrap_or_default(),
        project_key: f.project.as_ref().map(|p| p.key.clone()),
        status: f.status.as_ref().map(|s| s.name.clone()),
        issue_type: f.issuetype.as_ref().map(|t| t.name.clone()),
        priority: f.priority.as_ref().map(|p| p.name.clone()),
        assignee: f
            .assignee
            .as_ref()
            .and_then(|u| u.display_name.clone().or_else(|| u.name.clone())),
        reporter: f
            .reporter
            .as_ref()
            .and_then(|u| u.display_name.clone().or_else(|| u.name.clone())),
        labels: f.labels.clone().unwrap_or_default(),
        created: f.created.clone(),
        updated: f.updated.clone(),
        url: make_issue_url(&config.base_url, &issue.key),
        description_markdown: rendered.description_markdown,
        comments: rendered.comments,
        omitted_comments: rendered.omitted_comments,
        notice: JIRA_NOTICE,
    };

    match args.effective_format() {
        IssueFormat::Json => print_jira_issue_json(&output)?,
        IssueFormat::Markdown => print_jira_issue_markdown(&output),
        IssueFormat::Plain => print_jira_issue_plain(&output),
    }

    Ok(())
}

// ── config check command ──────────────────────────────────────────────────────

async fn run_config(args: cli::ConfigArgs) -> Result<(), ConfluenceError> {
    match args.command {
        ConfigSubcommand::Check { profile } => {
            println!("Profile      : {}", profile.as_deref().unwrap_or("default"));
            println!();

            let mut first_err: Option<ConfluenceError> = None;
            let mut confluence_configured = false;
            let mut jira_configured = false;

            match load_config(profile.as_deref()) {
                Ok(config) => {
                    confluence_configured = true;
                    println!("Confluence   :");
                    println!("  base_url       : {}", config.base_url);
                    println!("  api_path       : {}", config.api_path);
                    let token_src = match config.token_source {
                        TokenSource::Env => "[from env]",
                        TokenSource::Keyring => "[from keyring]",
                        _ => "[from unknown source]",
                    };
                    println!("  token          : {}", token_src);
                    println!(
                        "  allowed_spaces : {}",
                        config
                            .allowed_spaces
                            .as_ref()
                            .map(|v| v.join(", "))
                            .unwrap_or_else(|| "(none – all spaces allowed)".to_string())
                    );
                    println!(
                        "  default_space  : {}",
                        config.default_space.as_deref().unwrap_or("(none)")
                    );
                    println!("  default_limit  : {}", config.default_limit);
                    println!("  max_limit      : {}", config.max_limit);
                    println!("  max_page_chars : {}", config.max_page_chars);

                    print!("  Checking Confluence API connectivity... ");
                    let check: Result<(), ConfluenceError> = async {
                        let client = ConfluenceClient::new(
                            &config.base_url,
                            &config.api_path,
                            &config.token,
                        )?;
                        client
                            .search("type = page AND title = \"__confluence_ro_check__\"", 1)
                            .await?;
                        Ok(())
                    }
                    .await;
                    match check {
                        Ok(()) => println!("OK"),
                        Err(e) => {
                            println!("FAILED");
                            first_err.get_or_insert(e);
                        }
                    }
                }
                Err(ConfluenceError::MissingBaseUrl) => {
                    println!("Confluence   : (not configured)");
                }
                Err(e) => {
                    confluence_configured = true;
                    println!("Confluence   : configuration error: {}", e);
                    first_err.get_or_insert(e);
                }
            }

            println!();

            match load_jira_config(profile.as_deref()) {
                Ok(config) => {
                    jira_configured = true;
                    println!("Jira         :");
                    println!("  jira_base_url         : {}", config.base_url);
                    println!("  jira_api_path         : {}", config.api_path);
                    let token_src = match config.token_source {
                        TokenSource::Env => "[from env]",
                        TokenSource::Keyring => "[from keyring]",
                        _ => "[from unknown source]",
                    };
                    println!("  token                 : {}", token_src);
                    println!(
                        "  jira_allowed_projects : {}",
                        config
                            .allowed_projects
                            .as_ref()
                            .map(|v| v.join(", "))
                            .unwrap_or_else(|| "(none – all projects allowed)".to_string())
                    );
                    println!(
                        "  jira_default_project  : {}",
                        config.default_project.as_deref().unwrap_or("(none)")
                    );

                    print!("  Checking Jira API connectivity... ");
                    let check: Result<(), ConfluenceError> = async {
                        let client =
                            JiraClient::new(&config.base_url, &config.api_path, &config.token)?;
                        client.check_connectivity().await
                    }
                    .await;
                    match check {
                        Ok(()) => println!("OK"),
                        Err(e) => {
                            println!("FAILED");
                            first_err.get_or_insert(e);
                        }
                    }
                }
                Err(ConfluenceError::MissingJiraBaseUrl) => {
                    println!("Jira         : (not configured)");
                }
                Err(e) => {
                    jira_configured = true;
                    println!("Jira         : configuration error: {}", e);
                    first_err.get_or_insert(e);
                }
            }

            if !confluence_configured && !jira_configured {
                return Err(ConfluenceError::MissingBaseUrl);
            }

            if let Some(e) = first_err {
                return Err(e);
            }
        }
        ConfigSubcommand::Init {
            profile,
            confluence,
            jira,
        } => {
            run_config_init(profile, confluence, jira)?;
        }
        ConfigSubcommand::Token(token_args) => {
            run_config_token(token_args)?;
        }
    }

    Ok(())
}

fn run_config_token(args: cli::TokenArgs) -> Result<(), ConfluenceError> {
    use inquire::Password;

    let jira = match &args.command {
        cli::TokenSubcommand::Set { jira, .. } => *jira,
        cli::TokenSubcommand::Delete { jira, .. } => *jira,
    };
    let env_var = if jira {
        "JIRA_TOKEN"
    } else {
        "CONFLUENCE_TOKEN"
    };
    if std::env::var(env_var)
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false)
    {
        eprintln!(
            "Note: {} is set and will take precedence over the keyring token.",
            env_var
        );
    }
    match args.command {
        cli::TokenSubcommand::Set { profile, jira } => {
            let profile_name = profile.as_deref().unwrap_or("default");
            let token = Password::new("API Token:")
                .prompt()
                .map_err(|e| ConfluenceError::ConfigError(e.to_string()))?;
            if token.trim().is_empty() {
                return Err(ConfluenceError::ConfigError(
                    "Token must not be empty.".to_string(),
                ));
            }
            if jira {
                store_jira_token_in_keyring(profile_name, &token)?;
                println!(
                    "Jira token stored in keyring for profile '{}'.",
                    profile_name
                );
            } else {
                store_token_in_keyring(profile_name, &token)?;
                println!("Token stored in keyring for profile '{}'.", profile_name);
            }
        }
        cli::TokenSubcommand::Delete { profile, jira } => {
            let profile_name = profile.as_deref().unwrap_or("default");
            if jira {
                delete_jira_token_from_keyring(profile_name)?;
                println!(
                    "Jira token removed from keyring for profile '{}'.",
                    profile_name
                );
            } else {
                delete_token_from_keyring(profile_name)?;
                println!("Token removed from keyring for profile '{}'.", profile_name);
            }
        }
    }
    Ok(())
}

// ── config init command ───────────────────────────────────────────────────────

fn run_config_init(profile: String, confluence: bool, jira: bool) -> Result<(), ConfluenceError> {
    use inquire::error::InquireError;
    use inquire::validator::Validation;
    use inquire::{Confirm, CustomType, Password, Text};

    let profile_name = profile.as_str();
    let config_path = default_config_path().ok_or_else(|| {
        ConfluenceError::ConfigError("設定ファイルのパスを決定できません".to_string())
    })?;
    let existing = load_profile_config(profile_name)?;
    let profile_exists = profile_exists(profile_name)?;
    let has_confluence = existing.base_url.is_some();
    let has_jira = existing.jira_base_url.is_some();

    println!(
        "プロファイル '{}' を初期化します。Ctrl-C または Esc でキャンセルできます。\n",
        profile_name
    );

    let (configure_confluence, configure_jira) = if confluence || jira {
        (confluence, jira)
    } else {
        let confluence_help = existing
            .base_url
            .as_deref()
            .map(|base_url| format!("現在の Base URL: {base_url}"));
        let mut confluence_prompt =
            Confirm::new("Confluence を設定しますか?").with_default(!has_confluence);
        if let Some(help) = confluence_help.as_deref() {
            confluence_prompt = confluence_prompt.with_help_message(help);
        }
        let configure_confluence = match confluence_prompt.prompt() {
            Ok(answer) => answer,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };

        let jira_help = existing
            .jira_base_url
            .as_deref()
            .map(|base_url| format!("現在の Jira Base URL: {base_url}"));
        let mut jira_prompt = Confirm::new("Jira を設定しますか?").with_default(!has_jira);
        if let Some(help) = jira_help.as_deref() {
            jira_prompt = jira_prompt.with_help_message(help);
        }
        let configure_jira = match jira_prompt.prompt() {
            Ok(answer) => answer,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };
        (configure_confluence, configure_jira)
    };

    if !configure_confluence && !configure_jira {
        println!("変更する項目がありません。");
        return Ok(());
    }

    let confluence_values = if configure_confluence {
        let mut base_url_prompt = Text::new("Base URL (e.g. https://confluence.example.com):")
            .with_validator(|value: &str| match url::Url::parse(value.trim()) {
                Ok(url)
                    if matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none() =>
                {
                    Ok(Validation::Valid)
                }
                Ok(_) => Ok(Validation::Invalid(
                    "http:// または https://host/path 形式の URL を入力してください（認証情報・クエリ不可）"
                        .into(),
                )),
                Err(_) => Ok(Validation::Invalid("有効な URL を入力してください".into())),
            });
        if let Some(base_url) = existing.base_url.as_deref() {
            base_url_prompt = base_url_prompt.with_default(base_url);
        }
        let base_url = match base_url_prompt.prompt() {
            Ok(value) => value.trim().to_string(),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };

        let api_path_default = existing.api_path.as_deref().unwrap_or("/rest/api");
        let api_path = match Text::new("API path:")
            .with_default(api_path_default)
            .with_validator(|value: &str| {
                let value = value.trim();
                if value.is_empty() {
                    Ok(Validation::Invalid("API パスを入力してください".into()))
                } else if !value.starts_with('/') {
                    Ok(Validation::Invalid("API パスは / で始めてください".into()))
                } else {
                    Ok(Validation::Valid)
                }
            })
            .prompt()
        {
            Ok(value) => value.trim().to_string(),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };

        let allowed_spaces_default = existing
            .allowed_spaces
            .as_ref()
            .map(|spaces| spaces.join(","));
        let mut allowed_spaces_prompt = Text::new(
            "Allowed spaces (comma-separated, e.g. DEV,ARCH — leave blank to allow all):",
        );
        if let Some(default) = allowed_spaces_default.as_deref() {
            allowed_spaces_prompt = allowed_spaces_prompt.with_default(default);
        }
        let allowed_spaces = match allowed_spaces_prompt.prompt_skippable() {
            Ok(Some(value)) => {
                let spaces = value
                    .split(',')
                    .map(|space| space.trim().to_string())
                    .filter(|space| !space.is_empty())
                    .collect::<Vec<_>>();
                (!spaces.is_empty()).then_some(spaces)
            }
            Ok(None) => None,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };

        let mut default_space_prompt =
            Text::new("Default space key (optional — leave blank to skip):");
        if let Some(default) = existing.default_space.as_deref() {
            default_space_prompt = default_space_prompt.with_default(default);
        }
        let default_space = match default_space_prompt.prompt_skippable() {
            Ok(Some(value)) if !value.trim().is_empty() => Some(value.trim().to_string()),
            Ok(_) => None,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };
        Some((base_url, api_path, allowed_spaces, default_space))
    } else {
        None
    };

    let jira_values = if configure_jira {
        let mut base_url_prompt = Text::new("Jira Base URL (e.g. https://jira.example.com):")
            .with_validator(|value: &str| match url::Url::parse(value.trim()) {
                Ok(url)
                    if matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none() =>
                {
                    Ok(Validation::Valid)
                }
                Ok(_) => Ok(Validation::Invalid(
                    "http:// または https://host/path 形式の URL を入力してください（認証情報・クエリ不可）"
                        .into(),
                )),
                Err(_) => Ok(Validation::Invalid("有効な URL を入力してください".into())),
            });
        if let Some(base_url) = existing.jira_base_url.as_deref() {
            base_url_prompt = base_url_prompt.with_default(base_url);
        }
        let base_url = match base_url_prompt.prompt() {
            Ok(value) => value.trim().to_string(),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };

        let api_path_default = existing.jira_api_path.as_deref().unwrap_or("/rest/api/2");
        let api_path = match Text::new("Jira API path:")
            .with_default(api_path_default)
            .with_validator(|value: &str| {
                let value = value.trim();
                if value.is_empty() {
                    Ok(Validation::Invalid("API パスを入力してください".into()))
                } else if !value.starts_with('/') {
                    Ok(Validation::Invalid("API パスは / で始めてください".into()))
                } else {
                    Ok(Validation::Valid)
                }
            })
            .prompt()
        {
            Ok(value) => value.trim().to_string(),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };

        let allowed_projects_default = existing
            .jira_allowed_projects
            .as_ref()
            .map(|projects| projects.join(","));
        let mut allowed_projects_prompt =
            Text::new("Allowed projects (comma-separated — leave blank to allow all):");
        if let Some(default) = allowed_projects_default.as_deref() {
            allowed_projects_prompt = allowed_projects_prompt.with_default(default);
        }
        let allowed_projects = match allowed_projects_prompt.prompt_skippable() {
            Ok(Some(value)) => {
                let projects = value
                    .split(',')
                    .map(|project| project.trim().to_string())
                    .filter(|project| !project.is_empty())
                    .collect::<Vec<_>>();
                (!projects.is_empty()).then_some(projects)
            }
            Ok(None) => None,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };

        let mut default_project_prompt = Text::new("Default project key (optional):");
        if let Some(default) = existing.jira_default_project.as_deref() {
            default_project_prompt = default_project_prompt.with_default(default);
        }
        let default_project = match default_project_prompt.prompt_skippable() {
            Ok(Some(value)) if !value.trim().is_empty() => Some(value.trim().to_string()),
            Ok(_) => None,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };
        Some((base_url, api_path, allowed_projects, default_project))
    } else {
        None
    };

    let update_common_settings = if profile_exists {
        match Confirm::new("共通設定 (default_limit / max_limit / max_page_chars) を変更しますか?")
            .with_default(false)
            .prompt()
        {
            Ok(answer) => answer,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        }
    } else {
        true
    };
    let common_values = if update_common_settings {
        let default_limit = match CustomType::<u32>::new("Default result limit:")
            .with_default(existing.default_limit.unwrap_or(10))
            .with_parser(&|value: &str| value.trim().parse::<u32>().map_err(|_| ()))
            .with_error_message("正の整数を入力してください")
            .with_validator(|&value: &u32| {
                if value >= 1 {
                    Ok(Validation::Valid)
                } else {
                    Ok(Validation::Invalid("1 以上の値を入力してください".into()))
                }
            })
            .prompt()
        {
            Ok(value) => value,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };
        let max_limit = match CustomType::<u32>::new("Maximum result limit:")
            .with_default(existing.max_limit.unwrap_or(50))
            .with_parser(&|value: &str| value.trim().parse::<u32>().map_err(|_| ()))
            .with_error_message("正の整数を入力してください")
            .with_validator(move |&value: &u32| {
                if value >= default_limit {
                    Ok(Validation::Valid)
                } else {
                    Ok(Validation::Invalid(
                        format!(
                            "{} 以上の値を入力してください (>= default_limit)",
                            default_limit
                        )
                        .into(),
                    ))
                }
            })
            .prompt()
        {
            Ok(value) => value,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        };
        let max_page_chars =
            match CustomType::<usize>::new("Maximum page content length (characters):")
                .with_default(existing.max_page_chars.unwrap_or(50_000))
                .with_parser(&|value: &str| value.trim().parse::<usize>().map_err(|_| ()))
                .with_error_message("正の整数を入力してください")
                .with_validator(|&value: &usize| {
                    if value >= 1_000 {
                        Ok(Validation::Valid)
                    } else {
                        Ok(Validation::Invalid(
                            "1000 以上の値を入力してください".into(),
                        ))
                    }
                })
                .prompt()
            {
                Ok(value) => value,
                Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                    println!("\nキャンセルされました");
                    return Ok(());
                }
                Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
            };
        Some((default_limit, max_limit, max_page_chars))
    } else {
        None
    };

    let mut merged = existing;
    if let Some((base_url, api_path, allowed_spaces, default_space)) = confluence_values {
        merged.base_url = Some(base_url);
        merged.api_path = Some(api_path);
        merged.allowed_spaces = allowed_spaces;
        merged.default_space = default_space;
    }
    if let Some((base_url, api_path, allowed_projects, default_project)) = jira_values {
        merged.jira_base_url = Some(base_url);
        merged.jira_api_path = Some(api_path);
        merged.jira_allowed_projects = allowed_projects;
        merged.jira_default_project = default_project;
    }
    if let Some((default_limit, max_limit, max_page_chars)) = common_values {
        merged.default_limit = Some(default_limit);
        merged.max_limit = Some(max_limit);
        merged.max_page_chars = Some(max_page_chars);
    }

    save_profile_to_path(profile_name, &merged, &config_path)?;
    println!();
    println!("設定を {} に保存しました。", config_path.display());

    if configure_confluence {
        match Confirm::new("APIトークンをキーリングに保存しますか?")
            .with_default(true)
            .with_help_message("Noの場合は環境変数 CONFLUENCE_TOKEN で設定してください")
            .prompt()
        {
            Ok(true) => {
                let token = match Password::new("API Token:").prompt() {
                    Ok(token) => token,
                    Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                        println!("\nキャンセルされました。(トークンは環境変数 CONFLUENCE_TOKEN で設定してください)");
                        return Ok(());
                    }
                    Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
                };
                if token.trim().is_empty() {
                    return Err(ConfluenceError::ConfigError(
                        "Token must not be empty.".to_string(),
                    ));
                }
                store_token_in_keyring(profile_name, &token)?;
                println!("トークンをキーリングに保存しました。");
            }
            Ok(false) => println!("(トークンは環境変数 CONFLUENCE_TOKEN で設定してください)"),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました。(トークンは環境変数 CONFLUENCE_TOKEN で設定してください)");
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        }
    }
    if configure_jira {
        match Confirm::new("Jira APIトークンをキーリングに保存しますか?")
            .with_default(true)
            .with_help_message("Noの場合は環境変数 JIRA_TOKEN で設定してください")
            .prompt()
        {
            Ok(true) => {
                let token = match Password::new("Jira API Token:").prompt() {
                    Ok(token) => token,
                    Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                        println!("\nキャンセルされました。(トークンは環境変数 JIRA_TOKEN で設定してください)");
                        return Ok(());
                    }
                    Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
                };
                if token.trim().is_empty() {
                    return Err(ConfluenceError::ConfigError(
                        "Token must not be empty.".to_string(),
                    ));
                }
                store_jira_token_in_keyring(profile_name, &token)?;
                println!("Jira トークンをキーリングに保存しました。");
            }
            Ok(false) => println!("(トークンは環境変数 JIRA_TOKEN で設定してください)"),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!(
                    "\nキャンセルされました。(トークンは環境変数 JIRA_TOKEN で設定してください)"
                );
            }
            Err(error) => return Err(ConfluenceError::ConfigError(error.to_string())),
        }
    }

    Ok(())
}

// ── skill command ─────────────────────────────────────────────────────────────

fn run_skill(args: cli::SkillArgs) -> Result<(), ConfluenceError> {
    match args.command {
        SkillSubcommand::Install { force } => {
            let skills_dir = skill::default_skills_dir().ok_or_else(|| {
                ConfluenceError::SkillError(
                    "cannot determine home directory; set HOME and retry".to_string(),
                )
            })?;
            for bundled in skill::BUNDLED_SKILLS {
                let (path, outcome) =
                    skill::install_skill(&skills_dir, bundled.name, bundled.content, force)?;
                match outcome {
                    skill::InstallOutcome::Installed | skill::InstallOutcome::Overwritten => {
                        println!("{}: Installed skill to {}", bundled.name, path.display());
                    }
                    skill::InstallOutcome::AlreadyUpToDate => {
                        println!(
                            "{}: Skill at {} is already up to date.",
                            bundled.name,
                            path.display()
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_search_sources_defaults_to_both_backends_for_a_query() {
        assert_eq!(
            plan_search_sources(None, true, false, false, false).unwrap(),
            SearchPlan {
                confluence: true,
                jira: true,
            }
        );
    }

    #[test]
    fn plan_search_sources_rejects_a_search_without_query_or_jira_filters() {
        assert!(matches!(
            plan_search_sources(None, false, false, false, false),
            Err(ConfluenceError::NoSearchCriteria)
        ));
    }

    #[test]
    fn plan_search_sources_routes_filter_only_searches_to_jira() {
        assert_eq!(
            plan_search_sources(None, false, false, false, true).unwrap(),
            SearchPlan {
                confluence: false,
                jira: true,
            }
        );
    }

    #[test]
    fn plan_search_sources_requires_a_query_when_all_is_explicit_for_jira_filters() {
        assert!(matches!(
            plan_search_sources(Some(SearchSource::All), false, false, false, true),
            Err(ConfluenceError::QueryRequiredForConfluence)
        ));
    }

    #[test]
    fn plan_search_sources_requires_a_query_when_confluence_flags_are_present() {
        assert!(matches!(
            plan_search_sources(None, false, false, true, true),
            Err(ConfluenceError::QueryRequiredForConfluence)
        ));
    }

    #[test]
    fn plan_search_sources_rejects_confluence_flags_when_jira_is_explicit() {
        assert!(matches!(
            plan_search_sources(Some(SearchSource::Jira), true, false, true, false),
            Err(ConfluenceError::InvalidArguments(message))
                if message == "--space/--in apply to Confluence but --source jira excludes it"
        ));
    }

    #[test]
    fn plan_search_sources_rejects_jira_flags_when_confluence_is_explicit() {
        assert!(matches!(
            plan_search_sources(Some(SearchSource::Confluence), true, false, false, true),
            Err(ConfluenceError::InvalidArguments(message))
                if message == "--project/--status/--assignee/--reporter/--type apply to Jira but --source confluence excludes it"
        ));
    }

    #[test]
    fn plan_search_sources_honors_explicit_jira_for_a_query() {
        assert_eq!(
            plan_search_sources(Some(SearchSource::Jira), true, false, false, false).unwrap(),
            SearchPlan {
                confluence: false,
                jira: true,
            }
        );
    }

    #[test]
    fn plan_search_sources_routes_label_only_search_to_both_backends() {
        assert_eq!(
            plan_search_sources(None, false, true, false, false).unwrap(),
            SearchPlan {
                confluence: true,
                jira: true,
            }
        );
    }

    #[test]
    fn plan_search_sources_allows_label_only_confluence_search() {
        assert_eq!(
            plan_search_sources(Some(SearchSource::Confluence), false, true, false, false).unwrap(),
            SearchPlan {
                confluence: true,
                jira: false,
            }
        );
    }

    #[test]
    fn plan_search_sources_allows_label_only_jira_search() {
        assert_eq!(
            plan_search_sources(Some(SearchSource::Jira), false, true, false, false).unwrap(),
            SearchPlan {
                confluence: false,
                jira: true,
            }
        );
    }
}
