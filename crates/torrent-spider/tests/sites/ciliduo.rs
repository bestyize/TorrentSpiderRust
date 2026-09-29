use std::sync::Arc;

use base64::Engine;
use torrent_spider::site::parsers::ciliduo::{Ciliduo, engine_url, hash_of_doc_url};
use torrent_spider::site::{CILIDUO_ENGINE, CILIDUO_REFERER};
use torrent_spider::testing::*;
use torrent_spider::{DefaultSiteEndpoints, SearchService, SiteEndpoint, SiteEndpointProvider, TorrentSrc};

const CILIDUO_URL: &str = "https://qu.cilido.top";

/// 接口域名与前端域名是两个站，这是壳页面脚本里解出来的那个。
const ENGINE_URL: &str = "https://doc2.htmcdn.com:39988";

const HASH_LOWER: &str = "cf66f9a06d70defa18a6462f321006c69f16eb74";
const HASH_UPPER: &str = "CF66F9A06D70DEFA18A6462F321006C69F16EB74";

/// 站点实际写在脚本里的字符串。
const ENGINE_BASE64: &str = "aHR0cHM6Ly9kb2MyLmh0bWNkbi5jb206Mzk5ODg=";

/// 站点标题那段 atob，解出来是 URL 编码的中文而不是 URL。
const TITLE_BASE64: &str = "JUU3JUEzJTgxJUU1JThBJTlCJUU1JUE0JTlB";

/// 站点给的是小写，项目内部一律规范成大写，所以重拼出来的 magnet 是大写。
fn derived_magnet() -> String {
    magnet(HASH_UPPER)
}

/// 站点原样给出的 magnet：光秃秃的 `magnet:?xt=urn:btih:<小写 hash>`。
/// 只有条目没给链接、需要从详情页地址反推时才会用内部的大写 hash 重拼；
/// 两条路径大小写不同，测试里必须分清，否则会误以为解析层做了归一化。
fn raw_magnet() -> String {
    magnet(HASH_LOWER)
}

/// 取自站点实际返回的 markup，只去掉了同一条目里的其它兄弟节点。
fn search_page() -> String {
    format!(
        r#"<html><body><div class="tbox"><div class="ssbox">
  <div class="title"><h3><span>[视频]</span><a target="_blank" href="/doc/{HASH_LOWER}">AML <em>Test</em> CD +</a></h3></div>
  <div class="sbar">
    <span><a target="_blank" href="magnet:?xt=urn:btih:{HASH_LOWER}">[磁力链接]</a></span>
    <span>添加时间：<b>2020-05-24</b></span>
    <span>大小：<b class="cpill yellow-pill">255.80 MB</b></span>
    <span>文件数：<b>23</b></span>
    <span>热度：<b>77</b></span>
  </div>
</div></div></body></html>"#
    )
}

fn shell_page() -> String {
    format!(
        r#"<!DOCTYPE html><html lang="zh-CN"><head><title>BT搜索</title></head><body>
<iframe style="width:100%" frameborder="0" onload="loaded()"></iframe>
<script>
  var getProxy = function(){{    var proxy = atob('{ENGINE_BASE64}');    return proxy;}};
  $(function(){{ var params = location.pathname + "?host=" + location.host + "&v=1";
  var proxy = getProxy(); $("iframe").attr("src", proxy + params); }});
</script></body></html>"#
    )
}

fn service_with(http: Arc<FakeHttp>) -> Ciliduo {
    Ciliduo::new(http, DefaultSiteEndpoints.endpoint_for(TorrentSrc::Ciliduo))
}

fn service() -> Ciliduo {
    service_with(FakeHttp::serving(&search_page()))
}

fn entry(title_html: &str, bar_html: &str) -> String {
    format!(
        r#"<html><body>
  <div class="ssbox">
    <div class="title"><h3>{title_html}</h3></div>
    <div class="sbar">{bar_html}</div>
  </div>
</body></html>"#
    )
}

// ---- 解析 ----

