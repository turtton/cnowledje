use clap::{Args, Parser, Subcommand};

use cnowledje::types::{IssueFormat, PageFormat, SearchIn, SearchSource};

const MAIN_AFTER_HELP: &str = "\
TYPICAL WORKFLOW:
  1. Find candidates:      cnowledje search \"<keywords>\" --json
     (searches Confluence and Jira; restrict with --source confluence|jira)
  2. Read a page:          cnowledje page <ID>
  3. Read an issue:        cnowledje issue <KEY>
  4. Cite the source (title/key, URL, last-modified/updated) in your answer.

Run `cnowledje <command> --help` for per-command options and examples.
See `cnowledje config --help` for configuration and token management.
See `cnowledje skill install --help` to install the bundled agent skill.";

// NOTE: The JSON shape documented below must be kept in sync with UnifiedSearchOutput,
// SearchOutput, SearchResultOutput, JiraSearchOutput, and JiraSearchResultOutput in
// src/models.rs. Update both when the output types change.
const SEARCH_AFTER_HELP: &str = "\
EXAMPLES:
  # Search Confluence and Jira for matching keywords
  cnowledje search \"認証フロー\" --json

  # Restrict to Confluence title matches in a specific space
  cnowledje search \"Redis 設計\" --source confluence --space DEV --in title --json

  # Restrict to Jira and filter by status (repeatable; OR semantics)
  cnowledje search \"login\" --source jira --project DEV --status \"In Progress\" --status Open

  # Search Jira using filters only (query omitted)
  cnowledje search --project DEV --assignee jdoe --status Open --type Bug

  # Label-only search (no query) across Confluence and Jira
  cnowledje search --space DEV --project DEV --label api

  # Search across multiple Confluence spaces
  cnowledje search \"API仕様\" --space DEV --space ARCH --json

NOTES:
  * --source confluence, --source jira, or --source all selects the backend(s).
    Omitting --source requests both configured backends.
  * --space and --in apply only to Confluence. --project, --status, --assignee,
    --reporter, --type apply only to Jira. --label applies to both backends.
    Passing flags for a backend excluded by --source is an error.
  * Without a query, at least one Jira filter or --label is required. Filters-only searches
    automatically search Jira alone only when --source is omitted and no Confluence flag is given;
    --label also permits a label-only Confluence search. --in requires a search query.
  * A backend named by --source or with one of its flags is required to be
    configured. Unpinned, unconfigured backends may be skipped with a warning.
  * --space is required for Confluence unless default_space is configured;
    --project is required for Jira unless jira_default_project is configured.
  * Queries are generated internally as CQL/JQL; raw CQL and JQL are not supported.
  * --limit is capped by each participating backend's configured max_limit
    (default 50).
  * --json output shape:
      { \"query\", \"confluence\": { \"query\", \"spaces\", \"labels\", \"search_in\", \"returned\", \"has_more\", \"results\": [...] } | null,
        \"jira\": { \"query\", \"projects\", \"jql\", \"total\", \"returned\", \"has_more\", \"results\": [
          { ..., \"labels\", \"project_name\" }
        ] } | null }
    For label-only Confluence searches, \"query\" and \"search_in\" are null.
    Confluence and Jira result objects retain their existing fields; Jira search
    results additionally include \"labels\" and \"project_name\". The Jira \"jql\"
    field remains in JSON for compatibility, but JQL is not shown in human output.
    \"returned\" is the final number of results in the response. \"has_more\" is
    pagination metadata only; search does not fetch a next page. Confluence sets
    it from a participating leg's next link or excess deduplicated unique results;
    Jira sets it when \"total\" exceeds \"returned\". A backend that was not searched
    is represented as null.
";

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
  * Markdown output includes a Labels comment line when the page has labels.
  * --max-chars is bounded by the configured max_page_chars; the smaller
    value wins. Truncated output ends with [content truncated].
  * Supported URL forms: \"?pageId=<ID>\" and \"/pages/<ID>\".
    \"/display/SPACE/Title\" URLs are NOT supported — resolve the page ID
    via `search` first.";

