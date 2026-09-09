use reqwest::{
    header::{self, HeaderMap, HeaderValue},
    Client, StatusCode,
};
use url::Url;

use crate::error::ConfluenceError;
use crate::models::{PageListResponse, PageResponse, SearchResponse, SpaceWithHomepage};

/// Build a `reqwest::Client` with the Bearer auth header set for `token`.
///
/// The token value is marked sensitive so it will not appear in debug
/// output from `reqwest`.
pub(crate) fn build_http_client(token: &str) -> Result<Client, ConfluenceError> {
    let mut auth_value = HeaderValue::from_str(&format!("Bearer {}", token)).map_err(|_| {
        ConfluenceError::ConfigError("invalid token: contains non-ASCII bytes".to_string())
    })?;
    auth_value.set_sensitive(true);

    let mut default_headers = HeaderMap::new();
    default_headers.insert(header::AUTHORIZATION, auth_value);
    default_headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));

    Client::builder()
        .default_headers(default_headers)
        .build()
        .map_err(ConfluenceError::RequestError)
}

pub struct ConfluenceClient {
    client: Client,
    base_url: Url,
    api_path: String,
}

impl ConfluenceClient {
    /// Build a new client with a Bearer token.
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
        let full = format!(
            "{}{}{}",
            self.base_url.as_str().trim_end_matches('/'),
            self.api_path,
            path
        );
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
                ("expand", "space,version,metadata.labels,_links".to_string()),
            ])
            .send()
            .await?;

        handle_response(response, ConfluenceError::Unauthorized).await
    }

    /// Retrieve direct children only; pagination is controlled by the caller.
    pub async fn get_children(
        &self,
        id: &str,
        start: u32,
        limit: u32,
    ) -> Result<PageListResponse, ConfluenceError> {
        let id = crate::cql::extract_page_id(id)?;
        self.list_pages(
            self.api_url(&format!("/content/{id}/child/page"))?,
            start,
            limit,
        )
        .await
    }

    /// All current pages in a space, including its homepage and orphaned pages.
    pub async fn get_space_pages(
        &self,
        space: &str,
        start: u32,
        limit: u32,
    ) -> Result<PageListResponse, ConfluenceError> {
        let mut url = self.api_url("/content")?;
        url.query_pairs_mut()
            .append_pair("spaceKey", space)
            .append_pair("type", "page")
            .append_pair("status", "current");
        self.list_pages(url, start, limit).await
    }

    async fn list_pages(
        &self,
        url: Url,
        start: u32,
        limit: u32,
    ) -> Result<PageListResponse, ConfluenceError> {
        let response = self
            .client
            .get(url)
            .query(&[
                ("start", start.to_string()),
                ("limit", limit.to_string()),
                ("expand", "children.page,_links".to_string()),
            ])
            .send()
            .await?;
        handle_response(response, ConfluenceError::Unauthorized).await
    }

    pub async fn get_homepage(&self, space: &str) -> Result<SpaceWithHomepage, ConfluenceError> {
        let mut url = self.api_url("/space/")?;
        url.path_segments_mut()
            .map_err(|_| ConfluenceError::ConfigError("invalid base URL".into()))?
            .pop_if_empty()
            .push(space);
        let response = self
            .client
            .get(url)
            .query(&[("expand", "homepage")])
            .send()
            .await?;
        handle_response(response, ConfluenceError::Unauthorized).await
    }

    /// Retrieve a single page by numeric ID.
    pub async fn get_page(&self, id: &str) -> Result<PageResponse, ConfluenceError> {
        let url = self.api_url(&format!("/content/{}", id))?;
        let response = self
            .client
            .get(url)
            .query(&[(
                "expand",
                "space,version,body.storage,metadata.labels,ancestors,_links",
            )])
            .send()
            .await?;

        // Confluence can hide restricted pages with 404 when an invalid token
        // causes the request to be treated as anonymous.
        if response.status() == StatusCode::NOT_FOUND {
            let current = self
                .client
                .get(self.api_url("/user/current")?)
                .send()
                .await?;
            let user: serde_json::Value =
                handle_response(current, ConfluenceError::Unauthorized).await?;
            if user.get("type").and_then(|value| value.as_str()) == Some("anonymous") {
                return Err(ConfluenceError::Unauthorized);
            }
        }
        handle_response(response, ConfluenceError::Unauthorized).await
    }
}

pub(crate) async fn handle_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    unauthorized: ConfluenceError,
) -> Result<T, ConfluenceError> {
    match response.status() {
        s if s.is_success() => Ok(response.json::<T>().await?),
        StatusCode::UNAUTHORIZED => Err(unauthorized),
        StatusCode::FORBIDDEN => Err(ConfluenceError::Forbidden),
        StatusCode::NOT_FOUND => Err(ConfluenceError::NotFound(
            "resource or endpoint not found".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn page_error(responses: Vec<(u16, &'static str)>) -> ConfluenceError {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (index, (status, body)) in responses.into_iter().enumerate() {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let byte = stream.read_u8().await.unwrap();
                    request.push(byte);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap();
                let path = if index == 0 {
                    "/rest/api/content/123?"
                } else {
                    "/rest/api/user/current "
                };
                assert!(request.starts_with(&format!("GET {path}")));
                assert!(request
                    .to_lowercase()
                    .contains("authorization: bearer test-token"));
                let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client =
            ConfluenceClient::new(&format!("http://{address}"), "/rest/api", "test-token").unwrap();
        let error = tokio::time::timeout(std::time::Duration::from_secs(5), client.get_page("123"))
            .await
            .unwrap()
            .unwrap_err();
        server.await.unwrap();
        error
    }

    #[tokio::test]
    async fn missing_page_with_anonymous_token_is_unauthorized() {
        let error = page_error(vec![(404, "{}"), (200, r#"{"type":"anonymous"}"#)]).await;
        assert!(matches!(error, ConfluenceError::Unauthorized));
        assert_eq!(error.kind(), "unauthorized");
        assert!(error.to_string().contains("expired"));
    }

    #[tokio::test]
    async fn missing_page_with_rejected_token_is_unauthorized() {
        assert!(matches!(
            page_error(vec![(404, "{}"), (401, "{}")]).await,
            ConfluenceError::Unauthorized
        ));
    }

    #[tokio::test]
    async fn missing_page_with_authenticated_user_stays_not_found() {
        assert!(matches!(
            page_error(vec![(404, "{}"), (200, r#"{"type":"known"}"#)]).await,
            ConfluenceError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn direct_authentication_errors_are_preserved() {
        assert!(matches!(
            page_error(vec![(401, "{}")]).await,
            ConfluenceError::Unauthorized
        ));
        assert!(matches!(
            page_error(vec![(403, "{}")]).await,
            ConfluenceError::Forbidden
        ));
    }

    #[tokio::test]
    async fn authentication_probe_failure_is_not_reported_as_missing_page() {
        assert!(matches!(
            page_error(vec![(404, "{}"), (500, "{}")]).await,
            ConfluenceError::HttpError { status: 500, .. }
        ));
    }
}
