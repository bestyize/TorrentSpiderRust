//! 0magnet：列表页只有标题和尺寸，磁力要进详情页取。

use std::sync::Arc;

use async_trait::async_trait;
use scraper::ElementRef;
use scraper::node::Node;

use super::SearchService;
use crate::error::SpiderError;
use crate::magnet;
use crate::model::{TorrentInfo, opt};
use crate::net::HttpClient;
use crate::net::encoding::url_encode;
use crate::net::html::{self, SIZE_TEXT};
use crate::site::SiteEndpoint;
use crate::site::TorrentSrc;

const DETAIL_PATH_PREFIX: &str = "/detail/";

pub struct ZeroMagnet {
    http: Arc<dyn HttpClient>,
    endpoint: SiteEndpoint,
}

impl ZeroMagnet {
    pub fn new(http: Arc<dyn HttpClient>, endpoint: SiteEndpoint) -> Self {
        Self { http, endpoint }
    }

    /// 站点没有分页参数，`page` 被忽略。
    pub fn search_url(&self, keyword: &str, _page: u32) -> String {
        self.endpoint.url(&format!("/search?q={}", url_encode(keyword)))
    }

    pub fn parse_search_response(&self, body: &str) -> Vec<TorrentInfo> {
        let document = html::parse(body);
        let cells = html::select(document.root_element(), "td");
        // 列表是「标题单元格 + 尺寸单元格」两两成对排的
        cells
            .chunks_exact(2)
            .filter_map(|pair| {
                let (title_cell, size_cell) = (pair[0], pair[1]);
                let detail_href =
                    html::select_first_any(title_cell, &[&format!("a[href^='{DETAIL_PATH_PREFIX}']"), "a"])
                        .map(|a| html::attr(a, "href"))
                        .and_then(opt)?;
                let title = opt(html::without_zero_width(&text_without_paragraphs(title_cell)))?;
                Some(TorrentInfo {
                    detail_url: Some(self.endpoint.url(&detail_href)),
                    title: Some(title),
                    size: opt(html::text(size_cell)).or_else(|| opt(html::text_matching(title_cell, &SIZE_TEXT, "td"))),
                    src: TorrentSrc::ZeroMagnet.code(),
                    ..Default::default()
                })
            })
            .collect()
    }

    /// 详情页把完整 magnet 放在隐藏输入框的 value 里；站点给的链接带一堆杂项参数，
    /// 只留哈希重拼成最小链接。
    pub fn parse_detail_page(body: &str) -> Option<String> {
        let document = html::parse(body);
        let root = document.root_element();
        let raw = html::select_first_any(root, &["#input-magnet", "input[name=magnet]"])
            .map(|el| html::attr(el, "value").to_owned())
            .and_then(opt)
            .or_else(|| html::href_with_prefix(root, "magnet:"))?;
        magnet::magnet_to_hash(&raw).and_then(|h| magnet::hash_to_magnet(&h))
    }
}

/// 标题单元格里混着一段 `<p>` 提示文案，取正文时跳过。
fn text_without_paragraphs(cell: ElementRef<'_>) -> String {
    let mut out = String::new();
    for node in cell.descendants() {
        if let Node::Text(text) = node.value() {
            let inside_paragraph = node
                .ancestors()
                .take_while(|a| a.id() != cell.id())
                .any(|a| a.value().as_element().is_some_and(|e| e.name() == "p"));
            if !inside_paragraph {
                out.push_str(text);
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[async_trait]
impl SearchService for ZeroMagnet {
    fn source(&self) -> TorrentSrc {
        TorrentSrc::ZeroMagnet
    }

    async fn search(&self, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        let body = self.http.get_text(&self.search_url(keyword, page), &[]).await?;
        Ok(self.parse_search_response(&body))
    }

    async fn find_magnet_url(&self, detail_url: &str) -> Option<String> {
        let headers = [("Referrer-Policy", "strict-origin-when-cross-origin")];
        let body = self.http.get_text(detail_url, &headers).await.ok()?;
        Self::parse_detail_page(&body)
    }
}
