use std::sync::Arc;

use torrent_spider::site::parsers::torrentkitty::TorrentKitty;
use torrent_spider::testing::*;
use torrent_spider::{DefaultSiteEndpoints, SearchService, SiteEndpointProvider, TorrentInfo, TorrentSrc};

const KITTY_URL: &str = "https://www.torrentkitty.tv";

fn action_links() -> String {
    format!(
        r#"<a href="/detail/kitty">detail</a>
        <a href="{}">magnet</a>"#,
        magnet(HEX)
    )
}

fn search_page() -> String {
    format!(
        r#"<html><body>
  <table id="archiveResult">
    <tr>
      <td class="name">Kitty Title</td>
      <td class="size">2 GB</td>
      <td class="date">2026-03-15</td>
      <td class="action">
        {}
      </td>
    </tr>
  </table>
</body></html>"#,
        action_links()
    )
}

fn service_with(http: Arc<FakeHttp>) -> TorrentKitty {
    TorrentKitty::new(http, DefaultSiteEndpoints.endpoint_for(TorrentSrc::TorrentKitty))
}

fn parse(body: &str) -> Vec<TorrentInfo> {
    service_with(FakeHttp::not_found()).parse_search_response(body)
}

#[test]
fn extracts_every_field_of_an_archive_row() {
    let result = parse(&search_page());

    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.src, TorrentSrc::TorrentKitty.code());
    assert_eq!(item.title.as_deref(), Some("Kitty Title"));
    assert_eq!(item.hash.as_deref(), Some(HEX));
    assert_eq!(item.magnet_url, Some(magnet(HEX)));
    assert_eq!(item.torrent_url, Some(torrent_url(HEX)));
    assert_eq!(item.detail_url, Some(format!("{KITTY_URL}/detail/kitty")));
    assert_eq!(item.size.as_deref(), Some("2 GB"));
    assert_eq!(item.date.as_deref(), Some("2026-03-15"));
}

#[test]
fn tells_detail_and_magnet_apart_by_href_rather_than_by_position() {
    // 老实现取 action 单元格里第 0 个和第 1 个 <a>，两个链接顺序一换就取错
    let page = search_page();
    let swapped_links = format!(
        r#"<a href="{}">magnet</a>
        <a href="/detail/kitty">detail</a>"#,
        magnet(HEX)
    );
    let swapped = page.replace(&action_links(), &swapped_links);
    assert_ne!(swapped, page, "fixture replacement must take effect");

    let result = parse(&swapped);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].hash.as_deref(), Some(HEX));
    assert_eq!(result[0].detail_url, Some(format!("{KITTY_URL}/detail/kitty")));
}

#[test]
fn falls_back_to_content_matching_for_the_size() {
    let result = parse(&format!(
        r#"<html><body>
  <table id="archiveResult">
    <tr>
      <td class="name">Title</td>
      <td>6.5 GB</td>
      <td class="date">2026-03-15</td>
      <td class="action"><a href="{}">m</a></td>
    </tr>
  </table>
</body></html>"#,
        magnet(HEX)
    ));

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].size.as_deref(), Some("6.5 GB"));
}

#[test]
fn skips_rows_without_a_name() {
    let m = magnet(HEX);
    let result = parse(&format!(
        r#"<html><body>
  <table id="archiveResult">
    <tr><td class="size">1 GB</td><td class="action"><a href="{m}">m</a></td></tr>
    <tr><td class="name">Kept</td><td class="action"><a href="{m}">m</a></td></tr>
  </table>
</body></html>"#
    ));

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("Kept"));
}

#[test]
fn empty_page_yields_no_rows() {
    assert!(parse("").is_empty());
    assert!(parse("<html><body></body></html>").is_empty());
}

#[tokio::test]
async fn magnet_is_derived_from_the_hash_in_the_detail_url() {
    let server = FakeHttp::not_found();
    let service = service_with(server.clone());

    assert_eq!(service.find_magnet_url(&format!("{KITTY_URL}/{HEX_LOWER}")).await, Some(magnet(HEX)));
    // 末段就是哈希，不该再请求详情页
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn an_already_built_magnet_passes_through_unchanged() {
    let server = FakeHttp::not_found();
    let service = service_with(server.clone());

    assert_eq!(service.find_magnet_url(&magnet(HEX)).await, Some(magnet(HEX)));
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn a_detail_url_without_a_usable_hash_is_fetched_instead() {
    let detail =
        FakeHttp::serving(&format!(r#"<html><body><a class="magnet-link" href="{}">m</a></body></html>"#, magnet(HEX)));
    let service = service_with(detail.clone());

    assert_eq!(service.find_magnet_url(&format!("{KITTY_URL}/some/page")).await, Some(magnet(HEX)));
    assert_eq!(detail.requested_urls(), [format!("{KITTY_URL}/some/page")]);
}

#[tokio::test]
async fn an_unusable_magnet_on_the_detail_page_yields_none() {
    let detail = FakeHttp::serving(r#"<html><body><a class="magnet-link">magnet:?xt=urn:btih:SHORT</a></body></html>"#);
    let service = service_with(detail);

    assert_eq!(service.find_magnet_url(&format!("{KITTY_URL}/some/page")).await, None);
}

#[tokio::test]
async fn search_url_carries_the_term_and_page() {
    let server = FakeHttp::serving(&search_page());
    service_with(server.clone()).search("女", 2).await.unwrap();

    assert_eq!(server.requested_urls(), [format!("{KITTY_URL}/search/%E5%A5%B3/2")]);
}

#[tokio::test]
async fn a_custom_endpoint_redirects_every_request() {
    let mirror = FakeHttp::serving(&search_page());
    let mirrored = TorrentKitty::new(mirror.clone(), endpoint("https://kitty.example"));

    mirrored.search("x", 1).await.unwrap();
    assert_eq!(mirror.requested_urls(), ["https://kitty.example/search/x/1"]);
}

#[test]
fn header_row_with_name_class_is_not_an_item() {
    // 2026-09 实测：表头 <th class="name"> 也带 name 类，但整行没有任何链接
    let page = format!(
        r#"<table id="archiveResult">
          <tr><th class="name">Torrent Description</th><th class="size">Torrent Size</th><th class="date">Upload Date</th><td class="action"></td></tr>
          <tr><td class="name">ubuntu.iso</td><td class="size">223.04 kb</td><td class="date">2026-07-20</td>
              <td class="action"><a href="/information/{HEX}">Detail</a><a href="{}">Magnet</a></td></tr>
        </table>"#,
        magnet(HEX)
    );
    let items = TorrentKitty::new(FakeHttp::new(), endpoint(KITTY_URL)).parse_search_response(&page);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title.as_deref(), Some("ubuntu.iso"));
    assert_eq!(items[0].hash.as_deref(), Some(HEX));
}
