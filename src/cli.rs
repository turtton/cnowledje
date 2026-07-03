use clap::{Args, Parser, Subcommand};

use cnowledje::types::{PageFormat, SearchIn};

const MAIN_AFTER_HELP: &str = "\
TYPICAL WORKFLOW:
  1. Find candidate pages:   cnowledje search \"<keywords>\" --space <KEY> --json
  2. Read the chosen page:   cnowledje page <ID>
  3. Cite the source (title, URL, last-modified) in your answer.

Run `cnowledje <command> --help` for per-command options and examples.
See `cnowledje config --help` for configuration and token management.";

const SEARCH_AFTER_HELP: &str = "\
EXAMPLES:
  # Search title and body across the default space
  cnowledje search \"認証フロー\" --json

  # Restrict to title matches in a specific space
  cnowledje search \"Redis 設計\" --space DEV --in title --json

  # Search body text only
  cnowledje search \"デプロイ手順\" --space OPS --in text --json

  # Search across multiple spaces
  cnowledje search \"API仕様\" --space DEV --space ARCH --json

NOTES:
  * --space is required unless default_space is configured; omitting it
    without a default_space is an error.
  * --limit is capped by the configured max_limit (default 50).
  * --json output shape:
      { \"query\", \"spaces\", \"search_in\",
        \"results\": [ { \"id\", \"title\", \"space_key\", \"space_name\",
                       \"url\", \"last_modified\", \"matched_by\", \"excerpt\" } ] }
    last_modified and excerpt may be null depending on the Confluence API.
  * If a search returns 0 results, broaden it: shorten the keywords, use
    --in both, or try other spaces with --space <KEY>.";

const PAGE_AFTER_HELP: &str = "\
EXAMPLES:
  # Print page content as Markdown (default)
  cnowledje page 123456789

  # Get structured JSON
  cnowledje page 123456789 --json

  # Limit the content length
  cnowledje page 123456789 --max-chars 10000

  # Select Japanese content from sv-translation macros
  cnowledje page 123456789 --language ja

  # Fetch by URL instead of a numeric ID
  cnowledje page \"https://confluence.example.local/pages/viewpage.action?pageId=123456789\"

NOTES:
  * Markdown output always includes the title and URL as HTML comments;
    the last-modified date is included only when available.
  * --max-chars is bounded by the configured max_page_chars; the smaller
    value wins. Truncated output ends with [content truncated].
  * Supported URL forms: \"?pageId=<ID>\" and \"/pages/<ID>\".
    \"/display/SPACE/Title\" URLs are NOT supported — resolve the page ID
    via `search` first.";

#[derive(Parser)]
#[command(
    name = "cnowledje",
    about = "Read-only Confluence CLI for Server/Data Center",
    long_about = "cnowledje provides safe, read-only access to Confluence pages.\n\
                  It uses GET requests only and never performs write operations.\n\
                  Designed for use by developers and AI agents.",
    after_long_help = MAIN_AFTER_HELP,
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
#[command(after_long_help = SEARCH_AFTER_HELP)]
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
#[command(after_long_help = PAGE_AFTER_HELP)]
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
