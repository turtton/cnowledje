use reqwest::{
    header::{self, HeaderMap, HeaderValue},
    Client, StatusCode,
};
use url::Url;

use crate::error::ConfluenceError;
use crate::models::{
    AttachmentListResponse, PageListResponse, PageResponse, SearchResponse, SpaceWithHomepage,
};

/// Build a `reqwest::Client` with the Bearer auth header set for `token`.
///
/// The token value is marked sensitive so it will not appear in debug
/// output from `reqwest`.
pub(crate) fn build_http_client(token: &str) -> Result<Client, ConfluenceError> {
    build_http_client_with_redirect(token, reqwest::redirect::Policy::default())
}

fn build_http_client_with_redirect(
    token: &str,
    redirect: reqwest::redirect::Policy,
) -> Result<Client, ConfluenceError> {
    let mut auth_value = HeaderValue::from_str(&format!("Bearer {}", token)).map_err(|_| {
        ConfluenceError::ConfigError("invalid token: contains non-ASCII bytes".to_string())
    })?;
    auth_value.set_sensitive(true);

    let mut default_headers = HeaderMap::new();
    default_headers.insert(header::AUTHORIZATION, auth_value);
    default_headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));

    Client::builder()
        .default_headers(default_headers)
        .redirect(redirect)
        .build()
        .map_err(ConfluenceError::RequestError)
}

