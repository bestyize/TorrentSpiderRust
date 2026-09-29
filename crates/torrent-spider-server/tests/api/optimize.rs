//! 搜索优化：相关性排序、`filter` 隐藏无关结果、`optimize` 补搜另一种数字写法。

use crate::support::*;

#[tokio::test]
async fn single_source_is_ranked_by_relevance_too() {
    // 站点自己多半按入库时间给顺序；单源和聚合的排序口径要一致
    let search = FakeSearch::default().on(0, |_, _| {
        Ok(vec![
            item(0, "Celtics City S01E08 Chapter VIII Ubuntu 720p WEB-DL.mkv"),
            item(0, "ubuntu-22.04-desktop-amd64.iso"),
        ])
    });
    let reply = Harness::search(search, sources_subset(1)).get("/api/search?q=ubuntu&src=0").await;
    assert_eq!(titles_of(&reply)[0], "ubuntu-22.04-desktop-amd64.iso");
}

/// 一条沾边、一条完全不沾边（TorrentKitty 那行表头），用来观察 filter 开关的差别。
fn mixed_search() -> FakeSearch {
    FakeSearch::default()
        .on(0, |_, _| Ok(vec![item(0, "ubuntu-24.04-desktop-amd64.iso"), item(0, "Torrent Description")]))
}

#[tokio::test]
async fn filter_is_on_by_default_like_optimize() {
    // 两者是网页端同一个开关的两半，什么都不传不能落进半开状态
    let reply = Harness::search(mixed_search(), sources_subset(1)).get("/api/search?q=ubuntu&src=0").await;
    assert_eq!(titles_of(&reply), ["ubuntu-24.04-desktop-amd64.iso"]);
}

#[tokio::test]
async fn filter_zero_keeps_everything() {
    let reply = Harness::search(mixed_search(), sources_subset(1)).get("/api/search?q=ubuntu&src=0&filter=0").await;
    assert_eq!(titles_of(&reply).len(), 2);
}

#[tokio::test]
async fn filter_and_optimize_only_accept_boolean_spellings() {
    let harness = Harness::search(mixed_search(), sources_subset(1));
    for (uri, code) in [
        ("/api/search?q=ubuntu&src=0&filter=yes", "INVALID_FILTER"),
        ("/api/search?q=ubuntu&src=0&optimize=2", "INVALID_OPTIMIZE"),
    ] {
        let reply = harness.get(uri).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(reply.code(), code);
    }
    assert!(harness.search.calls().is_empty(), "参数不合法就不该打到上游");
    assert_eq!(harness.get("/api/search?q=ubuntu&src=0&filter=TRUE&optimize=False").await.status, StatusCode::OK);
}

#[tokio::test]
async fn count_stays_the_upstream_number_after_filtering() {
    // 前端用 count 与 items 的差显示「已隐藏 N 条」；count 若也被过滤，那个提示就永远出不来
    let reply = Harness::search(mixed_search(), sources_subset(1)).get("/api/search?q=ubuntu&src=0&filter=1").await;
    assert_eq!(titles_of(&reply).len(), 1);
    assert_eq!(counts_of(&reply), [2]);
}

#[tokio::test]
async fn aggregate_counts_add_up_to_shown_plus_hidden() {
    let search = FakeSearch::default()
        .on(0, |_, _| Ok(vec![item(0, "ubuntu server image")]))
        .on(1, |_, _| Ok(vec![item(1, "ubuntu desktop image"), item(1, "完全无关的一条")]));
    let reply = Harness::search(search, sources_subset(2)).get("/api/search?q=ubuntu&src=all&filter=1").await;
    assert_eq!(counts_of(&reply), [1, 2]);
    assert_eq!(titles_of(&reply).len(), 2);
}

#[tokio::test]
async fn everything_filtered_is_200_with_empty_items() {
    let search = FakeSearch::default().on(0, |_, _| Ok(vec![item(0, "Torrent Description")]));
    let reply = Harness::search(search, sources_subset(1)).get("/api/search?q=ubuntu&src=0").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(titles_of(&reply).is_empty());
}

#[tokio::test]
async fn optimize_searches_the_other_numeral_form_by_default() {
    // 站点大多按字面搜：「复仇者联盟四」与「复仇者联盟4」的结果几乎不相交
    let harness = Harness::search(FakeSearch::default(), sources_subset(1));
    let reply = harness.get("/api/search?q=复仇者联盟四&src=0").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(queries_of(&reply), ["复仇者联盟4"]);
    assert_eq!(harness.search.count_of(0, "复仇者联盟四"), 1, "原写法仍要打一次");
    assert_eq!(harness.search.count_of(0, "复仇者联盟4"), 1, "补出的写法也要打");
}

#[tokio::test]
async fn no_variant_means_a_single_upstream_call() {
    let harness = Harness::search(FakeSearch::default(), sources_subset(1));
    let reply = harness.get("/api/search?q=windows%2011&src=0").await;
    assert!(queries_of(&reply).is_empty());
    assert_eq!(harness.search.calls().len(), 1);
}

#[tokio::test]
async fn optimize_zero_searches_literally_once() {
    let harness = Harness::search(FakeSearch::default(), sources_subset(1));
    let reply = harness.get("/api/search?q=复仇者联盟四&src=0&optimize=0").await;
    assert!(queries_of(&reply).is_empty());
    assert_eq!(harness.search.calls(), vec![(0, "复仇者联盟四".to_owned(), 1)]);
}

#[tokio::test]
async fn variant_results_are_merged_and_deduplicated() {
    // 同一资源两种写法都会返回，靠 hash 认出是同一条；留先到的（原写法）那条
    let search = FakeSearch::default().on(0, |keyword, _| {
        Ok(if keyword == "复仇者联盟四" {
            vec![hashed(0, "复仇者联盟四 2019", "AAA"), hashed(0, "复仇者联盟 单独一条", "BBB")]
        } else {
            vec![hashed(0, "复仇者联盟4 2019", "aaa"), hashed(0, "复仇者联盟4 另一条", "CCC")]
        })
    });
    let reply = Harness::search(search, sources_subset(1)).get("/api/search?q=复仇者联盟四&src=0&filter=0").await;
    let titles = titles_of(&reply);
    assert_eq!(titles.len(), 3, "AAA 只该出现一次：{titles:?}");
    assert!(titles.contains(&"复仇者联盟四 2019".to_owned()));
    assert!(!titles.contains(&"复仇者联盟4 2019".to_owned()));
    assert_eq!(counts_of(&reply), [3], "count 是去重后的条数");
}

#[tokio::test]
async fn one_failing_variant_does_not_fail_the_source() {
    let search = FakeSearch::default().on(0, |keyword, _| {
        if keyword == "复仇者联盟4" {
            Err(http_failure(504))
        } else {
            Ok(vec![hashed(0, "复仇者联盟四", "OK")])
        }
    });
    let reply = Harness::search(search, sources_subset(1)).get("/api/search?q=复仇者联盟四&src=0").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["sources"][0]["ok"], true);
    assert_eq!(titles_of(&reply).len(), 1);
}

#[tokio::test]
async fn aggregate_runs_every_variant_on_every_source() {
    let harness = Harness::search(FakeSearch::default(), sources_subset(2));
    assert_eq!(harness.get("/api/search?q=第2季&src=all").await.status, StatusCode::OK);
    for src in [0, 1] {
        assert_eq!(harness.search.count_of(src, "第2季"), 1, "源 {src} 该打原写法");
        assert_eq!(harness.search.count_of(src, "第二季"), 1, "源 {src} 该打补出的写法");
    }
}
