mod cli;

use clap::Parser;
use std::collections::{HashMap, HashSet};

use cli::{Cli, Commands, ConfigSubcommand};
use cnowledje::client::ConfluenceClient;
use cnowledje::config::{load_config, resolve_spaces, validate_spaces};
use cnowledje::cql::{build_text_cql, build_title_cql, extract_page_id};
use cnowledje::error::ConfluenceError;
use cnowledje::format::{
    make_page_url, print_error_json, print_page_json, print_page_markdown,
    print_page_plain, print_page_storage_html, print_search_human, print_search_json,
};
use cnowledje::markdown;
use cnowledje::models::{PageOutput, SearchOutput, SearchResultOutput, NOTICE};
use cnowledje::models::SearchResult;
use cnowledje::types::{PageFormat, SearchIn};

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
    };

    std::process::exit(exit_code);
}

// ── search command ────────────────────────────────────────────────────────────

async fn run_search(args: cli::SearchArgs) -> Result<(), ConfluenceError> {
    let config = load_config(args.profile.as_deref())?;

    // Enforce limit
    if args.limit > config.max_limit {
        return Err(ConfluenceError::LimitExceeded {
            requested: args.limit,
            max: config.max_limit,
        });
    }

    let spaces = resolve_spaces(args.spaces, &config)?;
    validate_spaces(&spaces, &config)?;

    let client = ConfluenceClient::new(&config.base_url, &config.api_path, &config.token)?;

    // Internal fetch limit: fetch more to allow dedup and still meet user's limit.
    let internal_limit = std::cmp::min(args.limit * 2, config.max_limit);

    // -- Execute searches ------------------------------------------------------
    let (title_results, text_results) = match &args.search_in {
        SearchIn::Title => {
            let cql = build_title_cql(&spaces, &args.query);
            let resp = client.search(&cql, internal_limit).await?;
            (resp.results, vec![])
        }
        SearchIn::Text => {
            let cql = build_text_cql(&spaces, &args.query);
            let resp = client.search(&cql, internal_limit).await?;
            (vec![], resp.results)
        }
        SearchIn::Both => {
            let title_cql = build_title_cql(&spaces, &args.query);
            let text_cql = build_text_cql(&spaces, &args.query);
            // Run both requests concurrently.
            let (tr, xr) = tokio::try_join!(
                client.search(&title_cql, internal_limit),
                client.search(&text_cql, internal_limit)
            )?;
            (tr.results, xr.results)
        }
    };

    // -- Merge with dedup and matched_by tracking -----------------------------
    let mut seen: HashSet<String> = HashSet::new();
    // id → vec of matched_by strings
    let mut matched_by: HashMap<String, Vec<String>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for r in &title_results {
        if seen.insert(r.id.clone()) {
            order.push(r.id.clone());
        }
        matched_by
            .entry(r.id.clone())
            .or_default()
            .push("title".to_string());
    }
    for r in &text_results {
        if seen.insert(r.id.clone()) {
            order.push(r.id.clone());
        }
        matched_by
            .entry(r.id.clone())
            .or_default()
            .push("text".to_string());
    }

    // Build a lookup so we can retrieve results by id.
    let all_results: HashMap<String, &SearchResult> = title_results
        .iter()
        .chain(text_results.iter())
        .map(|r| (r.id.clone(), r))
        .collect();

    // Apply user limit.
    let output_results: Vec<SearchResultOutput> = order
        .iter()
        .take(args.limit as usize)
        .filter_map(|id| all_results.get(id).copied())
        .map(|r| {
            let url = make_page_url(
                &config.base_url,
                None,
                r.links.webui.as_deref(),
            );
            SearchResultOutput {
                id: r.id.clone(),
                title: r.title.clone(),
                space_key: r.space.key.clone(),
                space_name: r.space.name.clone(),
                url,
                last_modified: r.version.when.clone(),
                matched_by: matched_by
                    .get(&r.id)
                    .cloned()
                    .unwrap_or_default(),
                excerpt: r.excerpt.clone(),
            }
        })
        .collect();

    let output = SearchOutput {
        query: args.query.clone(),
        spaces,
        search_in: args.search_in.to_string(),
        results: output_results,
    };

    if args.json {
        print_search_json(&output)?;
    } else {
        print_search_human(&output);
    }

    Ok(())
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

    let content_markdown = markdown::html_to_markdown(html, effective_max);

    let output = PageOutput {
        id: page.id.clone(),
        title: page.title.clone(),
        space_key: page.space.key.clone(),
        url: url.clone(),
        last_modified: page.version.when.clone(),
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

// ── config check command ──────────────────────────────────────────────────────

async fn run_config(args: cli::ConfigArgs) -> Result<(), ConfluenceError> {
    let ConfigSubcommand::Check { profile } = args.command;

    let config = load_config(profile.as_deref())?;

    println!("Profile      : {}", profile.as_deref().unwrap_or("default"));
    println!("base_url     : {}", config.base_url);
    println!("api_path     : {}", config.api_path);
    println!("token        : [set]");
    println!(
        "allowed_spaces : {}",
        config
            .allowed_spaces
            .as_ref()
            .map(|v| v.join(", "))
            .unwrap_or_else(|| "(none – all spaces allowed)".to_string())
    );
    println!(
        "default_space  : {}",
        config
            .default_space
            .as_deref()
            .unwrap_or("(none)")
    );
    println!("default_limit  : {}", config.default_limit);
    println!("max_limit      : {}", config.max_limit);
    println!("max_page_chars : {}", config.max_page_chars);

    // Quick connectivity check: attempt a trivial search
    println!();
    print!("Checking API connectivity... ");
    let client = ConfluenceClient::new(&config.base_url, &config.api_path, &config.token)?;
    // Use a CQL that returns 0 results quickly
    match client.search("type = page AND title = \"__confluence_ro_check__\"", 1).await {
        Ok(_) => println!("OK"),
        Err(e) => {
            println!("FAILED");
            return Err(e);
        }
    }

    Ok(())
}
