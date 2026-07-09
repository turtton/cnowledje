use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfluenceError {
    #[error("CONFLUENCE_BASE_URL is not set")]
    MissingBaseUrl,

    #[error("CONFLUENCE_TOKEN is not set and no token found in system keyring")]
    MissingToken,

    #[error("space \"{0}\" is not allowed by the current profile")]
    SpaceNotAllowed(String),

    #[error("no space specified and no default_space configured")]
    NoSpaceSpecified,

    #[error("HTTP {status}: {message}")]
    HttpError { status: u16, message: String },

    #[error(
        "Unauthorized: check your Confluence token (CONFLUENCE_TOKEN env var or system keyring)"
    )]
    Unauthorized,

    #[error("Unauthorized: check your Jira token (JIRA_TOKEN env var or system keyring)")]
    JiraUnauthorized,

    #[error("Forbidden: the account does not have permission to access this resource")]
    Forbidden,

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("could not extract page ID from \"{0}\": provide a numeric ID or a URL containing pageId= or /pages/<id>")]
    InvalidPageUrl(String),

    #[error("limit {requested} exceeds maximum allowed {max}")]
    LimitExceeded { requested: u32, max: u32 },

    #[error("config error: {0}")]
    ConfigError(String),

    #[error("keyring error: {0}")]
    KeyringError(String),

    #[error("request error: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("URL parse error: {0}")]
    UrlError(#[from] url::ParseError),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("{0}")]
    SkillError(String),

    #[error("JIRA_BASE_URL is not set and jira_base_url is not configured")]
    MissingJiraBaseUrl,

    #[error("JIRA_TOKEN is not set and no Jira token found in system keyring")]
    MissingJiraToken,

    #[error("project \"{0}\" is not allowed by the current profile")]
    ProjectNotAllowed(String),

    #[error("no project specified and no jira_default_project configured")]
    NoProjectSpecified,

    #[error("could not extract issue key from \"{0}\": provide a key like PROJ-123 or a URL containing /browse/<KEY>")]
    InvalidIssueKey(String),

    #[error("specify a search query or at least one filter (--status/--assignee/--reporter/--type/--label)")]
    NoSearchCriteria,
}

impl ConfluenceError {
    /// Short machine-readable kind string for JSON error output.
    pub fn kind(&self) -> &'static str {
        match self {
            ConfluenceError::MissingBaseUrl => "missing_config",
            ConfluenceError::MissingToken => "missing_config",
            ConfluenceError::SpaceNotAllowed(_) => "space_not_allowed",
            ConfluenceError::NoSpaceSpecified => "no_space_specified",
            ConfluenceError::HttpError { .. } => "http_error",
            ConfluenceError::Unauthorized => "unauthorized",
            ConfluenceError::JiraUnauthorized => "unauthorized",
            ConfluenceError::Forbidden => "forbidden",
            ConfluenceError::NotFound(_) => "not_found",
            ConfluenceError::InvalidPageUrl(_) => "invalid_page_url",
            ConfluenceError::LimitExceeded { .. } => "limit_exceeded",
            ConfluenceError::ConfigError(_) => "config_error",
            ConfluenceError::KeyringError(_) => "keyring_error",
            ConfluenceError::RequestError(_) => "request_error",
            ConfluenceError::UrlError(_) => "url_error",
            ConfluenceError::JsonError(_) => "json_error",
            ConfluenceError::SkillError(_) => "skill_error",
            ConfluenceError::MissingJiraBaseUrl => "missing_config",
            ConfluenceError::MissingJiraToken => "missing_config",
            ConfluenceError::ProjectNotAllowed(_) => "project_not_allowed",
            ConfluenceError::NoProjectSpecified => "no_project_specified",
            ConfluenceError::InvalidIssueKey(_) => "invalid_issue_key",
            ConfluenceError::NoSearchCriteria => "no_search_criteria",
        }
    }
}
