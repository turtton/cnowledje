use clap::{Args, Parser, Subcommand};

use cnowledje::types::{PageFormat, SearchIn};

#[derive(Parser)]
#[command(
    name = "cnowledje",
    about = "Read-only Confluence CLI for Server/Data Center",
    long_about = "cnowledje provides safe, read-only access to Confluence pages.\n\
                  It uses GET requests only and never performs write operations.\n\
                  Designed for use by developers and AI agents.",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Search Confluence pages by title and/or text.
    Search(SearchArgs),
    /// Retrieve a Confluence page by ID or URL.
    Page(PageArgs),
    /// Validate and display the current configuration.
    Config(ConfigArgs),
}

// ── search ────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct SearchArgs {
    /// Search query string.
    pub query: String,

    /// Space key(s) to search in. May be repeated: --space DEV --space ARCH
    #[arg(long = "space", short = 's')]
    pub spaces: Vec<String>,

    /// Where to search: title, text, or both (default).
    #[arg(long = "in", value_enum, default_value = "both")]
    pub search_in: SearchIn,

    /// Maximum number of results to return (default 10, max 50).
    #[arg(long, default_value = "10")]
    pub limit: u32,

    /// Output results as JSON.
    #[arg(long)]
    pub json: bool,

    /// Use a specific configuration profile.
    #[arg(long)]
    pub profile: Option<String>,
}

// ── page ──────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct PageArgs {
    /// Numeric page ID or a Confluence page URL.
    pub page_id_or_url: String,

    /// Output format.
    #[arg(long, value_enum, default_value = "markdown")]
    pub format: PageFormat,

    /// Shorthand for --format json.
    #[arg(long, conflicts_with = "format")]
    pub json: bool,

    /// Maximum content length in characters.
    #[arg(long, default_value = "50000")]
    pub max_chars: usize,

    /// Include metadata fields in markdown output.
    #[arg(long)]
    pub include_metadata: bool,

    /// Language code to select when the page contains sv-translation macros (e.g. ja, en).
    /// If omitted, the first sv-translation block is expanded.
    #[arg(long)]
    pub language: Option<String>,

    /// Use a specific configuration profile.
    #[arg(long)]
    pub profile: Option<String>,
}

impl PageArgs {
    /// Resolve the effective format, letting --json override --format.
    pub fn effective_format(&self) -> PageFormat {
        if self.json {
            PageFormat::Json
        } else {
            self.format.clone()
        }
    }
}

// ── config ────────────────────────────────────────────────────────────────────

fn parse_profile_name(s: &str) -> Result<String, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        Err("プロファイル名は空にできません".to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

#[derive(Subcommand)]
pub enum ConfigSubcommand {
    /// Check that the configuration is valid and the API is reachable.
    Check {
        /// Profile to check.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Interactively create or update a configuration profile.
    Init {
        /// Profile name to initialize (default: "default").
        #[arg(long, default_value = "default", value_parser = parse_profile_name)]
        profile: String,
        /// Overwrite existing profile without confirmation prompt.
        #[arg(long)]
        force: bool,
    },
    /// Manage the API token stored in the system keyring.
    Token(TokenArgs),
}

#[derive(Args)]
pub struct TokenArgs {
    #[command(subcommand)]
    pub command: TokenSubcommand,
}

#[derive(Subcommand)]
pub enum TokenSubcommand {
    /// Store a token in the system keyring for the given profile.
    Set {
        /// Profile to store the token for.
        #[arg(long, value_parser = parse_profile_name)]
        profile: Option<String>,
    },
    /// Remove the token for the given profile from the system keyring.
    Delete {
        /// Profile to remove the token for.
        #[arg(long, value_parser = parse_profile_name)]
        profile: Option<String>,
    },
}