#[derive(Parser)]
#[command(
    name = "cnowledje",
    about = "Read-only Confluence & Jira CLI for Server/Data Center",
    long_about = "cnowledje provides safe, read-only access to Confluence pages and Jira issues.\n\
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
    /// Search Confluence pages and Jira issues.
    Search(SearchArgs),
    /// Retrieve a Confluence page by ID or URL.
    Page(PageArgs),
    /// Retrieve a Jira issue by key or URL (includes comments).
    Issue(IssueArgs),
    /// Validate and display the current configuration.
    Config(ConfigArgs),
    /// Manage the bundled agent skill.
    Skill(SkillArgs),
}

// ── search ────────────────────────────────────────────────────────────────────

#[derive(Args)]
#[command(after_long_help = SEARCH_AFTER_HELP)]
pub struct SearchArgs {
    /// Search keywords. Optional if --label or at least one Jira filter is given.
    /// Without a query, --in cannot be used.
    pub query: Option<String>,

    /// Backend(s) to search (default: all configured backends).
    #[arg(long, value_enum)]
    pub source: Option<SearchSource>,

    /// Space key(s) to search in. May be repeated: --space DEV --space ARCH
    #[arg(long = "space", short = 's')]
    pub spaces: Vec<String>,

    /// Where to search in Confluence: title, text, or both (default).
    #[arg(long = "in", value_enum)]
    pub search_in: Option<SearchIn>,

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

    /// Filter by label (Confluence and Jira). May be repeated (OR).
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

// ── issue ────────────────────────────────────────────────────────────────────

const ISSUE_AFTER_HELP: &str = "\
EXAMPLES:
  # Print issue content as Markdown (default), including comments
  cnowledje issue PROJ-123

  # Fetch by URL instead of a key
  cnowledje issue \"https://jira.example.com/browse/PROJ-123\"

  # Get structured JSON
  cnowledje issue PROJ-123 --json

  # Limit the combined description+comments length
  cnowledje issue PROJ-123 --max-chars 10000

NOTES:
  * description and comments share a single --max-chars budget, further
    capped by the configured max_page_chars (the smaller value wins).
    Comments dropped once the budget runs out are reported via the
    omitted_comments count.
  * Only /browse/<KEY> issue URLs are supported.";

#[derive(Args)]
#[command(after_long_help = ISSUE_AFTER_HELP)]
pub struct IssueArgs {
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

impl IssueArgs {
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
        /// Configure the Confluence section (skips the interactive section prompt).
        #[arg(long)]
        confluence: bool,
        /// Configure the Jira section (skips the interactive section prompt).
        #[arg(long)]
        jira: bool,
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use cnowledje::types::SearchSource;

    use super::{Cli, Commands, ConfigSubcommand};

    #[test]
    fn parses_flattened_issue_command() {
        let cli = Cli::try_parse_from(["cnowledje", "issue", "PROJ-1"]).unwrap();

        match cli.command {
            Commands::Issue(args) => assert_eq!(args.issue_key_or_url, "PROJ-1"),
            _ => panic!("expected the flattened issue command"),
        }
    }

    #[test]
    fn parses_unified_jira_search_with_source_and_filter() {
        let cli = Cli::try_parse_from([
            "cnowledje",
            "search",
            "q",
            "--source",
            "jira",
            "--status",
            "Open",
        ])
        .unwrap();

        match cli.command {
            Commands::Search(args) => {
                assert_eq!(args.source, Some(SearchSource::Jira));
                assert_eq!(args.status, ["Open"]);
            }
            _ => panic!("expected the unified search command"),
        }
    }

    #[test]
    fn rejects_removed_jira_command_hierarchy() {
        assert!(Cli::try_parse_from(["cnowledje", "jira", "search", "q"]).is_err());
    }

    #[test]
    fn config_init_accepts_jira_section_flag_and_rejects_force() {
        let cli = Cli::try_parse_from(["cnowledje", "config", "init", "--jira"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Config(config)
                if matches!(config.command, ConfigSubcommand::Init { jira: true, .. })
        ));

        assert!(Cli::try_parse_from(["cnowledje", "config", "init", "--force"]).is_err());
    }
}
