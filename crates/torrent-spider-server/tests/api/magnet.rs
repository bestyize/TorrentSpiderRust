//! `/api/magnet`：按详情页取磁力。

use crate::support::*;

#[tokio::test]
async fn magnet_fetch_returns_magnet_hash_and_derived_torrent_url() {
    let harness = Harness::magnet(FakeMagnet::returning(SAMPLE_MAGNET));
    let reply = harness.get(&format!("/api/magnet?src=7&detail_url={}", encode(DETAIL))).await;

    assert_eq!(reply.status, StatusCode::OK);
    let root = reply.json();
    assert_eq!(root["src"], 7);
    assert_eq!(root["detail_url"], DETAIL);
    assert_eq!(root["magnet_url"], SAMPLE_MAGNET);
    assert_eq!(root["hash"], SAMPLE_HASH);
    assert_eq!(root["resolved"], true);
    assert_eq!(root["torrent_url"], format!("https://itorrents.net/torrent/{SAMPLE_HASH}.torrent"));
    assert_eq!(harness.magnet.calls(), vec![(7, DETAIL.to_owned())], "detail_url 原样透传给上游");
}

#[tokio::test]
async fn missing_magnet_is_404_magnet_unavailable_not_502() {
    let reply =
        Harness::magnet(FakeMagnet::default()).get(&format!("/api/magnet?src=7&detail_url={}", encode(DETAIL))).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let root = reply.json();
    assert_eq!(root["code"], "MAGNET_UNAVAILABLE");
    // 错误信封要带上是哪个源、哪条详情页，用户才能判断该不该换源
    assert_eq!(root["src"], 7);
    assert_eq!(root["detail_url"], DETAIL);
}

#[tokio::test]
async fn non_magnet_answer_from_upstream_is_404() {
    // acg.rip 这类站点「磁力」其实是 .torrent 地址，不能当磁力回给前端
    let reply = Harness::magnet(FakeMagnet::returning("https://acg.rip/t/1.torrent"))
        .get(&format!("/api/magnet?src=3&detail_url={}", encode("https://acg.rip/t/1.torrent")))
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.code(), "MAGNET_UNAVAILABLE");
}

#[tokio::test]
async fn slow_magnet_lookup_times_out_as_404() {
    let magnet =
        FakeMagnet { answer: Some(SAMPLE_MAGNET.into()), delay: Some(Duration::from_secs(30)), ..Default::default() };
    let mut harness = Harness::magnet(magnet);
    harness.state.magnet_timeout = Duration::from_millis(50);
    let reply = harness.get(&format!("/api/magnet?src=7&detail_url={}", encode(DETAIL))).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn known_magnet_is_echoed_without_fetching() {
    let harness =
        Harness::magnet(FakeMagnet::returning("magnet:?xt=urn:btih:DD8255ECDC7CA55FB0BBF81323D87062DB1F6D1C"));
    let reply = harness
        .get(&format!("/api/magnet?src=6&detail_url={}&magnet_url={}", encode(DETAIL), encode(SAMPLE_MAGNET)))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["magnet_url"], SAMPLE_MAGNET);
    assert!(harness.magnet.calls().is_empty(), "已有磁力时不该再打到上游");
}

#[tokio::test]
async fn invalid_incoming_magnet_falls_back_to_fetching() {
    for bad in ["not-a-magnet", &format!("xt%3Durn%3Abtih%3A{SAMPLE_HASH}")] {
        let harness = Harness::magnet(FakeMagnet::returning(SAMPLE_MAGNET));
        let reply = harness.get(&format!("/api/magnet?src=7&detail_url={}&magnet_url={bad}", encode(DETAIL))).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.json()["magnet_url"], SAMPLE_MAGNET);
        assert_eq!(harness.magnet.calls().len(), 1, "非法磁力应被丢弃并回退到抓详情页：{bad}");
    }
}

#[tokio::test]
async fn magnet_parameter_errors_are_400_and_never_reach_upstream() {
    let cases = [
        ("/api/magnet?src=7".to_owned(), "MISSING_URL"),
        (format!("/api/magnet?src=7&detail_url={}", encode("file:///etc/passwd")), "INVALID_URL"),
        (format!("/api/magnet?detail_url={}", encode(DETAIL)), "INVALID_SRC"),
        (format!("/api/magnet?src=999&detail_url={}", encode(DETAIL)), "INVALID_SRC"),
        (format!("/api/magnet?src=all&detail_url={}", encode(DETAIL)), "INVALID_SRC"),
    ];
    for (uri, code) in cases {
        let harness = Harness::magnet(FakeMagnet::returning(SAMPLE_MAGNET));
        let reply = harness.get(&uri).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(reply.code(), code, "{uri}");
        assert!(harness.magnet.calls().is_empty(), "{uri} 不该打到上游");
    }
}

#[tokio::test]
async fn hash_case_does_not_matter() {
    let lower = format!("magnet:?xt=urn:btih:{}", SAMPLE_HASH.to_lowercase());
    let reply = Harness::magnet(FakeMagnet::returning(&lower))
        .get(&format!("/api/magnet?src=7&detail_url={}", encode(DETAIL)))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["hash"], SAMPLE_HASH);
}
