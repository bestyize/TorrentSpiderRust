//! Torrent Kitty：经典表格列表，条目自带 magnet。

use std::sync::Arc;

use async_trait::async_trait;

use super::{SearchService, fill, last_path_segment};
use crate::error::SpiderError;
use crate::magnet;
use crate::magnet::hash;
use crate::model::{TorrentInfo, opt};
use crate::net::HttpClient;
use crate::net::html::{self, SIZE_TEXT};
use crate::site::SiteEndpoint;
use crate::site::TorrentSrc;

const SEARCH_PATH: &str = "/search/{word}/{page}";

pub struct TorrentKitty {
    http: Arc<dyn HttpClient>,
    endpoint: SiteEndpoint,
}

impl TorrentKitty {
    pub fn new(http: Arc<dyn HttpClient>, endpoint: SiteEndpoint) -> Self {
        Self { http, endpoint }
    }

    pub fn search_url(&self, keyword: &str, page: u32) -> String {
        self.endpoint.url(&fill(SEARCH_PATH, keyword, page.max(1)))
    }

    pub fn parse_search_response(&self, body: &str) -> Vec<TorrentInfo> {
        let document = html::parse(body);
        let Some(table) = html::select_first_any(document.root_element(), &["#archiveResult", "table"]) else {
            return Vec::new();
        };
        html::select(table, "tr")
            .into_iter()
            .filter_map(|row| {
                let title = opt(html::without_zero_width(&html::class_text_any(row, &["name"])))?;

                // 详情和磁力都是 <a>，靠 href 特征区分，不依赖先后顺序
                let anchors = match html::by_class(row, "action") {
                    Some(action) => html::select(action, "a"),
                    None => html::select(row, "a"),
                };
                let hrefs: Vec<&str> = anchors.iter().map(|a| html::attr(*a, "href")).collect();
                let detail_href = hrefs.iter().find(|h| h.starts_with('/')).copied();
                let magnet_url =
                    hrefs.iter().find(|h| h.starts_with(magnet::MAGNET_PREFIX)).and_then(|h| magnet::validated(h));
                // 表头行（<th class="name">Torrent Description</th>）也带 name 类，但没有任何链接
                if detail_href.is_none() && magnet_url.is_none() {
                    return None;
                }

                Some(TorrentInfo {
                    hash: magnet_url.as_deref().and_then(magnet::magnet_to_hash),
                    torrent_url: magnet_url.as_deref().and_then(magnet::magnet_to_torrent_url),
                    magnet_url,
                    title: Some(title),
                    detail_url: detail_href.map(|h| self.endpoint.url(h)),
                    size: opt(html::class_text_any(row, &["size"]))
                        .or_else(|| opt(html::text_matching(row, &SIZE_TEXT, "td"))),
                    date: opt(html::class_text_any(row, &["date"])),
                    src: TorrentSrc::TorrentKitty.code(),
                    ..Default::default()
                })
            })
            .collect()
    }

    pub fn parse_detail_page(body: &str) -> Option<String> {
        let document = html::parse(body);
        let root = document.root_element();
        let element = html::select_first(root, ".magnet-link");
        // 站点有时把链接放在文本里，有时放在 href 上，两边都试
        element
            .and_then(|el| magnet::validated(&html::text(el)).or_else(|| magnet::validated(html::attr(el, "href"))))
            .or_else(|| html::href_with_prefix(root, "magnet:").and_then(|h| magnet::validated(&h)))
    }
}

#[async_trait]
impl SearchService for TorrentKitty {
    fn source(&self) -> TorrentSrc {
        TorrentSrc::TorrentKitty
    }

    async fn search(&self, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        let body = self.http.get_text(&self.search_url(keyword, page), &[]).await?;
        Ok(self.parse_search_response(&body))
    }

    async fn find_magnet_url(&self, detail_url: &str) -> Option<String> {
        if detail_url.starts_with(magnet::MAGNET_PREFIX) {
            return magnet::validated(detail_url);
        }
        // 本站详情页地址末段就是 info hash，能直接拼出来，省一次请求
        if detail_url.starts_with(self.endpoint.host())
            && let Some(h) = hash::normalize(last_path_segment(detail_url))
        {
            return magnet::hash_to_magnet(&h);
        }
        let body = self.http.get_text(detail_url, &[]).await.ok()?;
        Self::parse_detail_page(&body)
    }
}
