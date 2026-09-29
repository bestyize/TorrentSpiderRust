//! 1337x 镜像：搜索页没有磁力；详情页可能再指向另一个镜像的详情页。

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use regex::Regex;

use super::{SearchService, fill};
use crate::error::SpiderError;
use crate::magnet;
use crate::magnet::hash;
use crate::model::{TorrentInfo, opt};
use crate::net::HttpClient;
use crate::net::encoding::url_encode;
use crate::net::html;
use crate::site::SiteEndpoint;
use crate::site::TorrentSrc;

const SEARCH_PATH: &str = "/search/{word}/{page}/";

/// 详情页正文里以「Info Hash : xxxx」形式给出的哈希，十六进制和 Base32 都认。
static INFO_HASH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Info\s*Hash\s*:?\s*([A-Fa-f0-9]{40}|[A-Za-z2-7]{32})").unwrap());

/// 已改作他用的种子缓存域名，站点上残留的旧链接不可信。
const DEAD_TORRENT_MIRRORS: &[&str] = &["torrage.info", "btcache.me"];

pub struct X1337 {
    http: Arc<dyn HttpClient>,
    endpoint: SiteEndpoint,
}

impl X1337 {
    pub fn new(http: Arc<dyn HttpClient>, endpoint: SiteEndpoint) -> Self {
        Self { http, endpoint }
    }

    pub fn search_url(&self, keyword: &str, page: u32) -> String {
        self.endpoint.url(&fill(SEARCH_PATH, keyword, page.max(1)))
    }

    pub fn parse_search_response(&self, body: &str) -> Vec<TorrentInfo> {
        let document = html::parse(body);
        let rows = html::select_all_any(document.root_element(), &["tbody.torsearch tr", "tbody tr", "tr"]);
        rows.into_iter()
            .filter_map(|row| {
                let name_column = html::select_first_any(row, &[".coll-1.name", ".coll-1", "td.name"])?;
                let detail_link = html::select_first_any(name_column, &["a.openAdd", "a[href*='/torrent/']"])
                    .or_else(|| html::select(name_column, "a").pop())?;
                let title = opt(html::attr(detail_link, "id-text")).or_else(|| opt(html::title_text(detail_link)))?;
                let link = opt(html::attr(detail_link, "href"))?;
                Some(TorrentInfo {
                    title: Some(title),
                    detail_url: Some(self.endpoint.url(&link)),
                    date: opt(html::text_any(row, &[".coll-date", ".coll-date-2"]).replace('\'', "")),
                    size: opt(html::text_any(row, &[".coll-4.size.mob-uploader", ".coll-4.size", ".size"])),
                    leacher_count: opt(html::text_any(row, &[".coll-3.leeches", ".leeches"])),
                    sender_count: opt(html::text_any(row, &[".coll-2.seeds", ".seeds"])),
                    src: TorrentSrc::X1337.code(),
                    ..Default::default()
                })
            })
            .collect()
    }

    async fn fetch(&self, url: Option<&str>) -> Option<String> {
        let url = url.filter(|u| !u.trim().is_empty())?;
        self.http.get_text(url, &[]).await.ok()
    }
}

/// 详情页上现成的磁力链接。
pub fn extract_magnet(body: &str) -> Option<String> {
    let document = html::parse(body);
    html::href_with_prefix(document.root_element(), "magnet:").and_then(|h| magnet::validated(&h))
}

/// 详情页指向的另一个镜像详情页。按 `1337x.` + `/torrent/` 两个特征匹配，
/// 不写死具体镜像域名——这类镜像换得比主站还勤。
pub fn extract_mirror_detail_url(body: &str) -> Option<String> {
    let document = html::parse(body);
    let anchor =
        html::select_first_any(document.root_element(), &["a#openPopup", "a[href*='1337x.'][href*='/torrent/']"])?;
    opt(html::attr(anchor, "href")).filter(|h| !h.starts_with("magnet:"))
}

/// 详情页上可信的 `.torrent` 下载地址。
pub fn extract_torrent_mirror_url(body: &str) -> Option<String> {
    let document = html::parse(body);
    html::select(document.root_element(), "a[href]")
        .into_iter()
        .map(|a| html::attr(a, "href"))
        .find(|href| href.ends_with(".torrent") && !DEAD_TORRENT_MIRRORS.iter().any(|dead| href.contains(dead)))
        .map(str::to_owned)
}

/// 正文里「Info Hash : …」给出的哈希。
pub fn extract_info_hash(body: &str) -> Option<String> {
    let document = html::parse(body);
    let text: String = document.root_element().text().collect();
    INFO_HASH.captures(&text).and_then(|caps| hash::normalize(&caps[1]))
}

/// 用正文里的哈希和页面标题拼 magnet。
pub fn build_magnet_from_info_hash(body: &str) -> Option<String> {
    let info_hash = extract_info_hash(body)?;
    let document = html::parse(body);
    let title = html::text_any(document.root_element(), &["h1", "title"]);
    Some(match opt(title) {
        Some(title) => format!("{}{info_hash}&dn={}", magnet::MAGNET_PREFIX, url_encode(&title)),
        None => format!("{}{info_hash}", magnet::MAGNET_PREFIX),
    })
}

#[async_trait]
impl SearchService for X1337 {
    fn source(&self) -> TorrentSrc {
        TorrentSrc::X1337
    }

    async fn search(&self, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        let body = self.http.get_text(&self.search_url(keyword, page), &[]).await?;
        Ok(self.parse_search_response(&body))
    }

    async fn find_magnet_url(&self, detail_url: &str) -> Option<String> {
        let detail = self.fetch(Some(detail_url)).await.unwrap_or_default();
        if let Some(found) = extract_magnet(&detail) {
            return Some(found);
        }
        let mirror_url = extract_mirror_detail_url(&detail);
        let mirror = self.fetch(mirror_url.as_deref()).await.unwrap_or_default();
        extract_magnet(&mirror)
            .or_else(|| build_magnet_from_info_hash(&mirror))
            .or_else(|| build_magnet_from_info_hash(&detail))
    }

    async fn find_torrent_url(&self, detail_url: &str) -> Option<String> {
        if detail_url.ends_with(".torrent") {
            return Some(detail_url.to_owned());
        }
        let detail = self.fetch(Some(detail_url)).await.unwrap_or_default();
        if let Some(found) = extract_torrent_mirror_url(&detail) {
            return Some(found);
        }
        let mirror_url = extract_mirror_detail_url(&detail);
        let mirror = self.fetch(mirror_url.as_deref()).await.unwrap_or_default();
        extract_torrent_mirror_url(&mirror)
            .or_else(|| extract_info_hash(&mirror).and_then(|h| magnet::hash_to_torrent_url(&h)))
            .or_else(|| extract_info_hash(&detail).and_then(|h| magnet::hash_to_torrent_url(&h)))
    }
}
