use std::sync::Arc;

use torrent_spider::site::parsers::yuhuage::YuHuaGe;
use torrent_spider::testing::*;
use torrent_spider::{DefaultSiteEndpoints, SearchService, SiteEndpointProvider, TorrentInfo, TorrentSrc};

const YUHUA_URL: &str = "https://www.yuhuage.win";

const SEARCH_PAGE: &str = r#"<html><body>
  <div class="search-item">
    <div class="item-title"><a href="/detail/yh">YuHuaGe Title</a></div>
    <div class="item-bar"><span>2026-03-15</span><span>900 MB</span></div>
  </div>
</body></html>"#;

fn service_with(http: Arc<FakeHttp>) -> YuHuaGe {
    YuHuaGe::new(http, DefaultSiteEndpoints.endpoint_for(TorrentSrc::YuHuaGe))
}

fn parse(body: &str) -> Vec<TorrentInfo> {
    service_with(FakeHttp::not_found()).parse_search_response(body)
}

#[test]
fn extracts_every_field_of_a_search_item() {
    let result = parse(SEARCH_PAGE);

    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.src, TorrentSrc::YuHuaGe.code());
    assert_eq!(item.title.as_deref(), Some("YuHuaGe Title"));
    assert_eq!(item.detail_url, Some(format!("{YUHUA_URL}/detail/yh")));
    assert_eq!(item.date.as_deref(), Some("2026-03-15"));
    assert_eq!(item.size.as_deref(), Some("900 MB"));
}

#[test]
fn falls_back_to_a_substring_match_when_the_container_class_changes() {
    let renamed = SEARCH_PAGE.replace(r#"class="search-item""#, r#"class="search-item-wide""#);

    let result = parse(&renamed);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("YuHuaGe Title"));
}

#[test]
fn finds_the_size_by_content_when_the_meta_spans_shift() {
    // 位置 1 空掉时按内容特征找回尺寸
    let result = parse(
        r#"<html><body>
  <div class="search-item">
    <div class="item-title"><a href="/detail/yh">Title</a></div>
    <div class="item-bar"><span>2026-03-15</span><span></span><span>1.2 GB</span></div>
  </div>
</body></html>"#,
    );

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].size.as_deref(), Some("1.2 GB"));
}

#[test]
fn skips_items_without_a_title_link() {
    let result = parse(
        r#"<html><body>
  <div class="search-item"><div class="item-bar"><span>a</span><span>b</span></div></div>
  <div class="search-item"><div class="item-title"><a href="/detail/ok">Kept</a></div></div>
</body></html>"#,
    );

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("Kept"));
}

#[test]
fn empty_page_yields_no_items() {
    assert!(parse("").is_empty());
    assert!(parse("<html><body></body></html>").is_empty());
}

#[tokio::test]
async fn reads_the_magnet_from_the_download_block() {
    let detail = format!(r#"<html><body><a class="download" href="{}">Download</a></body></html>"#, magnet(HEX));
    let service = service_with(FakeHttp::serving(&detail));

    assert_eq!(service.find_magnet_url(&format!("{YUHUA_URL}/detail/yh")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn detail_page_falls_back_to_scanning_for_a_magnet_href() {
    let detail = format!(r#"<html><body><a href="{}">Download</a></body></html>"#, magnet(HEX));
    let service = service_with(FakeHttp::serving(&detail));

    assert_eq!(service.find_magnet_url(&format!("{YUHUA_URL}/detail/yh")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn detail_page_without_a_magnet_yields_empty() {
    let service = service_with(FakeHttp::serving("<html><body><p>nothing</p></body></html>"));

    assert_eq!(service.find_magnet_url(&format!("{YUHUA_URL}/detail/yh")).await, None);
}

#[tokio::test]
async fn search_url_carries_the_term_and_page() {
    let http = FakeHttp::serving(SEARCH_PAGE);
    let service = service_with(http.clone());

    service.search("女", 2).await.unwrap();

    assert_eq!(http.last_request().unwrap().url, format!("{YUHUA_URL}/search/%E5%A5%B3-2.html"));
}

#[tokio::test]
async fn search_sends_browser_like_headers_because_the_site_rejects_bare_requests() {
    let http = FakeHttp::serving(SEARCH_PAGE);
    let service = service_with(http.clone());

    service.search("x", 1).await.unwrap();

    // Kotlin 版只断言发出了请求；这里能直接看到请求头，顺带确认 UA 真的带上了
    let request = http.last_request().expect("search should hit the site");
    let user_agent = request.header("User-Agent").unwrap_or_default();
    assert!(user_agent.starts_with("Mozilla/5.0"), "unexpected UA: {user_agent:?}");
}

#[tokio::test]
async fn a_custom_endpoint_redirects_every_request() {
    let mirror = FakeHttp::serving(SEARCH_PAGE);
    let mirrored = YuHuaGe::new(mirror.clone(), endpoint("https://yuhua.example"));

    mirrored.search("x", 1).await.unwrap();

    assert_eq!(mirror.last_request().unwrap().url, "https://yuhua.example/search/x-1.html");
}
