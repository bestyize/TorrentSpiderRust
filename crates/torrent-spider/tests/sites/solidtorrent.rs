use torrent_spider::site::parsers::SolidTorrent;
use torrent_spider::testing::*;
use torrent_spider::{DefaultSiteEndpoints, SearchService, SiteEndpoint, SiteEndpointProvider, TorrentSrc};

const SOLID_URL: &str = "https://bitsearch.eu";

/// 下载数那一格；整格用常量插入，方便下面整格摘掉。
const BLUE_CELL: &str = r#"<span class="inline-flex items-center space-x-1 text-blue-600">
  <i class="fas fa-download"></i>
  <span class="font-medium">500</span>
  <span>downloads</span>
</span>"#;

fn search_page() -> String {
    let magnet = magnet(HEX);
    format!(
        r#"<html><body>
  <div class="space-y-4">
    <div class="bg-white rounded-lg shadow-sm border border-gray-200 p-6">
      <div class="flex-1 min-w-0">
        <h3 class="text-base sm:text-lg font-medium text-gray-900 line-clamp-2">
          <a href="/torrent/1">Solid Title</a>
        </h3>
        <div class="flex flex-wrap items-center gap-4 text-sm text-gray-600 mb-3">
          <span class="inline-flex items-center space-x-1">
            <i class="fas fa-video text-blue-500"></i>
            <span>Other/Image</span>
          </span>
          <span class="inline-flex items-center space-x-1">
            <i class="fas fa-download"></i>
            <span>10 GB</span>
          </span>
          <span class="inline-flex items-center space-x-1">
            <i class="fas fa-calendar"></i>
            <span>1 day ago</span>
          </span>
        </div>
        <div class="flex flex-wrap items-center gap-4 text-sm">
          <span class="inline-flex items-center space-x-1 text-green-600">
            <i class="fas fa-arrow-up"></i>
            <span class="font-medium">12</span>
            <span>seeders</span>
          </span>
          <span class="inline-flex items-center space-x-1 text-red-600">
            <i class="fas fa-arrow-down"></i>
            <span class="font-medium">4</span>
            <span>leechers</span>
          </span>
          {BLUE_CELL}
        </div>
      </div>
      <div class="hidden sm:flex flex-col space-y-2 ml-6">
        <a href="https://download/test.torrent">torrent</a>
        <a href="{magnet}">magnet</a>
      </div>
    </div>
  </div>
</body></html>"#
    )
}

/// 一条没有下载记录的条目：站点连蓝色那一格都不渲染（取自线上第 3 页）。
/// 按位置取值的照妖镜——少一格，后面所有的下标都会错位。
fn no_download_cell_page() -> String {
    let page = search_page();
    let stripped = page.replace(BLUE_CELL, "");
    assert_ne!(page, stripped, "蓝色那格没被摘掉");
    stripped
}

fn service() -> SolidTorrent {
    SolidTorrent::new(FakeHttp::serving(&search_page()), DefaultSiteEndpoints.endpoint_for(TorrentSrc::SolidTorrent))
}

fn some(s: &str) -> Option<String> {
    Some(s.to_owned())
}

#[test]
fn extracts_every_field_of_a_card() {
    let result = service().parse_search_response(&search_page());

    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.src, TorrentSrc::SolidTorrent.code());
    assert_eq!(item.title, some("Solid Title"));
    assert_eq!(item.hash, some(HEX));
    assert_eq!(item.magnet_url, Some(magnet(HEX)));
    assert_eq!(item.torrent_url, some("https://download/test.torrent"));
    assert_eq!(item.detail_url, Some(format!("{SOLID_URL}/torrent/1")));
    assert_eq!(item.size, some("10 GB"));
    assert_eq!(item.date, some("1 day ago"));
    assert_eq!(item.download_count, some("500"));
    assert_eq!(item.sender_count, some("12"));
    assert_eq!(item.leacher_count, some("4"));
}

#[test]
fn finds_the_magnet_even_when_the_two_links_are_in_the_other_order() {
    // 老实现靠 .hidden 容器里 href 的先后顺序区分种子和磁力，顺序一变就取错。
    // 用「换成标记再换回来」来对调，不依赖 fixture 的缩进
    let m = magnet(HEX);
    let reversed = search_page()
        .replace(&m, "__MAGNET__")
        .replace("https://download/test.torrent", &m)
        .replace("__MAGNET__", "https://download/test.torrent");

    // 先把前提钉死：替换必须真的发生，否则测试等于空跑
    assert!(
        reversed.find(&m).unwrap() < reversed.find("download/test.torrent").unwrap(),
        "fixture 里的两个链接没被调换顺序"
    );

    let result = service().parse_search_response(&reversed);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].hash, some(HEX));
    assert_eq!(result[0].torrent_url, some("https://download/test.torrent"));
}