#[test]
fn extracts_every_field_of_an_entry() {
    let result = service().parse_search_response(&search_page());

    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.src, TorrentSrc::Ciliduo.code());
    assert_eq!(item.title.as_deref(), Some("AML Test CD +"));
    assert_eq!(item.hash.as_deref(), Some(HASH_UPPER));
    assert_eq!(item.magnet_url, Some(raw_magnet()));
    assert_eq!(item.detail_url, Some(format!("{CILIDUO_URL}/doc/{HASH_LOWER}")));
    assert_eq!(item.date.as_deref(), Some("2020-05-24"));
    assert_eq!(item.size.as_deref(), Some("255.80 MB"));
    assert_eq!(item.sender_count.as_deref(), Some("23"));
    assert_eq!(item.download_count.as_deref(), Some("77"));
}

#[test]
fn reads_labelled_fields_by_their_chinese_prefix_not_by_position() {
    // 站点插了一个新字段，全部列的位置都会后移；按前缀取值不受影响
    let shifted = search_page().replace(
        "<span>添加时间：<b>2020-05-24</b></span>",
        "<span>收录：<b>yes</b></span><span>添加时间：<b>2020-05-24</b></span>",
    );

    let result = service().parse_search_response(&shifted);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].date.as_deref(), Some("2020-05-24"));
    assert_eq!(result[0].size.as_deref(), Some("255.80 MB"));
    assert_eq!(result[0].sender_count.as_deref(), Some("23"));
}

#[test]
fn the_file_count_is_not_mistaken_for_the_size() {
    // 两个都是 <b>，只有按前缀取才不会串
    let result = service().parse_search_response(&search_page());

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].size.as_deref(), Some("255.80 MB"));
    assert_eq!(result[0].sender_count.as_deref(), Some("23"));
}

#[test]
fn title_keeps_text_that_lives_outside_the_anchor() {
    // 标题里带 <em> 高亮，不该被丢掉
    let page = entry(
        &format!(r#"<span>[视频]</span><a href="/doc/{HASH_LOWER}">AML <em>Test</em> CD +</a>"#),
        "<span>大小：<b>255.80 MB</b></span>",
    );

    let result = service().parse_search_response(&page);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("AML Test CD +"));
}

#[test]
fn adjacent_highlight_elements_do_not_gain_spaces_in_the_title() {
    // 站点把搜索词逐字拆进相邻的 <em>（真实 markup，来自线上「微微一笑很倾城」的搜索结果）。
    // 在子元素之间补空格会拼出「微微一笑 很 倾城」，和站点上显示的标题不一致。
    let page = entry(
        &format!(
            r#"<span>[视频]</span><a target="_blank" href="/doc/{HASH_LOWER}"><em>微微一笑</em><em>很</em><em>倾城</em></a>"#
        ),
        "<span>大小：<b>29.53 GB</b></span>",
    );

    let result = service().parse_search_response(&page);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("微微一笑很倾城"));
}

#[test]
fn falls_back_to_the_detail_url_when_the_magnet_anchor_is_missing() {
    let page = entry(&format!(r#"<a href="/doc/{HASH_LOWER}">From doc url</a>"#), "<span>大小：<b>1 GB</b></span>");

    let result = service().parse_search_response(&page);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].hash.as_deref(), Some(HASH_UPPER));
    assert_eq!(result[0].magnet_url, Some(derived_magnet()));
}

#[test]
fn a_detail_url_without_a_usable_hash_yields_no_magnet_but_keeps_the_row() {
    let page = entry(r#"<a href="/doc/not-a-hash">No hash</a>"#, "<span>大小：<b>1 GB</b></span>");

    let result = service().parse_search_response(&page);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("No hash"));
    assert_eq!(result[0].magnet_url, None);
    assert_eq!(result[0].hash, None);
}

#[test]
fn an_invalid_magnet_does_not_leak_through() {
    let page = entry(
        &format!(r#"<a href="/doc/{HASH_LOWER}">Bad magnet</a>"#),
        r#"<span><a href="magnet:?xt=urn:btih:SHORT">[磁力链接]</a></span>"#,
    );

    let result = service().parse_search_response(&page);

    // 条目自己的 magnet 不合法时应当退回详情页地址，而不是把残缺串透传出去
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].magnet_url, Some(derived_magnet()));
}

#[test]
fn falls_back_to_a_substring_match_when_the_container_class_changes() {
    let renamed = search_page().replace(r#"class="ssbox""#, r#"class="ssbox-v2""#);

    let result = service().parse_search_response(&renamed);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title.as_deref(), Some("AML Test CD +"));
}

