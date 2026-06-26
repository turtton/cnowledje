use reqwest::{
    header::{self, HeaderMap, HeaderValue},
    Client, StatusCode,
};
use url::Url;

use crate::error::ConfluenceError;
use crate::models::{PageResponse, SearchResponse};

pub struct ConfluenceClient {
    client: Client,
    base_url: Url,
    api_path: String,
}

impl ConfluenceClient {
    /// Build a new client with a Bearer token.
    ///
    /// The token value is marked sensitive so it will not appear in debug
    /// output from `reqwest`.
    pub fn new(base_url: &str, api_path: &str, token: &str) -> Result<Self, ConfluenceError> {
        let mut auth_value = HeaderValue::from_str(&format!("Bearer {}", token))
            .map_err(|_| ConfluenceError::ConfigError("invalid token: contains non-ASCII bytes".to_string()))?;
        auth_value.set_sensitive(true);

        let mut default_headers = HeaderMap::new();
        default_headers.insert(header::AUTHORIZATION, auth_value);
        default_headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/json"),
        );

        let client = Client::builder()
            .default_headers(default_headers)
            .build()
            .map_err(ConfluenceError::RequestError)?;

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

    /// Search pages using a pre-built CQL string.
    pub async fn search(&self, cql: &str, limit: u32) -> Result<SearchResponse, ConfluenceError> {
        let url = self.api_url("/content/search")?;
        let response = self
            .client
            .get(url)
            .query(&[
                ("cql", cql.to_string()),
                ("limit", limit.to_string()),
                ("expand", "space,version,_links".to_string()),
            ])
            .send()
            .await?;

        handle_response(response).await
    }

    /// Retrieve a single page by numeric ID.
    pub async fn get_page(&self, id: &str) -> Result<PageResponse, ConfluenceError> {
        let url = self.api_url(&format!("/content/{}", id))?;
        let response = self
            .client
            .get(url)
            .query(&[("expand", "space,version,body.storage,_links")])
            .send()
            .await?;

        handle_response(response).await
    }
}

async fn handle_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, ConfluenceError> {
    match response.status() {
        s if s.is_success() => Ok(response.json::<T>().await?),
        StatusCode::UNAUTHORIZED => Err(ConfluenceError::Unauthorized),
        StatusCode::FORBIDDEN => Err(ConfluenceError::Forbidden),
        StatusCode::NOT_FOUND => Err(ConfluenceError::NotFound(
            "page or endpoint not found".to_string(),
        )),
        s => {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "(no body)".to_string());
            Err(ConfluenceError::HttpError {
                status: s.as_u16(),
                message: body,
            })
        }
    }
}
