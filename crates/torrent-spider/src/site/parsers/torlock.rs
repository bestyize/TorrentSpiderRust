//! TorLock：搜索结果页里没有磁力，磁力只在详情页上。

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

const SEARCH_PATH: &str = "/all/torrents/{word}/{page}.html";
const DETAIL_PATH_PREFIX: &str = "/torrent";

pub struct TorLock {
    http: Arc<dyn HttpClient>,
    endpoint: SiteEndpoint,
}

impl TorLock {
    pub fn new(http: Arc<dyn HttpClient>, endpoint: SiteEndpoint) -> Self {
        Self { http, endpoint }
    }

    pub fn search_url(&self, keyword: &str, page: u32) -> String {
        self.endpoint.url(&fill(SEARCH_PATH, keyword, page.max(1)))
    }

    pub fn parse_search_response(&self, body: &str) -> Vec<TorrentInfo> {
        let document = html::parse(body);
        let tables = html::select(document.root_element(), ".table-responsive");
        // 条目表是第二张；表数量变了就找第一张有尺寸单元格的
        let table = tables
            .get(1)
            .copied()
            .or_else(|| tables.iter().copied().find(|t| !html::text_matching(*t, &SIZE_TEXT, "td").is_empty()));
        let Some(table) = table else {
            return Vec::new();
        };

        html::select(table, "tr")
            .into_iter()
            .filter_map(|row| {
                let size = opt(html::class_text_any(row, &["ts"]))
                    .or_else(|| opt(html::text_matching(row, &SIZE_TEXT, "td")))?;
                // 标题前面还有一个分类链接（/software.html），按 href 特征找详情页链接
                let anchor = html::select(row, "a[href]")
                    .into_iter()
                    .find(|a| html::attr(*a, "href").starts_with(DETAIL_PATH_PREFIX))?;
                let link = html::attr(anchor, "href");
                // 日期单元格里是 <time datetime="2022-05-19">5/19/2022</time>，优先取机读值
                let date = html::select_first(row, "time[datetime]")
                    .and_then(|t| opt(html::attr(t, "datetime")))
                    .or_else(|| opt(html::class_text_any(row, &["td"])));
                Some(TorrentInfo {
                    title: opt(html::title_text(anchor)),
                    detail_url: Some(self.endpoint.url(link)),
                    size: Some(size),
                    date,
                    sender_count: opt(html::class_text_any(row, &["tul"])),
                    leacher_count: opt(html::class_text_any(row, &["tdl"])),
                    src: TorrentSrc::TorLock.code(),
                    ..Default::default()
                })
            })
            .collect()
    }

    pub fn parse_detail_page(body: &str) -> Option<String> {
        let document = html::parse(body);
        let root = document.root_element();
        // 磁力图标是个 <i>，链接挂在它的父节点上；直接命中 <a> 时用它自己
        let from_icon = html::select_first(root, ".fa-magnet").and_then(|icon| {
            let holder = if icon.value().attr("href").is_some() { icon } else { html::parent(icon)? };
            magnet::validated(html::attr(holder, "href"))
        });
        from_icon.or_else(|| html::href_with_prefix(root, "magnet:").and_then(|h| magnet::validated(&h)))
    }
}

#[async_trait]
impl SearchService for TorLock {
    fn source(&self) -> TorrentSrc {
        TorrentSrc::TorLock
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
