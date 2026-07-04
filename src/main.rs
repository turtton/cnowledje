mod cli;

use clap::Parser;
use std::collections::{HashMap, HashSet};

use cli::{Cli, Commands, ConfigSubcommand, SkillSubcommand};
use cnowledje::client::ConfluenceClient;
use cnowledje::config::{
    default_config_path, delete_token_from_keyring, load_config, profile_exists, resolve_spaces,
    save_profile_to_path, store_token_in_keyring, validate_spaces, ProfileConfig, TokenSource,
};
use cnowledje::cql::{build_text_cql, build_title_cql, extract_page_id};
use cnowledje::error::ConfluenceError;
use cnowledje::format::{
    make_page_url, print_error_json, print_page_json, print_page_markdown, print_page_plain,
    print_page_storage_html, print_search_human, print_search_json,
};
use cnowledje::markdown;
use cnowledje::models::SearchResult;
use cnowledje::models::{PageOutput, SearchOutput, SearchResultOutput, NOTICE};
use cnowledje::skill;
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
        Commands::Skill(args) => match run_skill(args) {
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
            let url = make_page_url(&config.base_url, None, r.links.webui.as_deref());
            SearchResultOutput {
                id: r.id.clone(),
                title: r.title.clone(),
                space_key: r.space.key.clone(),
                space_name: r.space.name.clone(),
                url,
                last_modified: r.version.when.clone(),
                matched_by: matched_by.get(&r.id).cloned().unwrap_or_default(),
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

    let content_markdown =
        markdown::html_to_markdown(html, effective_max, args.language.as_deref());

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
    match args.command {
        ConfigSubcommand::Check { profile } => {
            let config = load_config(profile.as_deref())?;

            println!("Profile      : {}", profile.as_deref().unwrap_or("default"));
            println!("base_url     : {}", config.base_url);
            println!("api_path     : {}", config.api_path);
            let token_src = match config.token_source {
                TokenSource::Env => "[from env]",
                TokenSource::Keyring => "[from keyring]",
                _ => "[from unknown source]",
            };
            println!("token        : {}", token_src);
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
                config.default_space.as_deref().unwrap_or("(none)")
            );
            println!("default_limit  : {}", config.default_limit);
            println!("max_limit      : {}", config.max_limit);
            println!("max_page_chars : {}", config.max_page_chars);

            println!();
            print!("Checking API connectivity... ");
            let client = ConfluenceClient::new(&config.base_url, &config.api_path, &config.token)?;
            match client
                .search("type = page AND title = \"__confluence_ro_check__\"", 1)
                .await
            {
                Ok(_) => println!("OK"),
                Err(e) => {
                    println!("FAILED");
                    return Err(e);
                }
            }
        }
        ConfigSubcommand::Init { profile, force } => {
            run_config_init(profile, force)?;
        }
        ConfigSubcommand::Token(token_args) => {
            run_config_token(token_args)?;
        }
    }

    Ok(())
}

fn run_config_token(args: cli::TokenArgs) -> Result<(), ConfluenceError> {
    use inquire::Password;
    if std::env::var("CONFLUENCE_TOKEN")
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false)
    {
        eprintln!("Note: CONFLUENCE_TOKEN is set and will take precedence over the keyring token.");
    }
    match args.command {
        cli::TokenSubcommand::Set { profile } => {
            let profile_name = profile.as_deref().unwrap_or("default");
            let token = Password::new("API Token:")
                .prompt()
                .map_err(|e| ConfluenceError::ConfigError(e.to_string()))?;
            if token.trim().is_empty() {
                return Err(ConfluenceError::ConfigError(
                    "Token must not be empty.".to_string(),
                ));
            }
            store_token_in_keyring(profile_name, &token)?;
            println!("Token stored in keyring for profile '{}'.", profile_name);
        }
        cli::TokenSubcommand::Delete { profile } => {
            let profile_name = profile.as_deref().unwrap_or("default");
            delete_token_from_keyring(profile_name)?;
            println!("Token removed from keyring for profile '{}'.", profile_name);
        }
    }
    Ok(())
}

