use std::sync::Arc;
use std::time::Duration;

use crate::error::SpiderError;
use crate::model::{TorrentInfo, TorrentSource};
use crate::net::HttpClient;
use crate::search::aggregate::{self, AggregateResult};
use crate::search::relevance;
use crate::site::{SiteEndpointProvider, TorrentServiceRegistry, TorrentSrc};

/// 单个源一次搜索的默认上限。HTTP 层自己也有超时，这里兜底防止宿主注入的客户端没配超时。
pub const DEFAULT_SEARCH_TIMEOUT: Duration = Duration::from_secs(25);

/// 库的主入口。
#[derive(Clone)]
pub struct TorrentSpiderClient {
    registry: TorrentServiceRegistry,
    search_timeout: Duration,
}

impl TorrentSpiderClient {
    /// 用内置域名表和注入的 HTTP 客户端装配全部站点。
    pub fn new(http: Arc<dyn HttpClient>) -> Self {
        Self::with_registry(TorrentServiceRegistry::new(http))
    }

    pub fn with_endpoints(http: Arc<dyn HttpClient>, endpoints: &dyn SiteEndpointProvider) -> Self {
        Self::with_registry(TorrentServiceRegistry::with_endpoints(http, endpoints))
    }

    pub fn with_registry(registry: TorrentServiceRegistry) -> Self {
        Self { registry, search_timeout: DEFAULT_SEARCH_TIMEOUT }
    }

    /// 单个源一次搜索的上限，聚合搜索时每个源单独计时。
    #[must_use]
    pub fn search_timeout(mut self, timeout: Duration) -> Self {
        self.search_timeout = timeout;
        self
    }

    pub fn registry(&self) -> &TorrentServiceRegistry {
        &self.registry
    }

    pub fn sources(&self) -> &[TorrentSource] {
        self.registry.sources()
    }

    /// 搜索单个源。请求失败或超时返回错误；站点改版导致解析不出条目时返回空列表。
    pub async fn search(&self, src: TorrentSrc, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        let service = self.registry.service(src);
        match tokio::time::timeout(self.search_timeout, service.search(keyword, page.max(1))).await {
            Ok(result) => result,
            Err(_) => Err(SpiderError::Timeout(self.search_timeout)),
        }
    }

    /// 按编号搜索；未知编号返回 [`SpiderError::UnknownSource`]。
    pub async fn search_code(&self, code: u32, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        let src = TorrentSrc::from_code(code).ok_or(SpiderError::UnknownSource(code))?;
        self.search(src, keyword, page).await
    }

    /// 并发搜索全部源。单个源失败只体现在它自己的状态里。
    pub async fn search_all(&self, keyword: &str, page: u32) -> AggregateResult {
        self.search_many(&TorrentSrc::ALL, keyword, page).await
    }

    /// 并发搜索指定的几个源，结果按与关键词的相关性排序（见 [`crate::search::relevance::rank`]）。
    pub async fn search_many(&self, sources: &[TorrentSrc], keyword: &str, page: u32) -> AggregateResult {
        let targets = sources.iter().map(|src| (src.code(), src.title().to_owned()));
        let mut result = aggregate::fan_out(targets, self.search_timeout, |code| {
            let service = TorrentSrc::from_code(code).map(|src| self.registry.service(src));
            async move {
                match service {
                    Some(service) => service.search(keyword, page.max(1)).await,
                    None => Err(SpiderError::UnknownSource(code)),
                }
            }
        })
        .await;
        result.items = relevance::rank(std::mem::take(&mut result.items), keyword);
        result
    }

    /// 按详情页地址解析磁力。从不报错，拿不到就是 `None`。
    pub async fn find_magnet_url(&self, src: TorrentSrc, detail_url: &str) -> Option<String> {
        self.registry.service(src).find_magnet_url(detail_url).await
    }

    /// 按详情页地址解析 `.torrent` 下载地址。
    pub async fn find_torrent_url(&self, src: TorrentSrc, detail_url: &str) -> Option<String> {
        self.registry.service(src).find_torrent_url(detail_url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[tokio::test]
    async fn searches_and_resolves_through_injected_http() {
        let http = FakeHttp::new()
            .route(
                "https://0magnet.com/search",
                r#"<html><body><table><tr>
                    <td><a href="/detail/abc"></a><p>extra</p>Movie Name</td>
                    <td>1.2 GB</td>
                </tr></table></body></html>"#,
            )
            .route(
                "https://0magnet.com/detail/abc",
                &format!(r#"<input id="input-magnet" value="{}" />"#, magnet_named(HEX, "movie")),
            );
        let client = TorrentSpiderClient::new(http.clone());

        let items = client.search(TorrentSrc::ZeroMagnet, "movie", 1).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title.as_deref(), Some("Movie Name"));
        assert_eq!(items[0].size.as_deref(), Some("1.2 GB"));

        let detail = "https://0magnet.com/detail/abc";
        assert_eq!(client.find_magnet_url(TorrentSrc::ZeroMagnet, detail).await, Some(magnet(HEX)));
        assert_eq!(client.find_torrent_url(TorrentSrc::ZeroMagnet, detail).await, Some(torrent_url(HEX)));
    }

    #[tokio::test]
    async fn unknown_code_is_an_error_not_a_fallback() {
        let client = TorrentSpiderClient::new(FakeHttp::new());
        assert_eq!(client.search_code(9999, "x", 1).await, Err(SpiderError::UnknownSource(9999)));
    }

    #[tokio::test]
    async fn search_all_reports_every_source() {
        let client = TorrentSpiderClient::new(FakeHttp::new());
        let result = client.search_all("x", 1).await;
        assert_eq!(result.statuses.len(), TorrentSrc::ALL.len());
        assert!(result.all_failed(), "FakeHttp answers 404 to unknown routes");
    }
}
