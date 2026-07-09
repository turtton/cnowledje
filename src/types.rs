use clap::ValueEnum;
use serde::Serialize;

/// Where to search in Confluence pages.
#[derive(ValueEnum, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchIn {
    Both,
    Title,
    Text,
}

impl std::fmt::Display for SearchIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchIn::Both => write!(f, "both"),
            SearchIn::Title => write!(f, "title"),
            SearchIn::Text => write!(f, "text"),
        }
    }
}

/// Output format for the page command.
#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum PageFormat {
    Markdown,
    Json,
    #[value(name = "storage-html")]
    StorageHtml,
    Plain,
}

/// Output format for the jira issue command.
#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum IssueFormat {
    Markdown,
    Json,
    Plain,
}
