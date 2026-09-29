use torrent_spider::site::parsers::PirateBay;
use torrent_spider::testing::*;
use torrent_spider::{DefaultSiteEndpoints, SearchService, SiteEndpoint, SiteEndpointProvider, TorrentSrc};

const TPB_URL: &str = "https://thepiratebay.party";

fn search_page() -> String {
    format!(
        r#"<html><body><table>
  <tr>
    <td>index</td>
    <td><a href="{TPB_URL}/detail/pb">Pirate Title</a></td>
    <td>2026-03-15</td>
    <td><a href="{}">Magnet</a></td>
    <td>3 GB</td>
  </tr>
</table></body></html>"#,
        magnet(HEX)
    )
}

fn default_endpoint() -> SiteEndpoint {
    DefaultSiteEndpoints.endpoint_for(TorrentSrc::PirateBay)
}

fn service() -> PirateBay {
    PirateBay::new(FakeHttp::new().route(&format!("{TPB_URL}/"), &search_page()), default_endpoint())
}

fn some(s: &str) -> Option<String> {
    Some(s.to_owned())
}

#[test]
fn extracts_every_column_of_a_row() {
    let result = service().parse_search_response(&search_page());

    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.src, TorrentSrc::PirateBay.code());
    assert_eq!(item.title, some("Pirate Title"));
    assert_eq!(item.hash, some(HEX));
    assert_eq!(item.magnet_url, Some(magnet(HEX)));
    assert_eq!(item.torrent_url, Some(torrent_url(HEX)));
    assert_eq!(item.detail_url, Some(format!("{TPB_URL}/detail/pb")));
    assert_eq!(item.date, some("2026-03-15"));
    assert_eq!(item.size, some("3 GB"));
}

#[test]
fn skips_rows_that_do_not_have_all_five_columns() {
    let body = format!(
        r#"<html><body><table>
  <tr><td>only</td><td>two</td></tr>
  <tr>
    <td>i</td><td><a href="{TPB_URL}/detail/ok">Good</a></td><td>2026-03-15</td>
    <td><a href="{}">M</a></td><td>1 GB</td>
  </tr>
</table></body></html>"#,
        magnet(HEX)
    );

    let result = service().parse_search_response(&body);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title, some("Good"));
}

#[test]
fn keeps_the_row_when_the_magnet_cell_has_no_usable_hash() {
    let body = format!(
        r#"<html><body><table>
  <tr>
    <td>i</td><td><a href="{TPB_URL}/detail/x">Title</a></td><td>2026-03-15</td>
    <td><a href="magnet:?xt=urn:btih:FACE1234">M</a></td><td>1 GB</td>
  </tr>
</table></body></html>"#
    );

    let result = service().parse_search_response(&body);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title, some("Title"));
    assert_eq!(result[0].hash, None);
    assert_eq!(result[0].torrent_url, None);
}

#[test]
fn empty_page_yields_no_rows() {
    assert!(service().parse_search_response("").is_empty());
    assert!(service().parse_search_response("<html><body></body></html>").is_empty());
}

#[tokio::test]
async fn reads_the_magnet_from_the_detail_page_download_block() {
    let detail = FakeHttp::serving(&format!(
        r#"<html><body><div class="download"><a href="{}">M</a></div></body></html>"#,
        magnet(HEX)
    ));
    let service = PirateBay::new(detail, default_endpoint());

    assert_eq!(service.find_magnet_url(&format!("{TPB_URL}/detail/pb")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn detail_page_falls_back_to_scanning_for_a_magnet_href() {
    let detail =
        FakeHttp::serving(&format!(r#"<html><body><section><a href="{}">M</a></section></body></html>"#, magnet(HEX)));
    let service = PirateBay::new(detail, default_endpoint());

    assert_eq!(service.find_magnet_url(&format!("{TPB_URL}/detail/pb")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn detail_page_without_a_magnet_yields_none() {
    let detail = FakeHttp::serving("<html><body><p>nothing</p></body></html>");
    let service = PirateBay::new(detail, default_endpoint());

    assert_eq!(service.find_magnet_url(&format!("{TPB_URL}/detail/pb")).await, None);
}

#[tokio::test]
async fn search_url_carries_the_term_and_page() {
    let server = FakeHttp::new().route(&format!("{TPB_URL}/"), &search_page());
    let service = PirateBay::new(server.clone(), default_endpoint());

    service.search("女", 2).await.unwrap();

    assert_eq!(server.last_request().unwrap().url, format!("{TPB_URL}/search/%E5%A5%B3/2/99/0"));
}

#[tokio::test]
async fn a_custom_endpoint_redirects_every_request() {
    let mirror = FakeHttp::serving(&search_page());
    let mirrored = PirateBay::new(mirror.clone(), SiteEndpoint::new("https://tpb.example"));

    mirrored.search("x", 1).await.unwrap();

    assert_eq!(mirror.last_request().unwrap().url, "https://tpb.example/search/x/1/99/0");
}