// ── config init command ───────────────────────────────────────────────────────

fn run_config_init(profile: String, force: bool) -> Result<(), ConfluenceError> {
    use inquire::error::InquireError;
    use inquire::validator::Validation;
    use inquire::{Confirm, CustomType, Password, Text};

    let profile_name = profile.as_str();

    let config_path = default_config_path().ok_or_else(|| {
        ConfluenceError::ConfigError("設定ファイルのパスを決定できません".to_string())
    })?;

    if !force {
        let exists = profile_exists(profile_name)?;
        if exists {
            let answer = Confirm::new(&format!(
                "Profile '{}' already exists. Overwrite?",
                profile_name
            ))
            .with_default(false)
            .prompt();
            match answer {
                Ok(true) => {}
                Ok(false) => {
                    println!("キャンセルされました");
                    return Ok(());
                }
                Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                    println!("\nキャンセルされました");
                    return Ok(());
                }
                Err(e) => return Err(ConfluenceError::ConfigError(e.to_string())),
            }
        }
    }

    println!(
        "プロファイル '{}' を初期化します。Ctrl-C または Esc でキャンセルできます。\n",
        profile_name
    );

    let base_url = match Text::new("Base URL (e.g. https://confluence.example.com):")
        .with_validator(|s: &str| match url::Url::parse(s.trim()) {
            Ok(u)
                if matches!(u.scheme(), "http" | "https")
                    && u.host_str().is_some()
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.query().is_none()
                    && u.fragment().is_none() =>
            {
                Ok(Validation::Valid)
            }
            Ok(_) => Ok(Validation::Invalid(
                "http:// または https://host/path 形式の URL を入力してください（認証情報・クエリ不可）"
                    .into(),
            )),
            Err(_) => Ok(Validation::Invalid(
                "有効な URL を入力してください".into(),
            )),
        })
        .prompt()
    {
        Ok(v) => v.trim().to_string(),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            println!("\nキャンセルされました");
            return Ok(());
        }
        Err(e) => return Err(ConfluenceError::ConfigError(e.to_string())),
    };

    let api_path = match Text::new("API path:")
        .with_default("/rest/api")
        .with_validator(|s: &str| {
            let t = s.trim();
            if t.is_empty() {
                Ok(Validation::Invalid("API パスを入力してください".into()))
            } else if !t.starts_with('/') {
                Ok(Validation::Invalid("API パスは / で始めてください".into()))
            } else {
                Ok(Validation::Valid)
            }
        })
        .prompt()
    {
        Ok(v) => v.trim().to_string(),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            println!("\nキャンセルされました");
            return Ok(());
        }
        Err(e) => return Err(ConfluenceError::ConfigError(e.to_string())),
    };

    let allowed_spaces: Option<Vec<String>> = match Text::new(
        "Allowed spaces (comma-separated, e.g. DEV,ARCH — leave blank to allow all):",
    )
    .prompt_skippable()
    {
        Ok(Some(s)) => {
            let spaces: Vec<String> = s
                .split(',')
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect();
            if spaces.is_empty() {
                None
            } else {
                Some(spaces)
            }
        }
        Ok(None) => None,
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            println!("\nキャンセルされました");
            return Ok(());
        }
        Err(e) => return Err(ConfluenceError::ConfigError(e.to_string())),
    };

    // 4. default_space (optional)
    let default_space: Option<String> =
        match Text::new("Default space key (optional — leave blank to skip):").prompt_skippable()
        {
            Ok(Some(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Ok(_) => None,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(e) => return Err(ConfluenceError::ConfigError(e.to_string())),
        };

    // 5. default_limit (u32, default 10)
    let default_limit: u32 = match CustomType::<u32>::new("Default result limit:")
        .with_default(10)
        .with_parser(&|s: &str| s.trim().parse::<u32>().map_err(|_| ()))
        .with_error_message("正の整数を入力してください")
        .with_validator(|&n: &u32| {
            if n >= 1 {
                Ok(Validation::Valid)
            } else {
                Ok(Validation::Invalid("1 以上の値を入力してください".into()))
            }
        })
        .prompt()
    {
        Ok(v) => v,
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            println!("\nキャンセルされました");
            return Ok(());
        }
        Err(e) => return Err(ConfluenceError::ConfigError(e.to_string())),
    };

    // 6. max_limit (u32, default 50, >= default_limit)
    let max_limit: u32 = match CustomType::<u32>::new("Maximum result limit:")
        .with_default(50)
        .with_parser(&|s: &str| s.trim().parse::<u32>().map_err(|_| ()))
        .with_error_message("正の整数を入力してください")
        .with_validator(move |&n: &u32| {
            if n >= default_limit {
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
        Ok(v) => v,
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            println!("\nキャンセルされました");
            return Ok(());
        }
        Err(e) => return Err(ConfluenceError::ConfigError(e.to_string())),
    };

    // 7. max_page_chars (usize, default 50000, min 1000)
    let max_page_chars: usize =
        match CustomType::<usize>::new("Maximum page content length (characters):")
            .with_default(50_000)
            .with_parser(&|s: &str| s.trim().parse::<usize>().map_err(|_| ()))
            .with_error_message("正の整数を入力してください")
            .with_validator(|&n: &usize| {
                if n >= 1_000 {
                    Ok(Validation::Valid)
                } else {
                    Ok(Validation::Invalid(
                        "1000 以上の値を入力してください".into(),
                    ))
                }
            })
            .prompt()
        {
            Ok(v) => v,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                println!("\nキャンセルされました");
                return Ok(());
            }
            Err(e) => return Err(ConfluenceError::ConfigError(e.to_string())),
        };

    let profile_config = ProfileConfig {
        base_url: Some(base_url),
        api_path: Some(api_path),
        allowed_spaces,
        default_space,
        default_limit: Some(default_limit),
        max_limit: Some(max_limit),
        max_page_chars: Some(max_page_chars),
    };

    let saved_path =
        save_profile_to_path(profile_name, &profile_config, &config_path).map(|()| config_path)?;

    println!();
    println!("設定を {} に保存しました。", saved_path.display());

    match Confirm::new("APIトークンをキーリングに保存しますか?")
        .with_default(true)
        .with_help_message("Noの場合は環境変数 CONFLUENCE_TOKEN で設定してください")
        .prompt()
    {
        Ok(true) => {
            let token = Password::new("API Token:")
                .prompt()
                .map_err(|e| ConfluenceError::ConfigError(e.to_string()))?;
            if token.trim().is_empty() {
                return Err(ConfluenceError::ConfigError(
                    "Token must not be empty.".to_string(),
                ));
            }
            store_token_in_keyring(profile_name, &token)?;
            println!("トークンをキーリングに保存しました。");
        }
        Ok(false) => {
            println!("(トークンは環境変数 CONFLUENCE_TOKEN で設定してください)");
        }
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            println!(
                "\nキャンセルされました。(トークンは環境変数 CONFLUENCE_TOKEN で設定してください)"
            );
        }
        Err(e) => return Err(ConfluenceError::ConfigError(e.to_string())),
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
            let (path, outcome) = skill::install_skill(&skills_dir, skill::SKILL_CONTENT, force)?;
            match outcome {
                skill::InstallOutcome::Installed | skill::InstallOutcome::Overwritten => {
                    println!("Installed skill to {}", path.display());
                }
                skill::InstallOutcome::AlreadyUpToDate => {
                    println!("Skill at {} is already up to date.", path.display());
                }
            }
        }
    }
    Ok(())
}
