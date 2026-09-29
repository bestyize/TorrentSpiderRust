//! 服务端真正用到的核心库能力，收窄成两个 trait，测试可以直接喂假实现。
//!
//! 搜索与「按详情页取磁力」分成两个 trait：失败语义完全不同——
//! 搜索会报错，取磁力从不报错，拿不到就是 `None`。

use std::sync::Arc;

use async_trait::async_trait;
use torrent_spider::{SpiderError, TorrentInfo, TorrentSpiderClient, TorrentSrc};

#[async_trait]
pub trait SearchGateway: Send + Sync {
    async fn search(&self, src: u32, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError>;
}

#[async_trait]
pub trait MagnetGateway: Send + Sync {
    /// 解析出的磁力链接；拿不到时是 `None`。
    async fn find_magnet_url(&self, src: u32, detail_url: &str) -> Option<String>;
}

/// 生产实现：直接转给核心库。
#[derive(Clone)]
pub struct ClientGateway(pub Arc<TorrentSpiderClient>);

#[async_trait]
impl SearchGateway for ClientGateway {
    async fn search(&self, src: u32, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        self.0.search_code(src, keyword, page).await
    }
}

#[async_trait]
impl MagnetGateway for ClientGateway {
    async fn find_magnet_url(&self, src: u32, detail_url: &str) -> Option<String> {
        let src = TorrentSrc::from_code(src)?;
        self.0.find_magnet_url(src, detail_url).await
    }
}
