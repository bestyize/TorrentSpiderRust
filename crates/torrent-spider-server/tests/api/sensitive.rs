//! 违禁内容过滤在接口层的装配：按请求方 IP 过滤、不透露拦截数量、health 诊断。
//!
//! 查询词（linux）故意跟标题对不上，所以一律带 `filter=0` 关掉相关性过滤，
//! 否则标题会先被它滤光，测的就不是违禁过滤了。

use torrent_spider::testing::FakeHttp;
use torrent_spider_server::sensitive::lexicon::SensitiveLexicon;
use torrent_spider_server::{SensitiveConfig, SensitiveFilter};

use crate::support::*;

//
// 查询词（linux）故意跟标题对不上，所以一律带 filter=0 关掉相关性过滤，
// 否则标题会先被它滤光，测的就不是违禁过滤了。

const CHINA_IP: &str = "1.2.3.9:40000";
const FOREIGN_IP: &str = "8.8.8.8:40000";

/// 就绪的过滤器：注入两个词，不依赖打包的那份——上游词库更新不该让路由层测试变红。
async fn ready_filter() -> Arc<SensitiveFilter> {
    // 刷新只在这里做一次，之后不再写缓存，所以目录用完即删
    let cache = tempfile::tempdir().unwrap();
    // 违禁词表编在二进制里，假站点只该被请求 IP 段这一个地址
    let http = FakeHttp::new().route("https://example.test/ips", "1.2.3.0/24\n");
    let config = SensitiveConfig {
        ip_list_url: "https://example.test/ips.txt".into(),
        cache_dir: cache.path().to_owned(),
        ..SensitiveConfig::default()
    };
    let filter = SensitiveFilter::with_lexicon(config, http, SensitiveLexicon::build(["奶子", "写真"]));
    filter.refresh().await;
    assert!(filter.is_ready(), "测试用的过滤器应当已经就绪");
    Arc::new(filter)
}

async fn filtered_harness(search: FakeSearch, trust_forwarded_for: bool) -> Harness {
    let mut harness = Harness::search(search, sources_subset(1));
    harness.state = harness.state.clone().with_sensitive(ready_filter().await, trust_forwarded_for);
    harness
}

fn banned_search() -> FakeSearch {
    FakeSearch::default().on(0, |_, _| {
        Ok(vec![hashed(0, "奶子大合集", "B1"), hashed(0, "写真集", "B2"), hashed(0, "ubuntu-24.04.iso", "G1")])
    })
}

#[tokio::test]
async fn banned_items_disappear_for_china_ips() {
    let harness = filtered_harness(banned_search(), false).await;
    let reply = harness.request("/api/search?q=linux&src=0&filter=0", Some(CHINA_IP), &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(titles_of(&reply), ["ubuntu-24.04.iso"]);
}

#[tokio::test]
async fn blocked_items_do_not_leak_through_count() {
    // 前端渲染「已隐藏 count - items.length 条」，所以 count 必须是违禁过滤之后的数
    let harness = filtered_harness(banned_search(), false).await;
    let reply = harness.request("/api/search?q=linux&src=0&filter=0", Some(CHINA_IP), &[]).await;
    assert_eq!(counts_of(&reply), [1]);
}

#[tokio::test]
async fn foreign_ips_are_not_filtered() {
    let harness = filtered_harness(banned_search(), false).await;
    let reply = harness.request("/api/search?q=linux&src=0&filter=0", Some(FOREIGN_IP), &[]).await;
    assert_eq!(titles_of(&reply).len(), 3);
}

#[tokio::test]
async fn forwarded_for_cannot_be_spoofed_unless_trusted() {
    // 默认只看 TCP 对端：客户端自己加一行 XFF 就能改来源的话，过滤等于没有
    let spoofed = [("x-forwarded-for", "8.8.8.8")];
    let direct = filtered_harness(banned_search(), false).await;
    let reply = direct.request("/api/search?q=linux&src=0&filter=0", Some(CHINA_IP), &spoofed).await;
    assert_eq!(titles_of(&reply).len(), 1, "大陆对端伪造境外 XFF 也绕不过去");

    let behind_proxy = filtered_harness(banned_search(), true).await;
    let via_proxy = [("x-forwarded-for", "9.9.9.9, 1.2.3.9")];
    let reply = behind_proxy.request("/api/search?q=linux&src=0&filter=0", Some("10.0.0.2:80"), &via_proxy).await;
    assert_eq!(titles_of(&reply).len(), 1, "反代部署时认 XFF 的最后一段");
}

#[tokio::test]
async fn searching_a_banned_word_from_china_returns_nothing() {
    let harness = filtered_harness(banned_search(), false).await;
    let reply = harness.request("/api/search?q=奶子&src=0&filter=0", Some(CHINA_IP), &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(titles_of(&reply).is_empty(), "搜违禁词等于什么都不给");
}

#[tokio::test]
async fn banned_items_brought_in_by_variants_are_blocked_too() {
    let search = FakeSearch::default().on(0, |keyword, _| {
        Ok(if keyword == "第12季" {
            vec![hashed(0, "第12季 奶子特辑", "VIA-VARIANT")]
        } else {
            vec![hashed(0, "第十二季 正常版", "OK")]
        })
    });
    let harness = filtered_harness(search, false).await;
    let reply = harness.request("/api/search?q=第十二季&src=0", Some(CHINA_IP), &[]).await;
    assert_eq!(titles_of(&reply), ["第十二季 正常版"]);
}

#[tokio::test]
async fn without_a_filter_nothing_changes_and_health_reports_null() {
    let harness = Harness::search(banned_search(), sources_subset(1));
    let reply = harness.request("/api/search?q=linux&src=0&filter=0", Some(CHINA_IP), &[]).await;
    assert_eq!(titles_of(&reply).len(), 3);
    let health = harness.get("/api/health").await.json();
    assert!(health.get("filter").is_some_and(Value::is_null));
}

#[tokio::test]
async fn health_reports_how_this_request_was_judged() {
    // 内网手机连上来看到违禁内容时，靠这一块才能看出服务端把它判成了境外
    let harness = filtered_harness(FakeSearch::default(), false).await;
    let filter = harness.request("/api/health", Some("192.168.18.169:5000"), &[]).await.json()["filter"].clone();
    assert_eq!(filter["enabled"], true);
    assert_eq!(filter["ready"], true);
    assert_eq!(filter["force_china"], false);
    assert_eq!(filter["lexicon_entries"], 2);
    assert_eq!(filter["china_ranges"], 1);
    assert_eq!(filter["client_ip"], "192.168.18.169");
    assert_eq!(filter["is_china"], false);

    let china = harness.request("/api/health", Some(CHINA_IP), &[]).await.json()["filter"].clone();
    assert_eq!(china["client_ip"], "1.2.3.9");
    assert_eq!(china["is_china"], true);

    let unknown = harness.get("/api/health").await.json()["filter"].clone();
    assert!(unknown["client_ip"].is_null(), "拿不到对端地址时如实报 null");
    assert_eq!(unknown["is_china"], false);
}
