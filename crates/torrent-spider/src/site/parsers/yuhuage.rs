//! 雨花阁：会按 UA 做基础校验，缺了返回空页。

use std::sync::Arc;

use async_trait::async_trait;

use super::{SearchService, fill};
use crate::error::SpiderError;
use crate::magnet;
use crate::model::{TorrentInfo, opt};
use crate::net::HttpClient;
use crate::net::html::{self, SIZE_TEXT};
use crate::site::SiteEndpoint;
use crate::site::TorrentSrc;

const SEARCH_PATH: &str = "/search/{word}-{page}.html";

const HEADERS: &[(&str, &str)] = &[
    ("Referrer-Policy", "strict-origin-when-cross-origin"),
    (
        "User-Agent",
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
    ),
];

pub struct YuHuaGe {
    http: Arc<dyn HttpClient>,
    endpoint: SiteEndpoint,
}

impl YuHuaGe {
    pub fn new(http: Arc<dyn HttpClient>, endpoint: SiteEndpoint) -> Self {
        Self { http, endpoint }
    }

    pub fn search_url(&self, keyword: &str, page: u32) -> String {
        self.endpoint.url(&fill(SEARCH_PATH, keyword, page.max(1)))
    }

    pub fn parse_search_response(&self, body: &str) -> Vec<TorrentInfo> {
        let document = html::parse(body);
        let items = html::select_all_any(document.root_element(), &[".search-item", "[class*=search-item]"]);
        items
            .into_iter()
            .filter_map(|item| {
                let title_anchor = html::select_first_any(item, &[".item-title a", "h3 a"])?;
                let title = opt(html::title_text(title_anchor))?;
                let meta: Vec<String> = html::select(item, ".item-bar span").into_iter().map(html::text).collect();
                Some(TorrentInfo {
                    title: Some(title),
                    detail_url: opt(html::attr(title_anchor, "href")).map(|h| self.endpoint.url(&h)),
                    date: meta.first().and_then(opt),
                    // 站点偶尔多插一个 span，位置对不上就按内容特征找尺寸
                    size: meta.get(1).and_then(opt).or_else(|| opt(html::text_matching(item, &SIZE_TEXT, "span"))),
                    src: TorrentSrc::YuHuaGe.code(),
                    ..Default::default()
                })
            })
            .collect()
    }

    pub fn parse_detail_page(body: &str) -> Option<String> {
        let document = html::parse(body);
        let root = document.root_element();
        html::select_first(root, ".download")
            .and_then(|el| magnet::validated(html::attr(el, "href")))
            .or_else(|| html::href_with_prefix(root, "magnet:").and_then(|h| magnet::validated(&h)))
    }
}

#[async_trait]
impl SearchService for YuHuaGe {
    fn source(&self) -> TorrentSrc {
        TorrentSrc::YuHuaGe
    }

    async fn search(&self, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        let body = self.http.get_text(&self.search_url(keyword, page), HEADERS).await?;
        Ok(self.parse_search_response(&body))
    }

    async fn find_magnet_url(&self, detail_url: &str) -> Option<String> {
        let body = self.http.get_text(detail_url, HEADERS).await.ok()?;
        Self::parse_detail_page(&body)
    }
}
