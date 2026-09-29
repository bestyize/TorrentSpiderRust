use std::sync::Arc;

use torrent_spider::site::parsers::zeromagnet::ZeroMagnet;
use torrent_spider::testing::*;
use torrent_spider::{DefaultSiteEndpoints, SearchService, SiteEndpointProvider, TorrentInfo, TorrentSrc};

const ZERO_URL: &str = "https://0magnet.com";

const SEARCH_PAGE: &str = r#"<html><body><table>
  <tr>
    <td><a href="/detail/zm-1"></a><p>hidden hint</p>Zero Title</td>
    <td>6 GB</td>
  </tr>
</table></body></html>"#;

/// 详情页把完整 magnet 放在隐藏输入框的 value 里，并带了一堆杂项参数。
fn detail_page() -> String {
    format!(
        r#"<html><body>
  <input id="input-magnet" value="{}&xl=123&tr=udp%3A%2F%2Ft.example%3A80" />
</body></html>"#,
        magnet(HEX)
    )
}

fn service_with(http: Arc<FakeHttp>) -> ZeroMagnet {
    ZeroMagnet::new(http, DefaultSiteEndpoints.endpoint_for(TorrentSrc::ZeroMagnet))
}

fn server() -> Arc<FakeHttp> {
    FakeHttp::new()
        .route(&format!("{ZERO_URL}/search"), SEARCH_PAGE)
        .route(&format!("{ZERO_URL}/detail"), &detail_page())
}

fn parse(body: &str) -> Vec<TorrentInfo> {
    service_with(FakeHttp::not_found()).parse_search_response(body)
}

#[test]
fn extracts_title_size_and_detail_url_from_a_cell_pair() {
    let result = parse(SEARCH_PAGE);

    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.src, TorrentSrc::ZeroMagnet.code());
    assert_eq!(item.title.as_deref(), Some("Zero Title"));
    assert_eq!(item.size.as_deref(), Some("6 GB"));
    assert_eq!(item.detail_url, Some(format!("{ZERO_URL}/detail/zm-1")));
}

#[test]
fn drops_the_hint_paragraph_that_sits_inside_the_title_cell() {
    let result = parse(SEARCH_PAGE);

    assert_eq!(result[0].title.as_deref(), Some("Zero Title"));
}

#[test]
fn walks_cells_two_at_a_time() {
    let result = parse(
        r#"<html><body><table>
  <tr><td><a href="/detail/a"></a>First</td><td>1 GB</td></tr>
  <tr><td><a href="/detail/b"></a>Second</td><td>2 GB</td></tr>
</table></body></html>"#,
    );

    assert_eq!(result.len(), 2);
    let titles: Vec<_> = result.iter().map(|r| r.title.as_deref()).collect();
    let sizes: Vec<_> = result.iter().map(|r| r.size.as_deref()).collect();
    assert_eq!(titles, [Some("First"), Some("Second")]);
    assert_eq!(sizes, [Some("1 GB"), Some("2 GB")]);
}

#[test]
fn skips_a_trailing_cell_that_has_no_pair() {
    let result = parse(
        r#"<html><body><table>
  <tr><td><a href="/detail/a"></a>Only</td><td>1 GB</td></tr>
  <tr><td>dangling</td></tr>
</table></body></html>"#,
    );

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("Only"));
}

#[test]
fn skips_cells_without_a_detail_link() {
    let result = parse(
        r#"<html><body><table>
  <tr><td>no link here</td><td>1 GB</td></tr>
</table></body></html>"#,
    );

    assert!(result.is_empty());
}

#[test]
fn empty_page_yields_no_rows() {
    assert!(parse("").is_empty());
    assert!(parse("<html><body></body></html>").is_empty());
}

#[tokio::test]
async fn rebuilds_a_minimal_magnet_from_the_hidden_input() {
    // 站点给的链接带一堆杂项参数，这里只留哈希后重拼
    let service = service_with(server());

    assert_eq!(service.find_magnet_url(&format!("{ZERO_URL}/detail/zm-1")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn detail_page_falls_back_to_an_anchor_when_the_input_is_gone() {
    let detail = format!(r#"<html><body><a href="{}">Magnet</a></body></html>"#, magnet(HEX));
    let service = service_with(FakeHttp::serving(&detail));

    assert_eq!(service.find_magnet_url(&format!("{ZERO_URL}/detail/x")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn an_unusable_hash_on_the_detail_page_yields_empty() {
    let service = service_with(FakeHttp::serving(
        r#"<html><body><input id="input-magnet" value="magnet:?xt=urn:btih:SHORT" /></body></html>"#,
    ));

    assert_eq!(service.find_magnet_url(&format!("{ZERO_URL}/detail/x")).await, None);
}

#[tokio::test]
async fn detail_page_without_a_magnet_yields_empty() {
    let service = service_with(FakeHttp::serving("<html><body><p>nothing</p></body></html>"));

    assert_eq!(service.find_magnet_url(&format!("{ZERO_URL}/detail/x")).await, None);
}

#[tokio::test]
async fn search_url_carries_the_term() {
    let http = server();
    let service = service_with(http.clone());

    service.search("女", 1).await.unwrap();

    assert_eq!(http.last_request().unwrap().url, format!("{ZERO_URL}/search?q=%E5%A5%B3"));
}

#[tokio::test]
async fn a_custom_endpoint_redirects_every_request() {
    let mirror = FakeHttp::serving(SEARCH_PAGE);
    let mirrored = ZeroMagnet::new(mirror.clone(), endpoint("https://zero.example"));

    mirrored.search("x", 1).await.unwrap();

    assert_eq!(mirror.last_request().unwrap().url, "https://zero.example/search?q=x");
}
