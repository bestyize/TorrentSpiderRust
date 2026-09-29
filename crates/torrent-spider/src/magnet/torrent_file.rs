//! 按 info hash 提供 `.torrent` 文件的缓存服务。
//!
//! 已实测（2026-09）：
//! - `itorrents.net` 直接返回 `application/x-bittorrent`，可用；
//! - `itorrents.org` 301 跳到 `.net`，可用但多一跳。
//!
//! 曾经常用的 `torrage.info`、`btcache.me` 已改作他用，不要再加回来。
//! itorrents 缓存未命中时不是 404，而是跳到别的站点返回 HTML，
//! 所以判断可用性必须看 content-type，这由 [`TorrentFileResolver`] 负责。

use std::sync::Arc;

use crate::magnet::hash;
use crate::net::{HeadResponse, HttpClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TorrentFileSource {
    pub name: &'static str,
    /// 形如 `https://host/torrent/{hash}.torrent`。
    pub template: &'static str,
}

impl TorrentFileSource {
    pub fn url_for(&self, info_hash: &str) -> String {
        self.template.replace("{hash}", info_hash)
    }
}

pub const ITORRENTS: TorrentFileSource =
    TorrentFileSource { name: "itorrents.net", template: "https://itorrents.net/torrent/{hash}.torrent" };

pub const ITORRENTS_LEGACY: TorrentFileSource =
    TorrentFileSource { name: "itorrents.org", template: "https://itorrents.org/torrent/{hash}.torrent" };

pub const DEFAULT_SOURCES: &[TorrentFileSource] = &[ITORRENTS, ITORRENTS_LEGACY];

/// 依次探测候选缓存源，返回第一个真正提供种子文件的地址。
#[derive(Clone)]
pub struct TorrentFileResolver {
    http: Arc<dyn HttpClient>,
    sources: Vec<TorrentFileSource>,
}

impl TorrentFileResolver {
    pub fn new(http: Arc<dyn HttpClient>) -> Self {
        Self::with_sources(http, DEFAULT_SOURCES.to_vec())
    }

    pub fn with_sources(http: Arc<dyn HttpClient>, sources: Vec<TorrentFileSource>) -> Self {
        Self { http, sources }
    }

    /// 不做网络探测，只列出候选地址。
    pub fn candidates(&self, raw_hash: &str) -> Vec<String> {
        match hash::normalize(raw_hash) {
            Some(h) => self.sources.iter().map(|s| s.url_for(&h)).collect(),
            None => Vec::new(),
        }
    }

    /// 返回第一个可用的下载地址；全部不可用或哈希不合法时返回 `None`。
    pub async fn resolve(&self, raw_hash: &str) -> Option<String> {
        for url in self.candidates(raw_hash) {
            if let Ok(response) = self.http.head(&url).await
                && looks_like_torrent_file(&response)
            {
                return Some(url);
            }
        }
        None
    }
}

/// 非 2xx 或文本类 content-type 视为不可用；缺失 content-type 时保守接受，
/// 部分缓存服务只靠扩展名表达类型。
pub fn looks_like_torrent_file(response: &HeadResponse) -> bool {
    if !(200..300).contains(&response.status) {
        return false;
    }
    let content_type = response.content_type.as_deref().unwrap_or("").to_ascii_lowercase();
    !content_type.starts_with("text/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHttp;
    use crate::testing::*;

    fn head(status: u16, content_type: Option<&str>) -> HeadResponse {
        HeadResponse { status, content_type: content_type.map(str::to_owned) }
    }

    #[test]
    fn content_type_rules() {
        assert!(looks_like_torrent_file(&head(200, Some("application/x-bittorrent"))));
        assert!(looks_like_torrent_file(&head(200, None)));
        assert!(!looks_like_torrent_file(&head(200, Some("text/html; charset=utf-8"))));
        assert!(!looks_like_torrent_file(&head(404, Some("application/x-bittorrent"))));
    }

    #[tokio::test]
    async fn picks_first_source_serving_a_torrent() {
        let http = FakeHttp::new()
            .head_route("https://itorrents.net/", head(200, Some("text/html")))
            .head_route("https://itorrents.org/", head(200, Some("application/x-bittorrent")));
        let resolver = TorrentFileResolver::new(http.clone());
        assert_eq!(resolver.resolve(HEX_LOWER).await, Some(format!("https://itorrents.org/torrent/{HEX}.torrent")));
        assert_eq!(http.requested_urls().len(), 2);
    }

    #[tokio::test]
    async fn returns_none_when_nothing_serves_or_hash_is_bad() {
        let http = FakeHttp::new();
        let resolver = TorrentFileResolver::new(http.clone());
        assert_eq!(resolver.resolve(HEX).await, None);
        assert_eq!(resolver.resolve("bad").await, None);
        assert!(resolver.candidates("bad").is_empty());
        assert_eq!(resolver.candidates(BASE32).len(), 2);
    }
}
