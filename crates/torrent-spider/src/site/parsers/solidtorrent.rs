//! BitSearch / SolidTorrent：Tailwind 样式的卡片列表。

use std::sync::Arc;

use async_trait::async_trait;
use scraper::ElementRef;

use super::{SearchService, fill};
use crate::error::SpiderError;
use crate::magnet;
use crate::model::{TorrentInfo, opt};
use crate::net::HttpClient;
use crate::net::html::{self, SIZE_TEXT};
use crate::site::SiteEndpoint;
use crate::site::TorrentSrc;

const SEARCH_PATH: &str = "/search?q={word}&page={page}";

pub struct SolidTorrent {
    http: Arc<dyn HttpClient>,
    endpoint: SiteEndpoint,
}

impl SolidTorrent {
    pub fn new(http: Arc<dyn HttpClient>, endpoint: SiteEndpoint) -> Self {
        Self { http, endpoint }
    }

    pub fn search_url(&self, keyword: &str, page: u32) -> String {
        self.endpoint.url(&fill(SEARCH_PATH, keyword, page.max(1)))
    }

    pub fn parse_search_response(&self, body: &str) -> Vec<TorrentInfo> {
        let document = html::parse(body);
        let Some(root) = html::select_first_any(document.root_element(), &[".space-y-4", "[class*=space-y]"]) else {
            return Vec::new();
        };
        root.child_elements().filter_map(|card| self.card_to_info(card)).collect()
    }

    fn card_to_info(&self, card: ElementRef<'_>) -> Option<TorrentInfo> {
        let detail_anchor = html::select_first_any(card, &["h3 a", "h3 [href]"])?;
        let title = opt(html::title_text(detail_anchor))?;

        // 按 href 特征区分种子和磁力，不依赖它们在页面上的先后顺序
        let links: Vec<String> =
            html::select(card, "[href]").into_iter().map(|el| html::attr(el, "href").replace("\\u003d", "=")).collect();
        let torrent_link = links.iter().find(|l| l.ends_with(".torrent")).cloned();
        let magnet_url = links.iter().find(|l| l.starts_with(magnet::MAGNET_PREFIX)).and_then(|l| magnet::validated(l));
        let info_hash = magnet_url.as_deref().and_then(magnet::magnet_to_hash);

        // 站点用 Tailwind 工具类当 class 名，尺寸额外按内容特征兜底
        let meta: Vec<String> = html::select(card, ".text-gray-600 .inline-flex").into_iter().map(html::text).collect();
        let size = meta.get(1).and_then(opt).or_else(|| opt(html::text_matching(card, &SIZE_TEXT, "span")));

        Some(TorrentInfo {
            magnet_url: info_hash.as_deref().and_then(magnet::hash_to_magnet).or(magnet_url),
            torrent_url: torrent_link.or_else(|| info_hash.as_deref().and_then(magnet::hash_to_torrent_url)),
            hash: info_hash,
            title: Some(title),
            date: meta.get(2).and_then(opt),
            size,
            detail_url: opt(html::attr(detail_anchor, "href")).map(|h| self.endpoint.url(&h)),
            download_count: metric(card, "blue"),
            sender_count: metric(card, "green"),
            leacher_count: metric(card, "red"),
            src: TorrentSrc::SolidTorrent.code(),
            ..Default::default()
        })
    }

    pub fn parse_detail_page(body: &str) -> Option<String> {
        let document = html::parse(body);
        let root = document.root_element();
        let from_block = html::select_first_any(root, &[".dl-links", "[class*=dl-link]"])
            .and_then(|block| html::href_with_prefix(block, magnet::MAGNET_PREFIX));
        // 下载区整块被换掉时退回全页扫一遍
        from_block.or_else(|| html::href_with_prefix(root, magnet::MAGNET_PREFIX)).and_then(|h| magnet::validated(&h))
    }
}

/// 某个颜色标注的指标数字：蓝 = 下载数、绿 = 做种、红 = 吸血。
///
/// 下载数这一格在没有下载记录时整个不渲染，所以不能按位置取。
/// 规则是「同色元素里，内容全是数字的 `.font-medium`」：分类图标同色但没有数字，
/// 标题旁的 `Verified` 徽章同色且带 `font-medium` 但不是数字，都被挡掉。
/// 用色相子串（`text-blue-`）扛换色阶，换色相只能改选择器。
fn metric(card: ElementRef<'_>, color: &str) -> Option<String> {
    let colored = html::select(card, &format!("[class*=\"text-{color}-\"]"));
    colored.into_iter().find_map(|element| {
        let candidates = std::iter::once(element).chain(html::select(element, ".font-medium"));
        candidates
            .filter(|el| el.value().classes().any(|c| c == "font-medium"))
            .map(html::text)
            .find(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()))
    })
}

#[async_trait]
impl SearchService for SolidTorrent {
    fn source(&self) -> TorrentSrc {
        TorrentSrc::SolidTorrent
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
