//! 磁力猫：中文索引页，条目自带 magnet。

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

const SEARCH_PATH: &str = "/search-{word}-0-0-{page}.html";

pub struct Cilimao {
    http: Arc<dyn HttpClient>,
    endpoint: SiteEndpoint,
}

impl Cilimao {
    pub fn new(http: Arc<dyn HttpClient>, endpoint: SiteEndpoint) -> Self {
        Self { http, endpoint }
    }

    pub fn search_url(&self, keyword: &str, page: u32) -> String {
        self.endpoint.url(&fill(SEARCH_PATH, keyword, page.max(1)))
    }

    pub fn parse_search_response(&self, body: &str) -> Vec<TorrentInfo> {
        let document = html::parse(body);
        let items = html::select_all_any(document.root_element(), &[".ssbox", "[class*=ssbox]"]);
        items
            .into_iter()
            .filter_map(|item| {
                let title_anchor = html::select_first_any(item, &[".title h3 a", "h3 a"])?;
                let title = opt(html::title_text(title_anchor))?;

                let bar = html::select_first(item, ".sbar").unwrap_or(item);
                let details: Vec<String> = html::select(bar, "b").into_iter().map(html::text).collect();
                let detail = |index: usize| details.get(index).and_then(opt);

                let magnet_url = html::href_with_prefix(bar, magnet::MAGNET_PREFIX).and_then(|h| magnet::validated(&h));
                let info_hash = magnet_url.as_deref().and_then(magnet::magnet_to_hash);

                Some(TorrentInfo {
                    torrent_url: info_hash.as_deref().and_then(magnet::hash_to_torrent_url),
                    hash: info_hash,
                    title: Some(title),
                    detail_url: opt(html::attr(title_anchor, "href")).map(|h| self.endpoint.url(&h)),
                    magnet_url,
                    // 尺寸优先取第二个 <b>；结构变了就按内容特征找
                    size: detail(1).or_else(|| opt(html::text_matching(item, &SIZE_TEXT, "td"))),
                    date: detail(0),
                    last_download_date: detail(2),
                    download_count: detail(3),
                    src: TorrentSrc::Cilimao.code(),
                    ..Default::default()
                })
            })
            .collect()
    }
}

#[async_trait]
impl SearchService for Cilimao {
    fn source(&self) -> TorrentSrc {
        TorrentSrc::Cilimao
    }

    async fn search(&self, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        let body = self.http.get_text(&self.search_url(keyword, page), &[]).await?;
        Ok(self.parse_search_response(&body))
    }

    /// 详情页地址末段就是 info hash，不必真的请求详情页。
    async fn find_magnet_url(&self, detail_url: &str) -> Option<String> {
        let segment = last_path_segment(detail_url);
        hash::normalize(segment.strip_suffix(".html").unwrap_or(segment)).and_then(|h| magnet::hash_to_magnet(&h))
    }
}
