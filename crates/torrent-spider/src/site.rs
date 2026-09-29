//! 站点：编号与清单、域名表、解析器，以及把它们装配起来的注册表。

use async_trait::async_trait;

use crate::error::SpiderError;
use crate::magnet;
use crate::model::TorrentInfo;

mod catalog;
mod endpoint;
pub mod parsers;
mod registry;

pub use catalog::{TorrentSrc, supported_sources};
pub use endpoint::{CILIDUO_ENGINE, CILIDUO_REFERER, DefaultSiteEndpoints, SiteEndpoint, SiteEndpointProvider};
pub use registry::TorrentServiceRegistry;

/// 一个站点的搜索能力。
#[async_trait]
pub trait SearchService: Send + Sync {
    fn source(&self) -> TorrentSrc;

    /// 搜索一页结果。请求失败返回错误；解析不出条目返回空列表。
    async fn search(&self, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError>;

    /// 按详情页地址解析磁力。从不报错，拿不到就是 `None`。
    async fn find_magnet_url(&self, detail_url: &str) -> Option<String>;

    /// 按详情页地址解析 `.torrent` 下载地址。默认由磁力推导。
    async fn find_torrent_url(&self, detail_url: &str) -> Option<String> {
        if detail_url.ends_with(".torrent") {
            return Some(detail_url.to_owned());
        }
        let magnet_url = self.find_magnet_url(detail_url).await?;
        magnet::magnet_to_torrent_url(&magnet_url)
    }
}
