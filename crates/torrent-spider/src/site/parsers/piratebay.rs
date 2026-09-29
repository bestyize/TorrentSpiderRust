//! 海盗湾镜像：表格列表，第 4 列带 magnet。

use std::sync::Arc;

use async_trait::async_trait;

use super::{SearchService, fill};
use crate::error::SpiderError;
use crate::magnet;
use crate::model::{TorrentInfo, opt};
use crate::net::HttpClient;
use crate::net::html;
use crate::site::SiteEndpoint;
use crate::site::TorrentSrc;

const SEARCH_PATH: &str = "/search/{word}/{page}/99/0";

pub struct PirateBay {
    http: Arc<dyn HttpClient>,
    endpoint: SiteEndpoint,
}

impl PirateBay {
    pub fn new(http: Arc<dyn HttpClient>, endpoint: SiteEndpoint) -> Self {
        Self { http, endpoint }
    }

    pub fn search_url(&self, keyword: &str, page: u32) -> String {
        self.endpoint.url(&fill(SEARCH_PATH, keyword, page.max(1)))
    }

    pub fn parse_search_response(&self, body: &str) -> Vec<TorrentInfo> {
        let document = html::parse(body);
        let rows = html::select_all_any(document.root_element(), &["#searchResult tr", "tr"]);
        rows.into_iter()
            .filter_map(|row| {
                let columns = html::select(row, "td");
                if columns.len() < 5 {
                    return None;
                }
                let title_anchor = html::select_first(columns[1], "a")?;
                let title = opt(html::title_text(title_anchor))?;
                let magnet_url = html::select(columns[3], "a")
                    .into_iter()
                    .map(|a| html::attr(a, "href"))
                    .find(|href| href.starts_with(magnet::MAGNET_PREFIX))
                    .and_then(magnet::validated);
                Some(TorrentInfo {
                    hash: magnet_url.as_deref().and_then(magnet::magnet_to_hash),
                    torrent_url: magnet_url.as_deref().and_then(magnet::magnet_to_torrent_url),
                    magnet_url,
                    title: Some(title),
                    size: opt(html::text(columns[4])),
                    date: opt(html::text(columns[2])),
                    detail_url: opt(html::attr(title_anchor, "href")).map(|h| self.endpoint.url(&h)),
                    src: TorrentSrc::PirateBay.code(),
                    ..Default::default()
                })
            })
            .collect()
    }

    pub fn parse_detail_page(body: &str) -> Option<String> {
        let document = html::parse(body);
        let root = document.root_element();
        // 下载区里第一个链接就是磁力；class 改名了就按 href 特征找
        html::select_first(root, ".download a")
            .and_then(|a| magnet::validated(html::attr(a, "href")))
            .or_else(|| html::href_with_prefix(root, "magnet:").and_then(|h| magnet::validated(&h)))
    }
}

#[async_trait]
impl SearchService for PirateBay {
    fn source(&self) -> TorrentSrc {
        TorrentSrc::PirateBay
    }

    async fn search(&self, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        let body = self.http.get_text(&self.search_url(keyword, page), &[]).await?;
        Ok(self.parse_search_response(&body))
    }

    async fn find_magnet_url(&self, detail_url: &str) -> Option<String> {
        let body = self.http.get_text(detail_url, &[]).await.ok()?;
        Self::parse_detail_page(&body)
    }
}
