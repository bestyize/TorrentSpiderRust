//! 静态页与兜底路由。

use crate::support::*;

#[tokio::test]
async fn root_redirects_to_index() {
    let reply = Harness::search(FakeSearch::default(), sources_subset(1)).get("/").await;
    assert_eq!(reply.status, StatusCode::FOUND);
    assert_eq!(reply.headers.get(header::LOCATION).unwrap(), "/index.html");
}

#[tokio::test]
async fn index_references_assets_and_compliance_notice() {
    let reply = Harness::search(FakeSearch::default(), sources_subset(1)).get("/index.html").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.content_type().starts_with("text/html"));
    for needle in ["<title", "/styles.css", "/app.js", "仅用于搜索合法、可分发内容"] {
        assert!(reply.body.contains(needle), "index.html 缺少 {needle}");
    }
}

#[tokio::test]
async fn assets_have_correct_mime_types() {
    let harness = Harness::search(FakeSearch::default(), sources_subset(1));
    let css = harness.get("/styles.css").await;
    assert_eq!(css.status, StatusCode::OK);
    assert!(css.content_type().starts_with("text/css"));
    assert!(css.body.contains("--radius-card"));
    let js = harness.get("/app.js").await;
    assert_eq!(js.status, StatusCode::OK);
    assert!(js.content_type().contains("javascript"), "{}", js.content_type());
}

#[tokio::test]
async fn unknown_paths_are_404_not_the_index_page() {
    let harness = Harness::search(FakeSearch::default(), sources_subset(1));
    for uri in ["/nope.txt", "/api/nope"] {
        let reply = harness.get(uri).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(reply.code(), "NOT_FOUND");
    }
}

#[tokio::test]
async fn large_responses_are_gzipped_when_requested() {
    let harness = Harness::search(FakeSearch::default(), sources_subset(1));
    let request =
        Request::builder().uri("/app.js").header(header::ACCEPT_ENCODING, "gzip").body(Body::empty()).unwrap();
    let response = router(harness.state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.headers().get(header::CONTENT_ENCODING).unwrap(), "gzip");
}
