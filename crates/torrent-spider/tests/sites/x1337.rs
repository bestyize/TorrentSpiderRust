use std::sync::Arc;

use torrent_spider::site::parsers::x1337::X1337;
use torrent_spider::testing::*;
use torrent_spider::{DefaultSiteEndpoints, SearchService, SiteEndpointProvider, TorrentInfo, TorrentSrc};

const X1337_URL: &str = "https://www.1337xx.to";
const MIRROR_DETAIL: &str = "https://1337x.tube/torrent/xyz";

const SEARCH_PAGE: &str = r#"<html><body>
  <table><tbody>
    <tr>
      <td class="coll-1 name">
        <a href="https://1337x.pro/cat/tv">cat</a>
        <a class="openAdd" href="https://1337x.pro/torrent/xyz" id-text="1337 Full Title From Attribute">1337 Title...</a>
      </td>
      <td class="coll-2 seeds">77</td>
      <td class="coll-3 leeches">11</td>
      <td class="coll-4 size mob-uploader">8 GB</td>
      <td class="coll-4 size mob-uploader">Uploader</td>
      <td class="coll-date">'2026-03-15'</td>
    </tr>
  </tbody></table>
</body></html>"#;

fn detail_page() -> String {
    format!(
        r#"<html><body>
  <h1>1337 Title</h1>
  <a id="openPopup" href="{MIRROR_DETAIL}">Magnet Download</a>
  Info Hash : {HEX}
</body></html>"#
    )
}

/// 镜像详情页：磁力和种子链接都在这里。
fn mirror_page() -> String {
    format!(
        r#"<html><body>
  <a href="{}">Magnet Download</a>
  <a target="_blank" href="http://itorrents.org/torrent/{HEX}.torrent">ITORRENTS MIRROR</a>
</body></html>"#,
        magnet_named(HEX, "1337+Title")
    )
}

fn server() -> Arc<FakeHttp> {
    FakeHttp::new()
        .route(&format!("{X1337_URL}/search/"), SEARCH_PAGE)
        .route(&format!("{X1337_URL}/torrent/"), &detail_page())
        .route(MIRROR_DETAIL, &mirror_page())
}

fn service_with(http: Arc<FakeHttp>) -> X1337 {
    X1337::new(http, DefaultSiteEndpoints.endpoint_for(TorrentSrc::X1337))
}

fn parse(body: &str) -> Vec<TorrentInfo> {
    service_with(FakeHttp::not_found()).parse_search_response(body)
}

#[test]
fn extracts_every_column_of_a_search_row() {
    let result = parse(SEARCH_PAGE);

    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.src, TorrentSrc::X1337.code());
    assert_eq!(item.title.as_deref(), Some("1337 Full Title From Attribute"));
    assert_eq!(item.sender_count.as_deref(), Some("77"));
    assert_eq!(item.leacher_count.as_deref(), Some("11"));
    assert_eq!(item.size.as_deref(), Some("8 GB"));
    assert_eq!(item.date.as_deref(), Some("2026-03-15"));
    assert_eq!(item.detail_url.as_deref(), Some("https://1337x.pro/torrent/xyz"));
}

#[test]
fn falls_back_to_the_anchor_text_when_id_text_is_missing() {
    let result = parse(
        r#"<html><body><table><tbody>
  <tr>
    <td class="coll-1 name"><a href="https://1337x.pro/torrent/1">Plain Title</a></td>
    <td class="coll-4 size mob-uploader">8 GB</td>
  </tr>
</tbody></table></body></html>"#,
    );

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("Plain Title"));
}

#[test]
fn falls_back_to_a_looser_selector_when_the_class_names_change() {
    let result = parse(
        r#"<html><body><table><tbody>
  <tr>
    <td class="name"><a href="https://1337x.pro/torrent/1">Renamed</a></td>
    <td class="size">2 GB</td>
  </tr>
</tbody></table></body></html>"#,
    );

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("Renamed"));
    assert_eq!(result[0].size.as_deref(), Some("2 GB"));
}

#[test]
fn keeps_relative_detail_links_on_the_configured_host() {
    let result = parse(
        r#"<html><body><table><tbody>
  <tr><td class="coll-1 name"><a href="/torrent/rel">Relative</a></td></tr>
</tbody></table></body></html>"#,
    );

    assert_eq!(result[0].detail_url, Some(format!("{X1337_URL}/torrent/rel")));
}

#[test]
fn empty_page_yields_no_rows() {
    assert!(parse("").is_empty());
    assert!(parse("<html><body></body></html>").is_empty());
}

