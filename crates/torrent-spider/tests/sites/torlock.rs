use std::sync::Arc;

use torrent_spider::site::parsers::torlock::TorLock;
use torrent_spider::testing::*;
use torrent_spider::{DefaultSiteEndpoints, SearchService, SiteEndpointProvider, TorrentInfo, TorrentSrc};

const TORLOCK_URL: &str = "https://www.torlock.com";

const SEARCH_PAGE: &str = r#"<html><body>
  <div class="table-responsive"></div>
  <div class="table-responsive">
    <table>
      <tr><td class="ts">1 GB</td><td class="td">2026-03-15</td><td><div><a href="/torrent/abc">TorLock Title</a></div></td></tr>
      <tr><td class="ts"></td><td class="td">ignored</td><td><div><a href="/other/abc">Ignored</a></div></td></tr>
    </table>
  </div>
</body></html>"#;

fn service_with(http: Arc<FakeHttp>) -> TorLock {
    TorLock::new(http, DefaultSiteEndpoints.endpoint_for(TorrentSrc::TorLock))
}

fn parse(body: &str) -> Vec<TorrentInfo> {
    service_with(FakeHttp::not_found()).parse_search_response(body)
}

#[test]
fn extracts_rows_from_the_second_table_on_the_page() {
    let result = parse(SEARCH_PAGE);

    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.src, TorrentSrc::TorLock.code());
    assert_eq!(item.title.as_deref(), Some("TorLock Title"));
    assert_eq!(item.detail_url.as_deref(), Some(format!("{TORLOCK_URL}/torrent/abc").as_str()));
    assert_eq!(item.size.as_deref(), Some("1 GB"));
    assert_eq!(item.date.as_deref(), Some("2026-03-15"));
}

#[test]
fn skips_rows_without_a_size() {
    let result = parse(SEARCH_PAGE);

    assert_eq!(result.len(), 1);
    assert!(result.iter().all(|it| it.title.as_deref() != Some("Ignored")));
}

#[test]
fn finds_the_listing_table_by_content_when_the_table_count_changes() {
    // 页面顶部的空表若被去掉，第二张就不再是列表，这时按内容找
    let result = parse(
        r#"<html><body>
  <div class="table-responsive">
    <table>
      <tr><td class="ts">1 GB</td><td class="td">2026-03-15</td><td><div><a href="/torrent/abc">Only Table</a></div></td></tr>
    </table>
  </div>
</body></html>"#,
    );

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("Only Table"));
}

#[test]
fn ignores_links_that_do_not_point_at_a_torrent_detail_page() {
    let result = parse(
        r#"<html><body>
  <div class="table-responsive"></div>
  <div class="table-responsive">
    <table>
      <tr><td class="ts">1 GB</td><td class="td">d</td><td><div><a href="/other/abc">Wrong link</a></div></td></tr>
    </table>
  </div>
</body></html>"#,
    );

    assert!(result.is_empty());
}

#[test]
fn empty_page_yields_no_rows() {
    assert!(parse("").is_empty());
    assert!(parse("<html><body></body></html>").is_empty());
}

#[tokio::test]
async fn reads_the_magnet_from_the_icon_parent_link() {
    let detail = FakeHttp::serving(&format!(
        r#"<html><body><a href="{}"><i class="fa-magnet"></i></a></body></html>"#,
        magnet(HEX)
    ));
    let service = service_with(detail);

    assert_eq!(service.find_magnet_url(&format!("{TORLOCK_URL}/torrent/abc")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn detail_page_falls_back_to_scanning_for_a_magnet_href() {
    let detail = FakeHttp::serving(&format!(r#"<html><body><a href="{}">Magnet</a></body></html>"#, magnet(HEX)));
    let service = service_with(detail);

    assert_eq!(service.find_magnet_url(&format!("{TORLOCK_URL}/torrent/abc")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn detail_page_without_a_magnet_yields_none() {
    let detail = FakeHttp::serving("<html><body><p>nothing</p></body></html>");
    let service = service_with(detail);

    assert_eq!(service.find_magnet_url(&format!("{TORLOCK_URL}/torrent/abc")).await, None);
}

#[tokio::test]
async fn search_url_carries_the_term_and_page() {
    let server = FakeHttp::serving(SEARCH_PAGE);
    service_with(server.clone()).search("女", 2).await.unwrap();

    assert_eq!(server.requested_urls(), [format!("{TORLOCK_URL}/all/torrents/%E5%A5%B3/2.html")]);
}

#[tokio::test]
async fn a_custom_endpoint_redirects_every_request() {
    let mirror = FakeHttp::serving(SEARCH_PAGE);
    let mirrored = TorLock::new(mirror.clone(), endpoint("https://torlock.example"));

    mirrored.search("x", 1).await.unwrap();
    assert_eq!(mirror.requested_urls(), ["https://torlock.example/all/torrents/x/1.html"]);
}

/// 2026-09 实测的页面结构：标题前有分类链接，日期在 <time datetime> 里，做种 / 吸血在 tul / tdl。
const LIVE_LAYOUT_PAGE: &str = r#"<html><body>
  <div class="table-responsive"><table><tr><th>Sort</th></tr></table></div>
  <div class="table-responsive"><div class=tl-view-list><table class="table tl-list">
    <thead><th class=th1>TORRENT NAME</th><th class=th3>SIZE</th></thead>
    <tr><td class="tn"><a class="tl-cat c5" href="/software.html">APP</a><a class="tl-name" href="/torrent/63489835/ubuntu-22-04-desktop-amd64.html">Ubuntu-22.04-desktop-amd64</a><span class="tl-flags"><a class="tl-ver" href="/veriplus.html">&#10003;</a></span></td>
      <td class=td><time datetime="2022-05-19" data-ld>5/19/2022</time></td><td class=ts>3.4 GB</td><td class="tul h6">39</td><td class=tdl>0</td></tr>
  </table></div></div>
</body></html>"#;

#[test]
fn live_layout_skips_category_link_and_reads_machine_date() {
    let service = TorLock::new(FakeHttp::new(), endpoint(TORLOCK_URL));
    let items = service.parse_search_response(LIVE_LAYOUT_PAGE);
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.title.as_deref(), Some("Ubuntu-22.04-desktop-amd64"));
    assert_eq!(
        item.detail_url.as_deref(),
        Some("https://www.torlock.com/torrent/63489835/ubuntu-22-04-desktop-amd64.html")
    );
    assert_eq!(item.size.as_deref(), Some("3.4 GB"));
    assert_eq!(item.date.as_deref(), Some("2022-05-19"));
    assert_eq!(item.sender_count.as_deref(), Some("39"));
    assert_eq!(item.leacher_count.as_deref(), Some("0"));
}
