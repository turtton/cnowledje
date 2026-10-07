//! Page-tree root resolution and paginated navigation output.
use std::collections::HashMap;

use crate::{
    client::ConfluenceClient,
    cql::build_exact_title_cql,
    error::ConfluenceError,
    format::make_page_url,
    markdown::{extract_page_tree_refs, PageTreeRef},
    models::{
        ChildPageOutput, ChildrenOutput, PageListResponse, PageReference, PageResponse, NOTICE,
    },
};

// Reference values are data, not Markdown/HTML supplied by the remote page.
fn display_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('`', "\\`")
}

pub async fn resolve_page_trees(
    client: &ConfluenceClient,
    page: &PageResponse,
    html: &str,
) -> HashMap<PageTreeRef, String> {
    let mut resolved = HashMap::new();
    for reference in extract_page_tree_refs(html) {
        if resolved.contains_key(&reference) {
            continue;
        }
        let space = reference.space_key.as_deref().unwrap_or(&page.space.key);
        let description = if reference.root == "@none" {
            format!(
                "[ページツリー: スペース「{}」全体（スペースキー: {}）]",
                display_text(space),
                display_text(space)
            )
        } else {
            match resolve_root(client, page, &reference).await {
                Ok(Some(root)) => format!(
                    "[ページツリー: 「{}」の配下（ルートページID: {}）]",
                    display_text(&root.title),
                    display_text(&root.id)
                ),
                Ok(None) => format!(
                    "[ページツリー: ルート未解決（指定: {}, スペースキー: {}）]",
                    display_text(&reference.root),
                    display_text(space)
                ),
                Err(_) => format!(
                    "[ページツリー: ルート未解決・参照先取得失敗（指定: {}, スペースキー: {}）]",
                    display_text(&reference.root),
                    display_text(space)
                ),
            }
        };
        resolved.insert(reference, description);
    }
    resolved
}

async fn resolve_root(
    client: &ConfluenceClient,
    page: &PageResponse,
    reference: &PageTreeRef,
) -> Result<Option<PageReference>, ConfluenceError> {
    let space = reference.space_key.as_deref().unwrap_or(&page.space.key);
    match reference.root.as_str() {
        "@self" => Ok(Some(PageReference {
            id: page.id.clone(),
            title: page.title.clone(),
        })),
        "@parent" => Ok(page.ancestors.last().cloned()),
        "@home" => Ok(client.get_homepage(space).await?.homepage),
        name if name.starts_with('@') => Ok(None),
        title => {
            let response = client
                .search(&build_exact_title_cql(space, title), 1)
                .await?;
            Ok(response.results.into_iter().next().map(|p| PageReference {
                id: p.id,
                title: p.title,
            }))
        }
    }
}