#[test]
fn reads_each_metric_from_its_own_colour_rather_than_by_position() {
    // 没有下载记录的条目连蓝色那格都不渲染；按位置取值会整体错位
    let result = service().parse_search_response(&no_download_cell_page());

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].download_count, None);
    assert_eq!(result[0].sender_count, some("12"));
    assert_eq!(result[0].leacher_count, some("4"));
}

#[test]
fn the_same_colour_reused_on_an_icon_does_not_shadow_the_real_metric() {
    // 分类图标是 `fas fa-video text-blue-500`，和下载数同色但没有数字
    let result = service().parse_search_response(&search_page());

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].download_count, some("500"));
}

#[test]
fn a_same_coloured_badge_is_not_mistaken_for_a_metric() {
    // 标题旁的绿色 `Verified` 徽章也带 text-green-800 + font-medium，只是文本不是数字
    let anchor = r#"<a href="/torrent/1">Solid Title</a>"#;
    let with_badge = search_page().replace(
        anchor,
        r#"<a href="/torrent/1">Solid Title</a>
          <span class="inline-flex items-center px-2 py-1 rounded-full text-xs font-medium bg-green-100 text-green-800">
            <i class="fas fa-check-circle mr-1"></i> Verified
          </span>"#,
    );
    assert!(with_badge.contains("Verified"), "徽章没插进去");

    let result = service().parse_search_response(&with_badge);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].sender_count, some("12"));
    assert_eq!(result[0].leacher_count, some("4"));
    assert_eq!(result[0].download_count, some("500"));
}

#[test]
fn still_finds_metrics_when_the_colour_shade_changes() {
    // Tailwind 换主题时色阶号会变；换色相（blue 改 sky）则只能改选择器
    let recoloured = search_page()
        .replace("text-blue-600", "text-blue-500")
        .replace("text-green-600", "text-green-500")
        .replace("text-red-600", "text-red-500");

    let result = service().parse_search_response(&recoloured);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].download_count, some("500"));
    assert_eq!(result[0].sender_count, some("12"));
    assert_eq!(result[0].leacher_count, some("4"));
}

#[test]
fn falls_back_to_content_matching_for_the_size() {
    let body = format!(
        r#"<html><body>
  <div class="space-y-4">
    <div>
      <h3><a href="/torrent/9">Title</a></h3>
      <span>7.5 GB</span>
      <a href="{}">m</a>
    </div>
  </div>
</body></html>"#,
        magnet(HEX)
    );

    let result = service().parse_search_response(&body);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].size, some("7.5 GB"));
}

#[test]
fn a_card_without_a_heading_is_skipped() {
    let body = format!(
        r#"<html><body>
  <div class="space-y-4">
    <div><span>no heading here</span></div>
    <div><h3><a href="/torrent/2">Kept</a></h3><a href="{}">m</a></div>
  </div>
</body></html>"#,
        magnet(HEX)
    );

    let result = service().parse_search_response(&body);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title, some("Kept"));
}

#[test]
fn empty_page_yields_no_cards() {
    assert!(service().parse_search_response("").is_empty());
    assert!(service().parse_search_response("<html><body></body></html>").is_empty());
}

#[tokio::test]
async fn reads_the_magnet_from_the_detail_page() {
    let detail = FakeHttp::serving(&format!(
        r#"<html><body><div class="dl-links">
             <a href="https://download/x.torrent">t</a>
             <a href="{}">m</a>
           </div></body></html>"#,
        magnet(HEX)
    ));
    let service = SolidTorrent::new(detail, SiteEndpoint::new(SOLID_URL));

    assert_eq!(service.find_magnet_url(&format!("{SOLID_URL}/torrent/1")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn detail_page_falls_back_to_scanning_for_a_magnet_href() {
    let detail = FakeHttp::serving(&format!(r#"<html><body><a href="{}">m</a></body></html>"#, magnet(HEX)));
    let service = SolidTorrent::new(detail, SiteEndpoint::new(SOLID_URL));

    assert_eq!(service.find_magnet_url(&format!("{SOLID_URL}/torrent/1")).await, Some(magnet(HEX)));
}

#[tokio::test]
async fn search_url_carries_the_term_and_page() {
    let server = FakeHttp::serving(&search_page());
    let service = SolidTorrent::new(server.clone(), DefaultSiteEndpoints.endpoint_for(TorrentSrc::SolidTorrent));

    service.search("女", 2).await.unwrap();

    assert_eq!(server.last_request().unwrap().url, format!("{SOLID_URL}/search?q=%E5%A5%B3&page=2"));
}

#[tokio::test]
async fn a_custom_endpoint_redirects_every_request() {
    let mirror = FakeHttp::serving(&search_page());
    let mirrored = SolidTorrent::new(mirror.clone(), SiteEndpoint::new("https://solid.example"));

    mirrored.search("x", 1).await.unwrap();

    assert_eq!(mirror.last_request().unwrap().url, "https://solid.example/search?q=x&page=1");
}
