use std::sync::Arc;

use torrent_spider::site::parsers::btsow::BtSow;
use torrent_spider::testing::*;
use torrent_spider::{DefaultSiteEndpoints, SearchService, SiteEndpointProvider, TorrentSrc};

const BTSOW_URL: &str = "https://btsow.live";
const BTSOW_SEARCH: &str = "https://btsow.live/bts/data/api/search";

/// U+200B 零宽空格。用码点写出来：字面量在编辑器里不可见，很容易被误删或误加。
const ZWSP: char = '\u{200B}';

fn json_page() -> String {
    format!(
        r#"{{
  "code": 200,
  "data": [
    {{
      "name": "BtSow Title",
      "hash": "{HEX}",
      "size": 2097152,
      "lastUpdateTime": "2026-03-15"
    }}
  ]
}}"#
    )
}

fn detail_page() -> String {
    format!(
        r#"<html><body>
  <a id="magnetOpen" href="{}">Magnet</a>
</body></html>"#,
        magnet(HEX)
    )
}

fn service_with(http: Arc<FakeHttp>) -> BtSow {
    BtSow::new(http, DefaultSiteEndpoints.endpoint_for(TorrentSrc::BtSow))
}

fn server() -> Arc<FakeHttp> {
    FakeHttp::new().route(BTSOW_SEARCH, &json_page()).route(&format!("{BTSOW_URL}/detail"), &detail_page())
}

fn parse(body: &str) -> Vec<torrent_spider::TorrentInfo> {
    service_with(FakeHttp::not_found()).parse_search_response(body)
}

#[test]
fn decodes_the_json_payload_into_torrent_rows() {
    let result = parse(&json_page());

    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.title.as_deref(), Some("BtSow Title"));
    assert_eq!(item.hash.as_deref(), Some(HEX));
    assert_eq!(item.magnet_url, Some(format!("magnet:?xt=urn:btih:{HEX}")));
    assert_eq!(item.torrent_url, Some(torrent_url(HEX)));
    assert_eq!(item.date.as_deref(), Some("2026-03-15"));
    assert_eq!(item.size.as_deref(), Some("2.0M"));
    assert_eq!(item.src, TorrentSrc::BtSow.code());
}

#[test]
fn accepts_a_numeric_timestamp_as_seconds() {
    let result = parse(&format!(
        r#"{{"code":200,"data":[{{"hash":"{HEX}","name":"x","size":336741734,"lastUpdateTime":1752964611}}]}}"#
    ));

    assert_eq!(result[0].date.as_deref(), Some("2025-07-19"));
    assert_eq!(result[0].size.as_deref(), Some("321.1M"));
}

#[test]
fn accepts_a_numeric_timestamp_as_milliseconds() {
    let result =
        parse(&format!(r#"{{"data":[{{"hash":"{HEX}","name":"x","size":1,"lastUpdateTime":1752964611000}}]}}"#));

    assert_eq!(result[0].date.as_deref(), Some("2025-07-19"));
}

#[test]
fn strips_html_tags_from_the_name() {
    let result = parse(&format!(
        r#"{{"data":[{{"hash":"{HEX}","name":"<em>BtSow</em> Title","size":1,"lastUpdateTime":"2026-03-15"}}]}}"#
    ));

    assert_eq!(result[0].title.as_deref(), Some("BtSow Title"));
}

#[test]
fn highlight_tags_do_not_introduce_a_space_mid_title() {
    // 站点把文件后缀单独包成 <em>：标题里本来没有空格，不能补成「WEB-DL. mkv」
    let result = parse(&format!(
        r#"{{"data":[{{"hash":"{HEX}","name":"Mushoku Tensei S03E14 1080p WEB-DL.<em>mkv</em>","size":1,"lastUpdateTime":"2026-03-15"}}]}}"#
    ));

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("Mushoku Tensei S03E14 1080p WEB-DL.mkv"));
}

#[test]
fn keeps_the_row_but_drops_links_when_the_hash_is_unusable() {
    // 丢掉整行会让用户连「搜到了但拿不到链接」都看不到，反而更难排查
    let result = parse(r#"{"data":[{"hash":"abcdef123456","name":"Title","size":1,"lastUpdateTime":"2026-03-15"}]}"#);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("Title"));
    assert_eq!(result[0].hash, None);
    assert_eq!(result[0].magnet_url, None);
    assert_eq!(result[0].torrent_url, None);
}

#[test]
fn strips_the_zero_width_characters_the_site_hides_inside_names() {
    // 站点往标题中段插零宽空格反爬；不清掉的话用户复制标题去搜索就是错的
    let hidden = format!("the.random.driving.s{ZWSP}01{ZWSP}e{ZWSP}02.mkv");
    let result =
        parse(&format!(r#"{{"data":[{{"hash":"{HEX}","name":"{hidden}","size":1,"lastUpdateTime":"2026-03-15"}}]}}"#));

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("the.random.driving.s01e02.mkv"));
}

#[test]
fn normalizes_a_lower_case_hash_from_the_api() {
    let result =
        parse(&format!(r#"{{"data":[{{"hash":"{HEX_LOWER}","name":"x","size":1,"lastUpdateTime":"2026-03-15"}}]}}"#));

    assert_eq!(result[0].hash.as_deref(), Some(HEX));
}

#[test]
fn skips_entries_without_a_name() {
    let result = parse(&format!(
        r#"{{"data":[{{"hash":"{HEX}","name":null,"size":1}},{{"hash":"{OTHER_HEX}","name":"ok","size":1}}]}}"#
    ));

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("ok"));
}

#[test]
fn malformed_json_yields_an_empty_list_instead_of_throwing() {
    assert!(parse("not json at all").is_empty());
    assert!(parse("").is_empty());
    assert!(parse("{}").is_empty());
    assert!(parse(r#"{"data":null}"#).is_empty());
}

#[tokio::test]
async fn reads_the_magnet_from_the_detail_page() {
    let service = service_with(server());

    assert_eq!(service.find_magnet_url(&format!("{BTSOW_URL}/detail/abc")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn detail_page_without_a_usable_magnet_yields_empty() {
    let bare = service_with(FakeHttp::serving("<html><body><p>nothing here</p></body></html>"));

    assert_eq!(bare.find_magnet_url(&format!("{BTSOW_URL}/detail/abc")).await, None);
}

#[tokio::test]
async fn detail_page_falls_back_to_scanning_for_a_magnet_href() {
    let renamed = format!(r#"<html><body><a href="{}">Magnet</a></body></html>"#, magnet(HEX));
    let service = service_with(FakeHttp::serving(&renamed));

    assert_eq!(service.find_magnet_url(&format!("{BTSOW_URL}/detail/abc")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn search_posts_the_term_and_page_as_a_bare_array() {
    let http = server();
    let service = service_with(http.clone());

    service.search("movie", 3).await.unwrap();

    let request = http.last_request().unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.url, BTSOW_SEARCH);
    assert_eq!(request.body, r#"[{"search":"movie"},50,3]"#);
    assert_eq!(request.header("Content-Type"), Some("text/plain; charset=UTF-8"));
}

#[tokio::test]
async fn a_custom_endpoint_redirects_every_request() {
    let mirror = FakeHttp::new().route("https://btsow.example/bts/data/api/search", &json_page());
    let mirrored = BtSow::new(mirror.clone(), endpoint("https://btsow.example"));

    mirrored.search("x", 1).await.unwrap();

    assert_eq!(mirror.last_request().unwrap().url, "https://btsow.example/bts/data/api/search");
}