pub fn children_output(
    base_url: &str,
    parent_id: Option<String>,
    space_key: Option<String>,
    response: PageListResponse,
) -> ChildrenOutput {
    let next = response
        .links
        .as_ref()
        .and_then(|links| links.next.as_deref())
        .filter(|s| !s.is_empty());
    // Read only the offset; never follow an API-provided URL with credentials.
    let next_start = next
        .and_then(|next| {
            url::Url::parse("https://pagination.invalid/")
                .ok()?
                .join(next)
                .ok()
        })
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "start")
                .and_then(|(_, value)| value.parse::<u32>().ok())
        })
        .filter(|start| *start > response.start)
        .or_else(|| {
            next.and_then(|_| response.start.checked_add(response.size))
                .filter(|start| *start > response.start)
        });
    let results: Vec<_> = response
        .results
        .into_iter()
        .map(|page| {
            let has_children = page
                .children
                .and_then(|c| c.page)
                .map(|c| c.size > 0 || c.links.and_then(|l| l.next).is_some_and(|n| !n.is_empty()));
            let fallback = format!("/pages/viewpage.action?pageId={}", page.id);
            ChildPageOutput {
                id: page.id,
                title: page.title,
                url: make_page_url(
                    base_url,
                    response.links.as_ref().and_then(|l| l.base.as_deref()),
                    Some(page.links.webui.as_deref().unwrap_or(&fallback)),
                ),
                has_children,
            }
        })
        .collect();
    ChildrenOutput {
        parent_id,
        space_key,
        start: response.start,
        returned: results.len(),
        has_more: next.is_some(),
        next_start,
        results,
        notice: NOTICE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn page() -> PageResponse {
        serde_json::from_value(serde_json::json!({
            "id":"123", "title":"開発ガイド", "space":{"key":"DEV", "name":"Development"},
            "version":{"when":null}, "_links":{},
            "ancestors":[{"id":"1", "title":"Home"},{"id":"12", "title":"Parent"}]
        }))
        .unwrap()
    }

    async fn server(
        responses: Vec<(String, u16, String)>,
    ) -> (ConfluenceClient, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            for (expected, status, body) in responses {
                let (mut stream, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(&format!("GET {expected}")), "{request}");
                assert!(request
                    .to_lowercase()
                    .contains("authorization: bearer test-token"));
                let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (
            ConfluenceClient::new(
                &format!("http://{address}/confluence"),
                "/rest/api",
                "test-token",
            )
            .unwrap(),
            task,
        )
    }

    #[tokio::test]
    async fn local_roots_do_not_request_child_pages() {
        let client =
            ConfluenceClient::new("http://127.0.0.1:1", "/rest/api", "test-token").unwrap();
        let html = r#"<ac:structured-macro ac:name="pagetree"><ac:parameter ac:name="root">@self</ac:parameter></ac:structured-macro><ac:structured-macro ac:name="pagetree"><ac:parameter ac:name="root">@parent</ac:parameter></ac:structured-macro><ac:structured-macro ac:name="pagetree"><ac:parameter ac:name="root">@none</ac:parameter><ac:parameter ac:name="spaceKey">OPS</ac:parameter></ac:structured-macro>"#;
        let descriptions = resolve_page_trees(&client, &page(), html).await;
        let md = crate::markdown::html_to_markdown_with_references(html, None, &[], &descriptions);
        assert!(md.contains("「開発ガイド」の配下（ルートページID: 123）"));
        assert!(md.contains("「Parent」の配下（ルートページID: 12）"));
        assert!(md.contains("スペース「OPS」全体"));
        assert!(!md.contains("cnowledje"));
        assert!(!md.contains("@self"));
    }

    #[tokio::test]
    async fn resolves_home_and_named_cross_space_root_once() {
        let (client, task) = server(vec![
            ("/confluence/rest/api/space/DEV?expand=homepage".into(), 200, r#"{"homepage":{"id":"1","title":"Home"}}"#.into()),
            ("/confluence/rest/api/content/search?cql=space+%3D+%22OPS%22+AND+type+%3D+page+AND+title+%3D+%22Guide%22".into(), 200,
             r#"{"results":[{"id":"9","title":"Guide","space":{"key":"OPS","name":"Ops"},"version":{},"_links":{}}],"size":1}"#.into()),
        ]).await;
        let html = r#"<ac:structured-macro ac:name="pagetree"/><ac:structured-macro ac:name="pagetree"/><ac:structured-macro ac:name="pagetree"><ac:parameter ac:name="root"><ac:link><ri:page ri:space-key="OPS" ri:content-title="Guide"/></ac:link></ac:parameter></ac:structured-macro>"#;
        let descriptions = resolve_page_trees(&client, &page(), html).await;
        assert_eq!(descriptions.len(), 2);
        assert!(descriptions
            .values()
            .any(|s| s.contains("ルートページID: 1")));
        assert!(descriptions
            .values()
            .any(|s| s.contains("ルートページID: 9")));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn failed_resolution_keeps_body_and_explicit_unresolved_reference() {
        let (client, task) = server(vec![(
            "/confluence/rest/api/space/DEV?".into(),
            403,
            "{}".into(),
        )])
        .await;
        let html = r#"<p>本文</p><ac:structured-macro ac:name="pagetree"/>"#;
        let descriptions = resolve_page_trees(&client, &page(), html).await;
        let md = crate::markdown::html_to_markdown_with_references(html, None, &[], &descriptions);
        assert!(md.contains("本文"));
        assert!(md.contains("ルート未解決・参照先取得失敗"));
        assert!(md.contains("スペースキー: DEV"));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn list_requests_and_pagination_preserve_context_path() {
        let (client, task) = server(vec![
            ("/confluence/rest/api/content/123/child/page?start=5&limit=2&expand=children.page%2C_links".into(), 200,
             r#"{"results":[{"id":"7","title":"Child","_links":{"webui":"/pages/7"},"children":{"page":{"size":1}}},{"id":"8","title":"Leaf","children":{"page":{"size":0}}}],"start":5,"size":2,"_links":{"next":"/rest/api/content/123/child/page?start=7"}}"#.into()),
            ("/confluence/rest/api/content?spaceKey=OPS&type=page&status=current&start=0&limit=2".into(), 200, r#"{"results":[],"start":0,"size":0}"#.into()),
        ]).await;
        let response = client.get_children("123", 5, 2).await.unwrap();
        let output = children_output(
            "https://example.com/confluence",
            Some("123".into()),
            None,
            response,
        );
        assert_eq!(output.next_start, Some(7));
        assert!(output.has_more);
        assert_eq!(output.results[0].has_children, Some(true));
        assert_eq!(output.results[1].has_children, Some(false));
        assert_eq!(
            output.results[0].url,
            "https://example.com/confluence/pages/7"
        );
        assert!(output.results[1].url.ends_with("pageId=8"));
        let empty = children_output(
            "https://example.com",
            None,
            Some("OPS".into()),
            client.get_space_pages("OPS", 0, 2).await.unwrap(),
        );
        assert_eq!(empty.returned, 0);
        assert!(!empty.has_more);
        assert_eq!(empty.next_start, None);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn skipped_translation_and_literal_code_do_not_shift_roots() {
        let client =
            ConfluenceClient::new("http://127.0.0.1:1", "/rest/api", "test-token").unwrap();
        let html = r#"<ac:structured-macro ac:name="sv-translation"><ac:parameter ac:name="language">en</ac:parameter><ac:rich-text-body><ac:structured-macro ac:name="pagetree"><ac:parameter ac:name="root">@parent</ac:parameter></ac:structured-macro></ac:rich-text-body></ac:structured-macro><ac:structured-macro ac:name="sv-translation"><ac:parameter ac:name="language">ja</ac:parameter><ac:rich-text-body><ac:structured-macro ac:name="pagetree"><ac:parameter ac:name="root">@self</ac:parameter></ac:structured-macro></ac:rich-text-body></ac:structured-macro><ac:structured-macro ac:name="code"><ac:plain-text-body><![CDATA[<ac:structured-macro ac:name="pagetree"/>]]></ac:plain-text-body></ac:structured-macro>"#;
        let descriptions = resolve_page_trees(&client, &page(), html).await;
        assert_eq!(descriptions.len(), 2);
        let md =
            crate::markdown::html_to_markdown_with_references(html, Some("ja"), &[], &descriptions);
        assert!(md.contains("ルートページID: 123"));
        assert!(!md.contains("「Parent」"));
        assert!(md.contains(r#"<ac:structured-macro ac:name="pagetree"/>"#));
    }

    #[tokio::test]
    async fn missing_parent_and_search_miss_stay_unresolved() {
        let (client, task) = server(vec![(
            "/confluence/rest/api/content/search?".into(),
            200,
            r#"{"results":[],"size":0}"#.into(),
        )])
        .await;
        let mut page = page();
        page.ancestors.clear();
        let html = r#"<ac:structured-macro ac:name="pagetree"><ac:parameter ac:name="root">@parent</ac:parameter></ac:structured-macro><ac:structured-macro ac:name="pagetree"><ac:parameter ac:name="root">Missing</ac:parameter></ac:structured-macro>"#;
        let descriptions = resolve_page_trees(&client, &page, html).await;
        assert_eq!(descriptions.len(), 2);
        assert!(descriptions
            .values()
            .all(|value| value.contains("ルート未解決") && !value.contains("ルートページID")));
        task.await.unwrap();
    }

    #[test]
    fn missing_child_metadata_is_unknown_and_next_offset_uses_server_value() {
        let response = serde_json::from_str(r#"{"results":[{"id":"7","title":"Child"}],"start":0,"size":1,"_links":{"next":"https://other.invalid/path?start=25"}}"#).unwrap();
        let output = children_output("https://example.com", Some("1".into()), None, response);
        assert_eq!(output.next_start, Some(25));
        assert_eq!(output.results[0].has_children, None);
        let json = serde_json::to_value(output).unwrap();
        assert!(json["space_key"].is_null());
        assert!(json["results"][0]["has_children"].is_null());
    }
}