#[test]
fn finds_size_and_date_by_content_when_the_labels_change_wording() {
    let relabelled = search_page().replace("添加时间：", "发布日期：").replace("大小：", "体积：");

    let result = service().parse_search_response(&relabelled);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].date.as_deref(), Some("2020-05-24"));
    assert_eq!(result[0].size.as_deref(), Some("255.80 MB"));
}

#[test]
fn empty_page_yields_no_entries_instead_of_throwing() {
    assert!(service().parse_search_response("").is_empty());
    assert!(service().parse_search_response("<html><body></body></html>").is_empty());
}

#[test]
fn entries_without_a_title_anchor_are_skipped() {
    let page = r#"<html><body>
  <div class="ssbox"><div class="sbar"><span>大小：<b>1 GB</b></span></div></div>
</body></html>"#;

    assert!(service().parse_search_response(page).is_empty());
}

// ---- 请求 ----

#[tokio::test]
async fn search_requests_the_engine_host_with_the_referer_the_site_requires() {
    let http = FakeHttp::serving(&search_page());
    service_with(http.clone()).search("test", 1).await.unwrap();

    assert_eq!(http.requested_urls(), vec![format!("{ENGINE_URL}/search?word=test&page=1")]);
}

#[tokio::test]
async fn the_referer_header_is_sent() {
    // 缺 Referer 时接口返回 104 空响应，光看状态码看不出原因
    let http = FakeHttp::serving(&search_page());
    service_with(http.clone()).search("test", 1).await.unwrap();

    let request = http.last_request().unwrap();
    assert_eq!(request.header("Referer"), Some("https://qu.cilido.top/"));
    assert!(!request.header("User-Agent").unwrap_or_default().trim().is_empty());
}

#[tokio::test]
async fn page_number_is_passed_through() {
    let http = FakeHttp::serving(&search_page());
    service_with(http.clone()).search("女", 3).await.unwrap();

    assert_eq!(http.last_request().unwrap().url, format!("{ENGINE_URL}/search?word=%E5%A5%B3&page=3"));
}

#[tokio::test]
async fn magnet_is_derived_from_the_hash_in_the_detail_url_without_a_network_call() {
    let detail = FakeHttp::not_found();
    let service = service_with(detail.clone());

    // 详情页根本没被请求，说明磁力是从地址本身推出来的
    assert_eq!(service.find_magnet_url(&format!("{CILIDUO_URL}/doc/{HASH_LOWER}")).await, Some(derived_magnet()));
    assert!(detail.requested_urls().is_empty());
}

#[tokio::test]
async fn detail_url_without_a_usable_hash_yields_no_magnet() {
    let service = service();

    assert_eq!(service.find_magnet_url(&format!("{CILIDUO_URL}/doc/not-a-hash")).await, None);
    assert_eq!(service.find_magnet_url(&format!("{CILIDUO_URL}/")).await, None);
    assert_eq!(service.find_magnet_url("").await, None);
}

