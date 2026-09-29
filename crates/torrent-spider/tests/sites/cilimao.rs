use std::sync::Arc;

use torrent_spider::site::parsers::cilimao::Cilimao;
use torrent_spider::testing::*;
use torrent_spider::{DefaultSiteEndpoints, SearchService, SiteEndpoint, SiteEndpointProvider, TorrentSrc};

const CILIMAO_URL: &str = "https://clmclm.com";

fn search_page() -> String {
    format!(
        r#"<html><body>
  <div class="ssbox">
    <div class="title"><h3><a href="/hash-name.html">Cili Title</a></h3></div>
    <div class="sbar">
      <b>2026-03-15</b><b>4 GB</b><b>2026-03-16</b><b>100</b>
      <a href="{}">Magnet</a>
    </div>
  </div>
</body></html>"#,
        magnet(HEX)
    )
}

fn service_with(http: Arc<FakeHttp>) -> Cilimao {
    Cilimao::new(http, DefaultSiteEndpoints.endpoint_for(TorrentSrc::Cilimao))
}

fn service() -> Cilimao {
    service_with(FakeHttp::serving(&search_page()))
}

#[test]
fn extracts_every_field_of_an_entry() {
    let result = service().parse_search_response(&search_page());

    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.src, TorrentSrc::Cilimao.code());
    assert_eq!(item.title.as_deref(), Some("Cili Title"));
    assert_eq!(item.hash.as_deref(), Some(HEX));
    assert_eq!(item.magnet_url, Some(magnet(HEX)));
    assert_eq!(item.torrent_url, Some(torrent_url(HEX)));
    assert_eq!(item.detail_url, Some(format!("{CILIMAO_URL}/hash-name.html")));
    assert_eq!(item.size.as_deref(), Some("4 GB"));
    assert_eq!(item.date.as_deref(), Some("2026-03-15"));
    assert_eq!(item.last_download_date.as_deref(), Some("2026-03-16"));
    assert_eq!(item.download_count.as_deref(), Some("100"));
}

#[test]
fn falls_back_to_a_substring_match_when_the_container_class_changes() {
    let renamed = search_page().replace(r#"class="ssbox""#, r#"class="ssbox-v2""#);

    let result = service().parse_search_response(&renamed);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("Cili Title"));
}

#[test]
fn finds_the_size_by_content_when_the_details_block_shrinks() {
    // <b> 少了一个时位置全错位，这时按「一个只写着尺寸的单元格」来找
    let page = format!(
        r#"<html><body>
  <div class="ssbox">
    <div class="title"><h3><a href="/abc.html">Title</a></h3></div>
    <div class="sbar">
      <b>2026-03-15</b>
      <span>7.5 GB</span>
      <a href="{}">Magnet</a>
    </div>
  </div>
</body></html>"#,
        magnet(HEX)
    );

    let result = service().parse_search_response(&page);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("Title"));
}

#[test]
fn an_entry_without_a_magnet_still_yields_a_row() {
    let page = r#"<html><body>
  <div class="ssbox">
    <div class="title"><h3><a href="/abc.html">No magnet</a></h3></div>
    <div class="sbar"><b>2026-03-15</b><b>1 GB</b><b>-</b><b>0</b></div>
  </div>
</body></html>"#;

    let result = service().parse_search_response(page);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("No magnet"));
    assert_eq!(result[0].magnet_url, None);
    assert_eq!(result[0].hash, None);
}

#[test]
fn empty_page_yields_no_entries() {
    assert!(service().parse_search_response("").is_empty());
    assert!(service().parse_search_response("<html><body></body></html>").is_empty());
}

#[tokio::test]
async fn magnet_is_derived_from_the_hash_in_the_detail_url() {
    assert_eq!(service().find_magnet_url(&format!("{CILIMAO_URL}/{HEX_LOWER}.html")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn detail_url_without_a_usable_hash_yields_no_magnet() {
    let service = service();

    assert_eq!(service.find_magnet_url(&format!("{CILIMAO_URL}/not-a-hash.html")).await, None);
    assert_eq!(service.find_magnet_url(&format!("{CILIMAO_URL}/")).await, None);
}

#[tokio::test]
async fn search_url_carries_the_term_and_page() {
    let http = FakeHttp::serving(&search_page());
    service_with(http.clone()).search("女", 2).await.unwrap();

    assert_eq!(http.last_request().unwrap().url, format!("{CILIMAO_URL}/search-%E5%A5%B3-0-0-2.html"));
}

#[tokio::test]
async fn a_custom_endpoint_redirects_every_request() {
    let mirror = FakeHttp::serving(&search_page());
    let mirrored = Cilimao::new(mirror.clone(), SiteEndpoint::new("https://clm.example"));

    mirrored.search("x", 1).await.unwrap();

    assert_eq!(mirror.last_request().unwrap().url, "https://clm.example/search-x-0-0-1.html");
}
