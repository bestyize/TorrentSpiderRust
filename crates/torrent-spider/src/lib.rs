//! TorrentSpider 核心库：多个磁力索引站的搜索与磁力解析。
//!
//! 库本身不绑定任何 HTTP 实现，网络请求一律通过注入的 [`HttpClient`] 发出；
//! 打开 `reqwest` feature 可以直接用内置的 [`ReqwestHttpClient`]。
//!
//! ```no_run
//! # #[cfg(feature = "reqwest")]
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use torrent_spider::{ReqwestHttpClient, TorrentSpiderClient, TorrentSrc};
//!
//! let http = Arc::new(ReqwestHttpClient::builder().build()?);
//! let client = TorrentSpiderClient::new(http);
//! let items = client.search(TorrentSrc::BtSow, "ubuntu", 1).await?;
//! println!("{} results", items.len());
//! # Ok(())
//! # }
//! ```

mod client;
mod error;
pub mod magnet;
mod model;
pub mod net;
pub mod search;
pub mod site;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use client::TorrentSpiderClient;
pub use error::SpiderError;
pub use magnet::{MagnetLink, TorrentFileResolver, TorrentFileSource};
pub use model::{TorrentInfo, TorrentSource};
pub use net::{HeadResponse, HttpClient, HttpError};
#[cfg(feature = "reqwest")]
pub use net::{ReqwestHttpClient, ReqwestHttpClientBuilder};
pub use search::{AggregateResult, SourceStatus};
pub use site::{
    DefaultSiteEndpoints, SearchService, SiteEndpoint, SiteEndpointProvider, TorrentServiceRegistry, TorrentSrc,
    supported_sources,
};
