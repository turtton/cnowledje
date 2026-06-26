use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfluenceError {
    #[error("CONFLUENCE_BASE_URL is not set")]
    MissingBaseUrl,

    #[error("CONFLUENCE_TOKEN is not set")]
    MissingToken,

    #[error("space \"{0}\" is not allowed by the current profile")]
    SpaceNotAllowed(String),

    #[error("no space specified and no default_space configured")]
    NoSpaceSpecified,

    #[error("HTTP {status}: {message}")]
    HttpError { status: u16, message: String },

    #[error("Unauthorized: check your CONFLUENCE_TOKEN")]
    Unauthorized,

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

    #[error("request error: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("URL parse error: {0}")]
    UrlError(#[from] url::ParseError),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
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
            ConfluenceError::Forbidden => "forbidden",
            ConfluenceError::NotFound(_) => "not_found",
            ConfluenceError::InvalidPageUrl(_) => "invalid_page_url",
            ConfluenceError::LimitExceeded { .. } => "limit_exceeded",
            ConfluenceError::ConfigError(_) => "config_error",
            ConfluenceError::RequestError(_) => "request_error",
            ConfluenceError::UrlError(_) => "url_error",
            ConfluenceError::JsonError(_) => "json_error",
        }
    }
}
