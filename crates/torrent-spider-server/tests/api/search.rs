//! `/api/search` 的参数校验、信封形状与多源聚合。

use crate::support::*;

#[tokio::test]
async fn search_returns_contract_envelope_with_snake_case_fields() {
    let search = FakeSearch::default().on(0, |_, _| {
        Ok(vec![TorrentInfo {
            src: 0,
            title: Some("Ubuntu 24.04".into()),
            hash: Some("E90710273A1E578835CAC80D3E4B3E56A2755663".into()),
            magnet_url: Some("magnet:?xt=urn:btih:E90710273A1E578835CAC80D3E4B3E56A2755663".into()),
            size: Some("5.0G".into()),
            date: Some("2026-08-01".into()),
            download_count: Some("128".into()),
            ..Default::default()
        }])
    });
    let reply = Harness::search(search, sources_subset(3)).get("/api/search?q=ubuntu&page=1&src=0").await;

    assert_eq!(reply.status, StatusCode::OK);
    let root = reply.json();
    assert_eq!(root["keyword"], "ubuntu");
    assert_eq!(root["page"], 1);
    assert_eq!(root["src"], 0);
    let items = root["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item["title"], "Ubuntu 24.04");
    assert_eq!(item["size"], "5.0G");
    assert_eq!(item["date"], "2026-08-01");
    assert_eq!(item["download_count"], "128");
    for key in ["detail_url", "magnet_url", "torrent_url", "last_download_date", "leacher_count", "sender_count"] {
        assert!(item.get(key).is_some(), "missing key {key}");
    }
    assert!(reply.content_type().starts_with("application/json"));
}

#[tokio::test]
async fn out_of_range_src_is_400_not_a_silent_fallback() {
    let harness = Harness::search(FakeSearch::default().on(4, |_, _| Ok(vec![item(4, "x")])), sources_subset(11));
    let reply = harness.get("/api/search?q=x&src=999").await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.code(), "INVALID_SRC");
    assert!(harness.search.calls().is_empty(), "越界的 src 不该打到上游");
}

#[tokio::test]
async fn non_numeric_src_is_400() {
    let harness = Harness::search(FakeSearch::default(), sources_subset(3));
    let reply = harness.get("/api/search?q=x&src=abc").await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(harness.search.calls().is_empty());
}

#[tokio::test]
async fn src_outside_the_configured_sources_is_400() {
    // 编号合法但不在本服务的源清单里，也不该打到上游
    let harness = Harness::search(FakeSearch::default(), sources_subset(3));
    assert_eq!(harness.get("/api/search?q=x&src=5").await.status, StatusCode::BAD_REQUEST);
    assert!(harness.search.calls().is_empty());
}

#[tokio::test]
async fn missing_or_blank_keyword_is_400() {
    let harness = Harness::search(FakeSearch::default(), sources_subset(3));
    for uri in ["/api/search?page=1", "/api/search?q=%20%20"] {
        let reply = harness.get(uri).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(reply.code(), "MISSING_KEYWORD");
    }
}

#[tokio::test]
async fn invalid_page_is_400() {
    let harness = Harness::search(FakeSearch::default(), sources_subset(3));
    for page in ["0", "101", "abc", "99999999999999999999"] {
        let reply = harness.get(&format!("/api/search?q=x&page={page}")).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "page={page}");
        assert_eq!(reply.code(), "INVALID_PAGE");
    }
    assert!(harness.search.calls().is_empty());
}

#[tokio::test]
async fn all_sources_ok_but_empty_is_200_not_502() {
    let reply = Harness::search(FakeSearch::default(), sources_subset(3)).get("/api/search?q=nothingmatchesthis").await;
    assert_eq!(reply.status, StatusCode::OK);
    let root = reply.json();
    assert!(root["items"].as_array().unwrap().is_empty());
    let statuses = root["sources"].as_array().unwrap();
    assert_eq!(statuses.iter().filter(|s| s["ok"] == true).count(), 3);
}

#[tokio::test]
async fn one_failing_source_does_not_break_aggregate_search() {
    let sources = sources_subset(3);
    let failing = sources[1].src;
    let search = FakeSearch::default()
        .on(sources[0].src, |_, _| Ok(vec![item(0, "好的结果")]))
        .on(failing, |_, _| Err(http_failure(403)))
        .on(sources[2].src, |_, _| Ok(vec![item(2, "另一个结果")]));
    let reply = Harness::search(search, sources).get("/api/search?q=movie&page=1&src=all&filter=0").await;

    assert_eq!(reply.status, StatusCode::OK);
    let root = reply.json();
    assert_eq!(root["items"].as_array().unwrap().len(), 2);
    let statuses = root["sources"].as_array().unwrap();
    assert_eq!(statuses.len(), 3);
    let failed: Vec<_> = statuses.iter().filter(|s| s["ok"] == false).collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["src"], failing);
    assert_eq!(failed[0]["count"], 0);
    // 失败源必须带上状态码，否则界面只能显示「结果偏少」
    assert!(failed[0]["error"].as_str().unwrap().contains("403"));
}

