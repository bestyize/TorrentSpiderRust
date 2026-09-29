//! acg.rip：动画种子站，条目直接指向 `.torrent`。

use std::sync::Arc;

use async_trait::async_trait;
use scraper::ElementRef;

use super::SearchService;
use crate::error::SpiderError;
use crate::model::{TorrentInfo, opt};
use crate::net::HttpClient;
use crate::net::html;
use crate::site::SiteEndpoint;
use crate::site::TorrentSrc;

const ANIME_CATEGORY_PATH: &str = "/1";

pub struct AcgRip {
    http: Arc<dyn HttpClient>,
    endpoint: SiteEndpoint,
}

impl AcgRip {
    pub fn new(http: Arc<dyn HttpClient>, endpoint: SiteEndpoint) -> Self {
        Self { http, endpoint }
    }

    pub fn search_url(&self, keyword: &str, page: u32) -> String {
        let term = crate::net::encoding::url_encode(keyword);
        let path = match page.max(1) {
            1 => format!("{ANIME_CATEGORY_PATH}?term={term}"),
            n => format!("{ANIME_CATEGORY_PATH}/page/{n}?term={term}"),
        };
        self.endpoint.url(&path)
    }

    pub fn parse_search_response(&self, body: &str) -> Vec<TorrentInfo> {
        let document = html::parse(body);
        let root = document.root_element();
        // 列表在 .post-index 表格里；类名被改掉时退回页面上第一个表格
        let Some(table) = html::select_first_any(root, &[".post-index", "table"]) else {
            return Vec::new();
        };
        html::select(table, "tr")
            .into_iter()
            .filter_map(|row| {
                let title_anchor = find_anchor(row, "title", is_detail_href)?;
                let torrent_anchor = find_anchor(row, "action", is_torrent_href)?;
                let title = html::title_text(title_anchor);
                let detail_href = html::attr(title_anchor, "href");
                let torrent_href = html::attr(torrent_anchor, "href");
                if title.is_empty() || detail_href.is_empty() || torrent_href.is_empty() {
                    return None;
                }
                let torrent_file_url = self.endpoint.url(torrent_href);
                Some(TorrentInfo {
                    detail_url: Some(self.endpoint.url(detail_href)),
                    src: TorrentSrc::AcgRip.code(),
                    date: html::select_first(row, "time").map(html::text).and_then(opt),
                    size: opt(html::class_text_any(row, &["size"])),
                    magnet_url: Some(torrent_file_url.clone()),
                    torrent_url: Some(torrent_file_url),
                    title: Some(title),
                    ..Default::default()
                })
            })
            .collect()
    }
}

/// 优先在指定 class 的单元格里找；该 class 没了就退回整行按 href 特征匹配。
fn find_anchor<'a>(row: ElementRef<'a>, cell_class: &str, matches: fn(&str) -> bool) -> Option<ElementRef<'a>> {
    let in_cell = html::by_class(row, cell_class)
        .and_then(|cell| html::select(cell, "a").into_iter().find(|a| matches(html::attr(*a, "href"))));
    in_cell.or_else(|| html::select(row, "a").into_iter().find(|a| matches(html::attr(*a, "href"))))
}

/// 详情页链接形如 `/t/<id>`，且不以 `.torrent` 结尾。
fn is_detail_href(href: &str) -> bool {
    href.starts_with("/t/") && !href.ends_with(".torrent")
}

fn is_torrent_href(href: &str) -> bool {
    href.ends_with(".torrent")
}

#[async_trait]
impl SearchService for AcgRip {
    fn source(&self) -> TorrentSrc {
        TorrentSrc::AcgRip
    }

    async fn search(&self, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        let body = self.http.get_text(&self.search_url(keyword, page), &[]).await?;
        Ok(self.parse_search_response(&body))
    }

    /// 条目本身就指向 `.torrent`，不需要进详情页。
    async fn find_magnet_url(&self, detail_url: &str) -> Option<String> {
        opt(detail_url)
    }
}
