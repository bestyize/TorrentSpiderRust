use torrent_spider::site::parsers::AcgRip;
use torrent_spider::testing::*;
use torrent_spider::{DefaultSiteEndpoints, SearchService, SiteEndpoint, SiteEndpointProvider, TorrentSrc};

const ACG_URL: &str = "https://acg.rip";

const SEARCH_PAGE: &str = r#"<html><body>
<table class="table table-hover table-condensed post-index">
  <tr>
    <td class="date hidden-xs hidden-sm">
      <div><a href="/user/1">Uploader</a></div>
      <div><time datetime="1773568224">约 2 小时</time></div>
    </td>
    <td class="title">
      <span class="label label-team"><a href="/team/113">Team</a></span>
      <span class="title"><a href="/t/349761">Acg Title</a></span>
    </td>
    <td class="action"><a href="/t/349761.torrent">Download</a></td>
    <td class="size">700 MB</td>
  </tr>
</table>
</body></html>"#;

fn service() -> AcgRip {
    AcgRip::new(FakeHttp::serving(SEARCH_PAGE), DefaultSiteEndpoints.endpoint_for(TorrentSrc::AcgRip))
}

fn some(s: &str) -> Option<String> {
    Some(s.to_owned())
}

#[test]
fn extracts_every_field_of_a_row() {
    let result = service().parse_search_response(SEARCH_PAGE);

    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.src, TorrentSrc::AcgRip.code());
    assert_eq!(item.title, some("Acg Title"));
    assert_eq!(item.date, some("约 2 小时"));
    assert_eq!(item.size, some("700 MB"));
    assert_eq!(item.detail_url, Some(format!("{ACG_URL}/t/349761")));
    // acg.rip 的条目直接给 .torrent，磁力和种子是同一个地址
    assert_eq!(item.magnet_url, Some(format!("{ACG_URL}/t/349761.torrent")));
    assert_eq!(item.torrent_url, Some(format!("{ACG_URL}/t/349761.torrent")));
}

#[test]
fn picks_the_detail_link_over_other_links_in_the_title_cell() {
    // 标题单元格里还有上传者、字幕组等链接，只有 /t/<id> 才是详情页
    let result = service().parse_search_response(SEARCH_PAGE);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].detail_url, Some(format!("{ACG_URL}/t/349761")));
}

#[test]
fn falls_back_to_the_first_table_when_the_class_name_changes() {
    let renamed =
        SEARCH_PAGE.replace(r#"class="table table-hover table-condensed post-index""#, r#"class="table table-hover""#);
    assert!(!renamed.contains("post-index"), "类名没被替换掉");

    let result = service().parse_search_response(&renamed);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title, some("Acg Title"));
}

#[test]
fn skips_rows_that_have_no_torrent_link() {
    let result = service().parse_search_response(
        r#"<html><body>
<table class="post-index">
  <tr><td class="title"><a href="/t/1">No torrent here</a></td><td class="size">1 GB</td></tr>
</table>
</body></html>"#,
    );

    assert!(result.is_empty());
}

#[test]
fn empty_page_yields_no_rows_instead_of_throwing() {
    assert!(service().parse_search_response("").is_empty());
    assert!(service().parse_search_response("<html><body></body></html>").is_empty());
}

#[test]
fn first_page_uses_the_short_route() {
    assert_eq!(service().search_url("女", 1), format!("{ACG_URL}/1?term=%E5%A5%B3"));
}

#[test]
fn later_pages_use_the_paged_route() {
    assert_eq!(service().search_url("女", 2), format!("{ACG_URL}/1/page/2?term=%E5%A5%B3"));
    assert_eq!(service().search_url("女", 3), format!("{ACG_URL}/1/page/3?term=%E5%A5%B3"));
}

#[test]
fn page_number_below_one_is_clamped() {
    // 页码是 u32，Kotlin 里的负数页在这里不可能出现，只剩 0
    assert_eq!(service().search_url("x", 0), format!("{ACG_URL}/1?term=x"));
}

#[tokio::test]
async fn search_actually_requests_the_built_url() {
    let server = FakeHttp::serving(SEARCH_PAGE);
    let service = AcgRip::new(server.clone(), DefaultSiteEndpoints.endpoint_for(TorrentSrc::AcgRip));

    service.search("女", 2).await.unwrap();

    assert_eq!(server.last_request().unwrap().url, format!("{ACG_URL}/1/page/2?term=%E5%A5%B3"));
}

#[tokio::test]
async fn a_custom_endpoint_redirects_every_request() {
    let mirror = FakeHttp::serving(SEARCH_PAGE);
    let mirrored = AcgRip::new(mirror.clone(), SiteEndpoint::new("https://acg.example"));

    mirrored.search("x", 1).await.unwrap();

    assert_eq!(mirror.last_request().unwrap().url, "https://acg.example/1?term=x");
}