#[tokio::test]
async fn all_sources_failing_is_502_with_error_envelope() {
    let sources = sources_subset(2);
    let search = sources.iter().fold(FakeSearch::default(), |fake, s| fake.on(s.src, |_, _| Err(http_failure(403))));
    let reply = Harness::search(search, sources).get("/api/search?q=movie&src=all").await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    let root = reply.json();
    assert_eq!(root["code"], "ALL_SOURCES_FAILED");
    assert!(!root["message"].as_str().unwrap().is_empty());
    assert!(root.get("src").is_some() && root.get("detail_url").is_some(), "错误信封字段齐全");
}

#[tokio::test]
async fn single_source_failure_is_502() {
    let sources = sources_subset(1);
    let src = sources[0].src;
    let reply = Harness::search(FakeSearch::default().on(src, |_, _| Err(http_failure(503))), sources)
        .get(&format!("/api/search?q=x&src={src}"))
        .await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(reply.code(), "ALL_SOURCES_FAILED");
    assert!(reply.json()["message"].as_str().unwrap().contains("503"));
}

#[tokio::test]
async fn blank_fields_are_serialized_as_null() {
    // 解析器之外的数据也可能用 "" 表示「没有这个值」，前端只该判一次空
    let sources = sources_subset(1);
    let search = FakeSearch::default().on(0, |_, _| {
        Ok(vec![TorrentInfo {
            title: Some("有条目但缺字段".into()),
            hash: Some(String::new()),
            magnet_url: Some(String::new()),
            detail_url: Some(String::new()),
            size: Some(String::new()),
            date: Some("   ".into()),
            ..Default::default()
        }])
    });
    let reply = Harness::search(search, sources).get("/api/search?q=x&src=0&filter=0").await;
    assert_eq!(reply.status, StatusCode::OK);
    let item = &reply.json()["items"][0];
    for key in ["magnet_url", "hash", "detail_url", "size", "date"] {
        assert!(item.get(key).is_some_and(Value::is_null), "{key} should be null: {item}");
    }
    assert_eq!(item["title"], "有条目但缺字段");
}

#[tokio::test]
async fn keyword_and_page_are_passed_through() {
    let harness = Harness::search(FakeSearch::default(), sources_subset(1));
    harness.get("/api/search?q=%20ubuntu%2024%20&page=2&src=0").await;
    assert_eq!(harness.search.calls(), vec![(0, "ubuntu 24".to_owned(), 2)]);
}

#[tokio::test]
async fn aggregate_items_are_ranked_by_relevance() {
    // 站点多半按入库时间排；关键词埋在长标题尾部的条目应该沉下去
    let search = FakeSearch::default()
        .on(0, |_, _| Ok(vec![item(0, "Celtics City S01E08 Chapter VIII Ubuntu 720p WEB-DL.mkv")]))
        .on(1, |_, _| Ok(vec![item(1, "ubuntu-22.04-desktop-amd64.iso")]));
    let reply = Harness::search(search, sources_subset(2)).get("/api/search?q=ubuntu&src=all").await;
    let titles: Vec<_> = reply.json()["items"].as_array().unwrap().iter().map(|i| i["title"].clone()).collect();
    assert_eq!(titles, ["ubuntu-22.04-desktop-amd64.iso", "Celtics City S01E08 Chapter VIII Ubuntu 720p WEB-DL.mkv"]);
}

#[tokio::test]
async fn no_hits_keep_upstream_order_instead_of_title_order() {
    let search = FakeSearch::default().on(0, |_, _| Ok(vec![item(0, "b"), item(0, "A")]));
    let reply = Harness::search(search, sources_subset(1)).get("/api/search?q=x&src=0&filter=0").await;
    let titles: Vec<_> = reply.json()["items"].as_array().unwrap().iter().map(|i| i["title"].clone()).collect();
    assert_eq!(titles, ["b", "A"]);
}

#[tokio::test]
async fn missing_src_means_aggregate_and_src_is_null() {
    let search = FakeSearch::default().on(0, |_, _| Ok(vec![item(0, "a")])).on(1, |_, _| Ok(vec![item(1, "b")]));
    let reply = Harness::search(search, sources_subset(2)).get("/api/search?q=x&filter=0").await;
    assert_eq!(reply.status, StatusCode::OK);
    let root = reply.json();
    assert!(root.get("src").is_some_and(Value::is_null), "聚合时 src 键在、值为 null");
    assert_eq!(root["items"].as_array().unwrap().len(), 2);
    assert_eq!(root["sources"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn slow_source_times_out_without_blocking_others() {
    struct Slow;
    #[async_trait]
    impl SearchGateway for Slow {
        async fn search(&self, src: u32, _: &str, _: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
            if src == 1 {
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            Ok(vec![item(src, "fast")])
        }
    }
    let mut state = AppState::new(Arc::new(Slow), Arc::new(FakeMagnet::default()), sources_subset(2));
    state.search_timeout = Duration::from_millis(100);
    let request = Request::builder().uri("/api/search?q=fast").body(Body::empty()).unwrap();
    let response = router(state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["sources"][1]["ok"], false);
    assert_eq!(body["sources"][1]["error"], "请求超时");
}