#[tokio::test]
async fn hashes_in_a_base32_detail_url_are_still_recognised() {
    let base32 = torrent_spider::magnet::hash::hex_to_base32(HEX_LOWER).unwrap();

    assert_eq!(service().find_magnet_url(&format!("{CILIDUO_URL}/doc/{base32}")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn a_custom_endpoint_redirects_every_request() {
    let mirror = FakeHttp::serving(&search_page());
    let mirrored = Ciliduo::new(mirror.clone(), SiteEndpoint::new("https://duo.example"));

    mirrored.search("x", 1).await.unwrap();

    assert_eq!(mirror.last_request().unwrap().url, "https://duo.example/search?word=x&page=1");
}

#[test]
fn doc_url_parsing_tolerates_query_strings_and_trailing_slashes() {
    for url in [
        format!("https://a/doc/{HASH_LOWER}"),
        format!("https://a/doc/{HASH_LOWER}?x=1"),
        format!("https://a/doc/{HASH_LOWER}#frag"),
        format!("https://a/doc/{HASH_LOWER}/"),
    ] {
        assert_eq!(hash_of_doc_url(&url).as_deref(), Some(HASH_UPPER), "{url}");
    }
    assert_eq!(hash_of_doc_url("https://a/doc/"), None);
}

// ---- 壳页面：接口域名解析 ----
//
// 磁力多给用户的域名本身不含结果，脚本里用 `atob` 指到真正的接口域名。
// 这里用站点实际返回的片段作为 fixture，而不是自己编一个——编的 fixture
// 只验证了「我以为的格式」，验证不了站点真实发的是什么。

fn base64_of(value: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(value)
}

#[test]
fn shell_fixtures_are_the_expected_base64() {
    // Kotlin 的 decodeBase64 在 Rust 里由 base64 crate 承担，没有公开函数可测；
    // 这里只确认 fixture 本身是站点那两段字符串，其余用例经由 engine_url 覆盖解码
    let decode = |s: &str| String::from_utf8(base64::engine::general_purpose::STANDARD.decode(s).unwrap()).unwrap();
    assert_eq!(decode(ENGINE_BASE64), ENGINE_URL);
    // 站点标题也是 atob 出来的，解完是 URL 编码后的中文
    assert_eq!(decode(TITLE_BASE64), "%E7%A3%81%E5%8A%9B%E5%A4%9A");
}

#[test]
fn extracts_the_engine_url_from_a_real_shell_fragment() {
    assert_eq!(engine_url(&shell_page()).as_deref(), Some(ENGINE_URL));
}

#[test]
fn extracts_the_engine_url_whatever_quoting_the_script_uses() {
    for script in [
        format!(r#"var proxy = atob("{ENGINE_BASE64}");"#),
        format!("var proxy = atob('{ENGINE_BASE64}');"),
        format!("atob(  '{ENGINE_BASE64}'  )"),
    ] {
        assert_eq!(engine_url(&script).as_deref(), Some(ENGINE_URL), "{script}");
    }
}

#[test]
fn the_trailing_slash_is_dropped() {
    // 重拼路径靠字符串相加，留着结尾斜杠会拼出 //search
    let with_slash = format!("atob('{}')", base64_of(&format!("{ENGINE_URL}/")));

    assert_eq!(engine_url(&with_slash).as_deref(), Some(ENGINE_URL));
}

#[test]
fn ignores_base64_strings_that_are_not_urls() {
    // 壳页面里还有别的 atob（标题、cookie 之类），不能随便挑一个
    let mixed = format!("var title = atob('{TITLE_BASE64}');\nvar proxy = atob('{ENGINE_BASE64}');");

    assert_eq!(engine_url(&mixed).as_deref(), Some(ENGINE_URL));
}

#[test]
fn a_page_without_a_proxy_script_yields_none_instead_of_throwing() {
    assert_eq!(engine_url(""), None);
    assert_eq!(engine_url("<html><body>nothing here</body></html>"), None);
    assert_eq!(engine_url("atob('short')"), None);
}

#[test]
fn a_shell_whose_payload_is_not_base64_is_rejected() {
    assert_eq!(engine_url("atob('!!!!not-base64!!!!')"), None);
    // 字母表之外的字符：对应 Kotlin decodeBase64 的拒绝用例
    assert_eq!(engine_url("atob('not base64!!not base64!!')"), None);
    assert_eq!(engine_url("atob('@@@@@@@@@@@@@@@@')"), None);
}

#[test]
fn the_shipped_endpoint_matches_what_the_shell_advertises() {
    // 配置里的接口域名和壳页面解出来的必须一致，否则改域名时只改一处就会静默失效
    assert_eq!(CILIDUO_ENGINE, ENGINE_URL);
    assert_eq!(engine_url(&shell_page()).as_deref(), Some(ENGINE_URL));
    assert_eq!(DefaultSiteEndpoints.endpoint_for(TorrentSrc::Ciliduo).request_base(), CILIDUO_ENGINE);
}

#[test]
fn the_shipped_referer_matches_the_front_end_host() {
    let endpoint = DefaultSiteEndpoints.endpoint_for(TorrentSrc::Ciliduo);

    assert_eq!(endpoint.referer_header(), Some(CILIDUO_REFERER));
    assert_eq!(endpoint.referer_header().map(|r| r.trim_end_matches('/')), Some(endpoint.host()));
}