pub struct ConfluenceClient {
    client: Client,
    attachment_client: Client,
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
        let origin = base_url.origin();
        let attachment_client = build_http_client_with_redirect(
            token,
            reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 10 {
                    attempt.error("too many attachment redirects")
                } else if attempt.url().origin() != origin
                    || !attempt.url().username().is_empty()
                    || attempt.url().password().is_some()
                {
                    attempt.error("attachment redirect points outside configured server")
                } else {
                    attempt.follow()
                }
            }),
        )?;

        Ok(Self {
            client,
            attachment_client,
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

    /// Read a page's source attachment by its exact name (no extension guessing).
    /// Resolve its download link through the attachment API, preserving context
    /// paths and query parameters. Links and redirects must stay on this server.
    pub async fn get_attachment_text(
        &self,
        page_id: &str,
        filename: &str,
    ) -> Result<String, ConfluenceError> {
        let page_id = crate::cql::extract_page_id(page_id)?;
        if filename.trim().is_empty() || matches!(filename, "." | "..") {
            return Err(ConfluenceError::InvalidArguments(
                "invalid attachment filename".into(),
            ));
        }
        let response = self
            .client
            .get(self.api_url(&format!("/content/{page_id}/child/attachment"))?)
            .query(&[("filename", filename), ("limit", "2")])
            .send()
            .await?;
        let attachments: AttachmentListResponse =
            handle_response(response, ConfluenceError::Unauthorized).await?;
        let attachment = attachments
            .results
            .iter()
            .find(|attachment| attachment.title == filename)
            .ok_or_else(|| {
                ConfluenceError::NotFound("attachment not found in page metadata".into())
            })?;
        let download = attachment
            .links
            .download
            .as_deref()
            .filter(|link| !link.trim().is_empty())
            .ok_or_else(|| {
                ConfluenceError::InvalidArguments("attachment metadata has no download link".into())
            })?;
        let url = self.attachment_download_url(attachments.links.base.as_deref(), download)?;
        let mut response = self
            .attachment_client
            .get(url)
            .header(header::ACCEPT, "text/plain")
            .send()
            .await?;
        if !response.status().is_success() {
            handle_response::<serde_json::Value>(response, ConfluenceError::Unauthorized).await?;
            unreachable!("non-success responses always return an error");
        }
        // A login page or rendered diagram is not Mermaid source.
        if let Some(content_type) = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
        {
            let content_type = content_type
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if content_type == "text/html"
                || content_type == "application/xhtml+xml"
                || content_type.starts_with("image/")
            {
                return Err(ConfluenceError::InvalidArguments(
                    "attachment is not plain text source".into(),
                ));
            }
        }
        const MAX_SOURCE_BYTES: usize = 1024 * 1024;
        if response
            .content_length()
            .is_some_and(|len| len > MAX_SOURCE_BYTES as u64)
        {
            return Err(ConfluenceError::InvalidArguments(
                "attachment source exceeds 1 MiB".into(),
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if chunk.len() > MAX_SOURCE_BYTES - bytes.len() {
                return Err(ConfluenceError::InvalidArguments(
                    "attachment source exceeds 1 MiB".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes)
            .map_err(|_| ConfluenceError::InvalidArguments("attachment source is not UTF-8".into()))
    }

    fn attachment_download_url(
        &self,
        api_base: Option<&str>,
        download: &str,
    ) -> Result<Url, ConfluenceError> {
        if let Ok(absolute) = Url::parse(download) {
            self.validate_attachment_url(&absolute)?;
            return Ok(absolute);
        }
        if download.starts_with("//") {
            let absolute = self.base_url.join(download)?;
            self.validate_attachment_url(&absolute)?;
            return Ok(absolute);
        }
        let mut base = match api_base {
            Some(base) => self.base_url.join(base)?,
            None => self.base_url.clone(),
        };
        self.validate_attachment_url(&base)?;
        base.set_query(None);
        base.set_fragment(None);
        // Confluence's /download/... links are relative to _links.base, which
        // includes its context path. Already-prefixed and absolute URLs also occur.
        let context = base.path().trim_end_matches('/').to_owned();
        let url = if !context.is_empty() && download.starts_with(&format!("{context}/")) {
            base.join(download)?
        } else {
            base.set_path(&format!("{context}/"));
            base.join(download.trim_start_matches('/'))?
        };
        self.validate_attachment_url(&url)?;
        Ok(url)
    }

    fn validate_attachment_url(&self, url: &Url) -> Result<(), ConfluenceError> {
        if url.origin() != self.base_url.origin()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(ConfluenceError::InvalidArguments(
                "attachment URL points outside configured server".into(),
            ));
        }
        Ok(())
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

    async fn attachment_result(
        status: u16,
        content_type: &str,
        body: &[u8],
    ) -> Result<String, ConfluenceError> {
        attachment_result_with_redirect(status, content_type, body, None).await
    }

    async fn attachment_result_with_redirect(
        status: u16,
        content_type: &str,
        body: &[u8],
        redirect: Option<&str>,
    ) -> Result<String, ConfluenceError> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let body = body.to_vec();
        let redirect = redirect.map(str::to_owned);
        let server = tokio::spawn(async move {
            let metadata = serde_json::json!({
                "results": [{"title": "diagram & draft?.mmd", "_links": {"download": "/custom-sources/source%20file?version=11&format=raw"}}],
                "_links": {"base": format!("http://{address}/confluence/team")}
            }).to_string();
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            let request = String::from_utf8(request).unwrap();
            let target = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = Url::parse(&format!("http://{address}{target}")).unwrap();
            assert_eq!(
                url.path(),
                "/confluence/team/rest/api/content/123/child/attachment"
            );
            assert_eq!(
                url.query_pairs()
                    .find(|(key, _)| key == "filename")
                    .unwrap()
                    .1,
                "diagram & draft?.mmd"
            );
            assert!(request
                .to_lowercase()
                .contains("authorization: bearer test-token"));
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{metadata}", metadata.len()).as_bytes()).await.unwrap();
            drop(stream);
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET /confluence/team/custom-sources/source%20file?version=11&format=raw HTTP/1.1\r\n"), "{request}");
            assert!(request
                .to_lowercase()
                .contains("authorization: bearer test-token"));
            if let Some(location) = redirect {
                stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                drop(stream);
                if !location.starts_with('/') {
                    return;
                }
                (stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(&format!("GET {location} HTTP/1.1\r\n")));
                assert!(request
                    .to_lowercase()
                    .contains("authorization: bearer test-token"));
            }
            stream.write_all(response.as_bytes()).await.unwrap();
            // An oversized download can be rejected before reading its body.
            let _ = stream.write_all(&body).await;
        });
        let client = ConfluenceClient::new(
            &format!("http://{address}"),
            "/confluence/team/rest/api",
            "test-token",
        )
        .unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.get_attachment_text("123", "diagram & draft?.mmd"),
        )
        .await
        .unwrap();
        server.await.unwrap();
        result
    }

    #[tokio::test]
    async fn attachment_text_uses_metadata_paths_query_and_utf8() {
        let source = "graph TD\n  A[開始] --> B[終了]\n";
        assert_eq!(
            attachment_result(200, "text/plain; charset=utf-8", source.as_bytes())
                .await
                .unwrap(),
            source
        );
    }

    #[tokio::test]
    async fn attachment_redirects_stay_on_configured_server() {
        assert_eq!(
            attachment_result_with_redirect(
                200,
                "text/plain",
                b"graph TD\nA-->B",
                Some("/confluence/team/redirected?version=11")
            )
            .await
            .unwrap(),
            "graph TD\nA-->B"
        );
        let error = attachment_result_with_redirect(
            200,
            "text/plain",
            b"",
            Some("http://other.local/source"),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, ConfluenceError::RequestError(error) if error.is_redirect()));
    }

    #[tokio::test]
    async fn attachment_errors_and_non_source_responses_are_rejected() {
        for (status, kind) in [
            (401, "unauthorized"),
            (403, "forbidden"),
            (404, "not_found"),
        ] {
            assert_eq!(
                attachment_result(status, "application/json", b"{}")
                    .await
                    .unwrap_err()
                    .kind(),
                kind
            );
        }
        for content_type in ["text/html", "image/svg+xml", "application/xhtml+xml"] {
            assert!(attachment_result(200, content_type, b"<html>Login</html>")
                .await
                .is_err());
        }
        assert!(attachment_result(200, "text/plain", &[0xff]).await.is_err());
        let error = attachment_result(200, "text/plain", &vec![b'x'; 1024 * 1024 + 1])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("exceeds 1 MiB"));
    }

    #[tokio::test]
    async fn attachment_invalid_identity_is_rejected_before_request() {
        let client =
            ConfluenceClient::new("http://127.0.0.1:1", "/rest/api", "test-token").unwrap();
        for filename in ["", " ", ".", ".."] {
            assert!(matches!(
                client.get_attachment_text("123", filename).await,
                Err(ConfluenceError::InvalidArguments(_))
            ));
        }
        assert!(matches!(
            client.get_attachment_text("invalid", "diagram").await,
            Err(ConfluenceError::InvalidPageUrl(_))
        ));
    }

    #[test]
    fn attachment_download_urls_preserve_context_and_absolute_links() {
        let client = ConfluenceClient::new(
            "https://domain.local/confluence/team",
            "/rest/api",
            "test-token",
        )
        .unwrap();
        for link in [
            "/custom/source%20file?version=11",
            "custom/source%20file?version=11",
            "/confluence/team/custom/source%20file?version=11",
            "https://domain.local/confluence/team/custom/source%20file?version=11",
            "//domain.local/confluence/team/custom/source%20file?version=11",
        ] {
            assert_eq!(
                client.attachment_download_url(None, link).unwrap().as_str(),
                "https://domain.local/confluence/team/custom/source%20file?version=11"
            );
        }
        assert_eq!(
            client
                .attachment_download_url(Some("https://domain.local/proxy/wiki"), "/assets/source")
                .unwrap()
                .as_str(),
            "https://domain.local/proxy/wiki/assets/source"
        );
        let root =
            ConfluenceClient::new("https://domain.local", "/rest/api", "test-token").unwrap();
        assert_eq!(
            root.attachment_download_url(None, "/assets/source")
                .unwrap()
                .as_str(),
            "https://domain.local/assets/source"
        );
    }

    #[test]
    fn attachment_download_urls_reject_foreign_origins_and_credentials() {
        let client = ConfluenceClient::new(
            "https://domain.local/confluence/team",
            "/rest/api",
            "test-token",
        )
        .unwrap();
        for link in [
            "https://other.local/source",
            "//other.local/source",
            "http://domain.local/source",
            "https://domain.local:8443/source",
            "https://user:password@domain.local/source",
            "file:///source",
        ] {
            assert!(
                matches!(
                    client.attachment_download_url(None, link),
                    Err(ConfluenceError::InvalidArguments(_))
                ),
                "{link}"
            );
        }
        assert!(client
            .attachment_download_url(Some("https://other.local/wiki"), "/source")
            .is_err());
    }

    async fn attachment_metadata_error(metadata: serde_json::Value) -> ConfluenceError {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            let body = metadata.to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let client =
            ConfluenceClient::new(&format!("http://{address}"), "/rest/api", "test-token").unwrap();
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.get_attachment_text("123", "diagram"),
        )
        .await
        .unwrap()
        .unwrap_err();
        server.await.unwrap();
        error
    }

    #[tokio::test]
    async fn attachment_metadata_misses_and_missing_links_do_not_guess_paths() {
        for results in [
            serde_json::json!([]),
            serde_json::json!([{"title":"other-diagram", "_links":{"download":"/source"}}]),
        ] {
            let error = attachment_metadata_error(serde_json::json!({"results":results})).await;
            assert!(matches!(error, ConfluenceError::NotFound(_)));
            assert!(error.to_string().contains("page metadata"));
        }
        let error =
            attachment_metadata_error(serde_json::json!({"results":[{"title":"diagram"}]})).await;
        assert!(error.to_string().contains("no download link"));
        let error = attachment_metadata_error(serde_json::json!({"results":[{"title":"diagram", "_links":{"download":"http://other.local/source"}}]})).await;
        assert!(error.to_string().contains("outside configured server"));
    }

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
