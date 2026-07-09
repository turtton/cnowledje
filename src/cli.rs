use clap::{Args, Parser, Subcommand};

use cnowledje::types::{IssueFormat, PageFormat, SearchIn};

const MAIN_AFTER_HELP: &str = "\
TYPICAL WORKFLOW:
  1. Find candidate pages:   cnowledje search \"<keywords>\" --space <KEY> --json
  2. Read the chosen page:   cnowledje page <ID>
  3. Find candidate issues:  cnowledje jira search \"<keywords>\" --project <KEY> --json
  4. Read the chosen issue:  cnowledje jira issue <KEY>
  5. Cite the source (title/key, URL, last-modified/updated) in your answer.

Run `cnowledje <command> --help` for per-command options and examples.
See `cnowledje config --help` for configuration and token management.
See `cnowledje skill install --help` to install the bundled agent skill.";

// NOTE: The JSON shape documented below must be kept in sync with SearchOutput / SearchResultOutput
// in src/models.rs. Update both when the output types change.
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
    /// Manage the bundled agent skill.
    Skill(SkillArgs),
    /// Read-only access to Jira issues.
    Jira(JiraArgs),
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

// ── jira ──────────────────────────────────────────────────────────────────────

// NOTE: The JSON shape documented below must be kept in sync with JiraSearchOutput /
// JiraSearchResultOutput in src/models.rs. Update both when the output types change.
const JIRA_SEARCH_AFTER_HELP: &str = "\
EXAMPLES:
  # Search by keywords only
  cnowledje jira search \"redis timeout\" --project DEV --json

  # Filter by status (repeatable; OR semantics)
  cnowledje jira search \"login\" --project DEV --status \"In Progress\" --status Open

  # Filter by assignee, no keywords
  cnowledje jira search --project DEV --assignee jdoe

  # Filters only (query omitted)
  cnowledje jira search --project DEV --status Open --type Bug

NOTES:
  * --project is required unless jira_default_project is configured.
  * Queries are JQL generated internally; raw JQL input is not supported.
    The generated JQL is echoed back in the \"jql\" field of the output.
  * At least one of the query or a filter flag (--status/--assignee/
    --reporter/--type/--label) must be given.
  * --limit is capped by the configured max_limit (default 50).
  * --json output shape:
      { \"query\", \"projects\", \"jql\", \"total\",
        \"results\": [ { \"key\", \"summary\", \"status\", \"issue_type\",
                       \"priority\", \"assignee\", \"project_key\", \"url\",
                       \"updated\" } ] }";

const JIRA_ISSUE_AFTER_HELP: &str = "\
EXAMPLES:
  # Print issue content as Markdown (default), including comments
  cnowledje jira issue PROJ-123

  # Fetch by URL instead of a key
  cnowledje jira issue \"https://jira.example.com/browse/PROJ-123\"

  # Get structured JSON
  cnowledje jira issue PROJ-123 --json

  # Limit the combined description+comments length
  cnowledje jira issue PROJ-123 --max-chars 10000

NOTES:
  * description and comments share a single --max-chars budget, further
    capped by the configured max_page_chars (the smaller value wins).
    Comments dropped once the budget runs out are reported via the
    omitted_comments count.
  * Only /browse/<KEY> issue URLs are supported.";

#[derive(Args)]
pub struct JiraArgs {
    #[command(subcommand)]
    pub command: JiraSubcommand,
}

#[derive(Subcommand)]
pub enum JiraSubcommand {
    /// Search Jira issues with keywords and filters.
    Search(JiraSearchArgs),
    /// Retrieve a Jira issue by key or URL (includes comments).
    Issue(JiraIssueArgs),
}

#[derive(Args)]
#[command(after_long_help = JIRA_SEARCH_AFTER_HELP)]
pub struct JiraSearchArgs {
    /// Search keywords (matched against summary, description, and comments).
    /// Optional if at least one filter flag is given.
    pub query: Option<String>,

    /// Project key(s). May be repeated: --project DEV --project OPS
    #[arg(long = "project", short = 'p')]
    pub projects: Vec<String>,

    /// Filter by status name. May be repeated (OR).
    #[arg(long)]
    pub status: Vec<String>,

    /// Filter by assignee username.
    #[arg(long)]
    pub assignee: Option<String>,

    /// Filter by reporter username.
    #[arg(long)]
    pub reporter: Option<String>,

    /// Filter by issue type name. May be repeated (OR).
    #[arg(long = "type", value_name = "TYPE")]
    pub issue_types: Vec<String>,

    /// Filter by label. May be repeated (OR).
    #[arg(long = "label")]
    pub labels: Vec<String>,

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

#[derive(Args)]
#[command(after_long_help = JIRA_ISSUE_AFTER_HELP)]
pub struct JiraIssueArgs {
    /// Issue key (e.g. PROJ-123) or a Jira issue URL containing /browse/<KEY>.
    pub issue_key_or_url: String,

    /// Output format.
    #[arg(long, value_enum, default_value = "markdown")]
    pub format: IssueFormat,

    /// Shorthand for --format json.
    #[arg(long, conflicts_with = "format")]
    pub json: bool,

    /// Maximum content length in characters (description + comments combined).
    #[arg(long, default_value = "50000")]
    pub max_chars: usize,

    /// Use a specific configuration profile.
    #[arg(long)]
    pub profile: Option<String>,
}

impl JiraIssueArgs {
    /// Resolve the effective format, letting --json override --format.
    pub fn effective_format(&self) -> IssueFormat {
        if self.json {
            IssueFormat::Json
        } else {
            self.format.clone()
        }
    }
}

// ── skill ─────────────────────────────────────────────────────────────────────

const SKILL_AFTER_HELP: &str = "\
EXAMPLES:
  # Install all bundled skills to the default location
  # (~/.agents/skills/confluence-lookup/SKILL.md, ~/.agents/skills/jira-lookup/SKILL.md)
  cnowledje skill install

  # Overwrite existing files (e.g. after upgrading cnowledje)
  cnowledje skill install --force

NOTES:
  * Installs every bundled skill (confluence-lookup, jira-lookup); each
    SKILL.md is embedded in the binary at build time.
  * If a destination SKILL.md already exists with different content, the
    command aborts with an error on that skill to protect local edits.
    Re-run with --force to overwrite; already-installed skills before the
    failing one are not rolled back.
  * After upgrading cnowledje, an embedded SKILL.md may differ from the
    previously installed file.  Use --force to update it.";

#[derive(Args)]
#[command(after_long_help = SKILL_AFTER_HELP)]
pub struct SkillArgs {
    #[command(subcommand)]
    pub command: SkillSubcommand,
}

#[derive(Subcommand)]
pub enum SkillSubcommand {
    /// Install the confluence-lookup skill to ~/.agents/skills.
    Install {
        /// Overwrite an existing SKILL.md even if the content differs.
        #[arg(long)]
        force: bool,
    },
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
        /// Manage the Jira token instead of the Confluence token.
        #[arg(long)]
        jira: bool,
    },
    /// Remove the token for the given profile from the system keyring.
    Delete {
        /// Profile to remove the token for.
        #[arg(long, value_parser = parse_profile_name)]
        profile: Option<String>,
        /// Manage the Jira token instead of the Confluence token.
        #[arg(long)]
        jira: bool,
    },
}