#[tokio::test]
async fn reads_a_magnet_straight_off_the_detail_page() {
    let detail =
        FakeHttp::serving(&format!(r#"<html><body><a href="{}">Magnet Download</a></body></html>"#, magnet(HEX)));
    let service = service_with(detail.clone());

    assert_eq!(service.find_magnet_url(&format!("{X1337_URL}/torrent/1")).await, Some(magnet(HEX)));
    // 主详情页已有磁力，不该再去镜像页
    assert_eq!(detail.requests().len(), 1);
}

#[tokio::test]
async fn follows_the_mirror_detail_page_when_the_main_page_has_no_magnet() {
    let server = server();
    let service = service_with(server.clone());

    assert_eq!(
        service.find_magnet_url(&format!("{X1337_URL}/torrent/xyz")).await,
        Some(magnet_named(HEX, "1337+Title"))
    );
    assert_eq!(server.requested_urls(), [format!("{X1337_URL}/torrent/xyz"), MIRROR_DETAIL.to_owned()]);
}

#[tokio::test]
async fn builds_a_magnet_from_the_info_hash_when_no_link_exists() {
    let detail = FakeHttp::serving(&format!("<html><body><h1>Built Title</h1>Info Hash : {HEX}</body></html>"));
    let service = service_with(detail);

    assert_eq!(
        service.find_magnet_url(&format!("{X1337_URL}/torrent/1")).await,
        Some(format!("magnet:?xt=urn:btih:{HEX}&dn=Built%20Title"))
    );
}

#[tokio::test]
async fn accepts_a_base32_info_hash_in_the_page_text() {
    let detail = FakeHttp::serving(&format!("<html><body><h1>T</h1>Info Hash : {BASE32}</body></html>"));
    let service = service_with(detail);

    let found = service.find_magnet_url(&format!("{X1337_URL}/torrent/1")).await.unwrap();
    assert!(found.contains(HEX), "{found}");
}

#[tokio::test]
async fn returns_none_when_the_page_carries_no_hash_at_all() {
    let detail = FakeHttp::serving("<html><body><h1>Nothing</h1></body></html>");
    let service = service_with(detail);

    assert_eq!(service.find_magnet_url(&format!("{X1337_URL}/torrent/1")).await, None);
}

#[tokio::test]
async fn prefers_the_mirror_torrent_link() {
    let server = server();
    let service = service_with(server.clone());

    assert_eq!(
        service.find_torrent_url(&format!("{X1337_URL}/torrent/xyz")).await,
        Some(format!("http://itorrents.org/torrent/{HEX}.torrent"))
    );
    assert_eq!(server.requested_urls(), [format!("{X1337_URL}/torrent/xyz"), MIRROR_DETAIL.to_owned()]);
}

#[tokio::test]
async fn ignores_stale_links_to_caches_that_no_longer_serve_files() {
    // torrage.info 和 btcache.me 已经改作他用，页面上的残留链接不可信，
    // 这时应当回退到按 info hash 自行拼地址
    let detail = FakeHttp::serving(&format!(
        r#"<html><body>
  <h1>T</h1>
  Info Hash : {HEX}
  <a href="https://torrage.info/torrent.php?h={HEX}">Torrage</a>
  <a href="https://btcache.me/torrent/{HEX}">btcache</a>
</body></html>"#
    ));
    let service = service_with(detail);

    assert_eq!(service.find_torrent_url(&format!("{X1337_URL}/torrent/1")).await, Some(torrent_url(HEX)));
}

#[tokio::test]
async fn an_existing_torrent_url_passes_straight_through() {
    let server = server();
    let service = service_with(server.clone());
    let url = "https://example.com/x.torrent";

    assert_eq!(service.find_torrent_url(url).await.as_deref(), Some(url));
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn search_url_carries_the_term_and_page() {
    let server = server();
    service_with(server.clone()).search("女", 2).await.unwrap();

    assert_eq!(server.requested_urls(), [format!("{X1337_URL}/search/%E5%A5%B3/2/")]);
}

#[tokio::test]
async fn a_custom_endpoint_redirects_every_request() {
    let mirror = FakeHttp::serving(SEARCH_PAGE);
    let mirrored = X1337::new(mirror.clone(), endpoint("https://1337.example"));

    mirrored.search("x", 1).await.unwrap();
    assert_eq!(mirror.requested_urls(), ["https://1337.example/search/x/1/"]);
}
