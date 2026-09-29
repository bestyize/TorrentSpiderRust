//! `/api/sources` 与 `/api/health`。

use crate::support::*;

#[tokio::test]
async fn sources_match_the_library_catalog() {
    let expected = supported_sources();
    let reply = Harness::search(FakeSearch::default(), expected.clone()).get("/api/sources").await;
    assert_eq!(reply.status, StatusCode::OK);
    let items = reply.json()["sources"].as_array().unwrap().clone();
    assert_eq!(items.len(), expected.len());
    for (item, want) in items.iter().zip(&expected) {
        assert_eq!(item["src"], want.src);
        assert_eq!(item["title"], want.title.as_str());
        assert_eq!(item["official_url"], want.official_url.as_deref().unwrap());
        assert_eq!(item["level"], want.level);
    }
    let codes: Vec<u64> = items.iter().map(|i| i["src"].as_u64().unwrap()).collect();
    assert_eq!(codes, (0..=10).collect::<Vec<_>>(), "src 编号是冻结的 0 到 10");
    let yuhuage = items.iter().find(|i| i["src"] == 1).unwrap();
    assert_eq!(yuhuage["level"], 1);
}

#[tokio::test]
async fn health_is_ok() {
    let reply = Harness::search(FakeSearch::default(), sources_subset(1)).get("/api/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["status"], "ok");
    assert!(reply.content_type().starts_with("application/json"));
    assert_eq!(reply.headers.get("x-content-type-options").unwrap(), "nosniff");
}
