use reqwest::Client;
use url::Url;

use crate::client::{build_http_client, handle_response};
use crate::error::ConfluenceError;
use crate::models::{JiraIssueResponse, JiraSearchResponse};

pub struct JiraClient {
    client: Client,
    base_url: Url,
    api_path: String,
}

impl JiraClient {
    /// Build a new client with a Bearer (PAT) token.
    pub fn new(base_url: &str, api_path: &str, token: &str) -> Result<Self, ConfluenceError> {
        let client = build_http_client(token)?;

        // Ensure the base URL ends without a trailing slash so joins work
        // predictably.
        let base_url_str = base_url.trim_end_matches('/');
        let base_url = Url::parse(base_url_str)?;

        Ok(Self {
            client,
            base_url,
            api_path: api_path.trim_end_matches('/').to_string(),
        })
    }

    fn api_url(&self, path: &str) -> Result<Url, ConfluenceError> {
        let full = format!("{}{}{}", self.base_url, self.api_path, path);
        Ok(Url::parse(&full)?)
    }

    /// Search issues using a pre-built JQL string.
    pub async fn search(
        &self,
        jql: &str,
        max_results: u32,
    ) -> Result<JiraSearchResponse, ConfluenceError> {
        let url = self.api_url("/search")?;
        let response = self
            .client
            .get(url)
            .query(&[
                ("jql", jql.to_string()),
                ("maxResults", max_results.to_string()),
                (
                    "fields",
                    "summary,status,issuetype,priority,assignee,project,updated".to_string(),
                ),
            ])
            .send()
            .await?;

        handle_response(response, ConfluenceError::JiraUnauthorized).await
    }

    /// Retrieve a single issue by key, with rendered (HTML) fields included.
    pub async fn get_issue(&self, key: &str) -> Result<JiraIssueResponse, ConfluenceError> {
        let url = self.api_url(&format!("/issue/{}", key))?;
        let response = self
            .client
            .get(url)
            .query(&[("expand", "renderedFields")])
            .send()
            .await?;

        handle_response(response, ConfluenceError::JiraUnauthorized).await
    }

    /// Check auth/connectivity against the Jira REST API.
    pub async fn check_connectivity(&self) -> Result<(), ConfluenceError> {
        let url = self.api_url("/myself")?;
        let response = self.client.get(url).send().await?;
        handle_response::<serde_json::Value>(response, ConfluenceError::JiraUnauthorized).await?;
        Ok(())
    }
}
