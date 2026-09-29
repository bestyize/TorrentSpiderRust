//! 各测试模块共用的假网关、测试台与断言小工具。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;

use axum::extract::connect_info::MockConnectInfo;
use torrent_spider::HttpError;

// 各测试模块都会用到的类型，经 `use crate::support::*` 一并引入
pub use std::sync::Arc;
pub use std::time::Duration;

pub use async_trait::async_trait;
pub use axum::body::Body;
pub use axum::http::{Request, StatusCode, header};
pub use http_body_util::BodyExt;
pub use serde_json::Value;
pub use torrent_spider::{SpiderError, TorrentInfo, TorrentSource, supported_sources};
pub use torrent_spider_server::{AppState, MagnetGateway, SearchGateway, router};
pub use tower::ServiceExt;

pub const SAMPLE_HASH: &str = "2C6B6858D61DA9543D4231A71DB4B1C9264B0685";
pub const SAMPLE_MAGNET: &str =
    "magnet:?xt=urn:btih:2C6B6858D61DA9543D4231A71DB4B1C9264B0685&dn=Ubuntu-22.04-desktop-amd64";
pub const DETAIL: &str = "https://www.torlock.com/torrent/63489835/ubuntu-22-04-desktop-amd64.html";

pub type Handler = Box<dyn Fn(&str, u32) -> Result<Vec<TorrentInfo>, SpiderError> + Send + Sync>;

/// 假搜索网关：每个站点编号一个行为，没配的站点返回空列表；记录每次调用。
#[derive(Default)]
pub struct FakeSearch {
    handlers: HashMap<u32, Handler>,
    calls: Mutex<Vec<(u32, String, u32)>>,
}

impl FakeSearch {
    pub fn on(
        mut self,
        src: u32,
        handler: impl Fn(&str, u32) -> Result<Vec<TorrentInfo>, SpiderError> + Send + Sync + 'static,
    ) -> Self {
        self.handlers.insert(src, Box::new(handler));
        self
    }

    pub fn calls(&self) -> Vec<(u32, String, u32)> {
        self.calls.lock().unwrap().clone()
    }

    pub fn count_of(&self, src: u32, keyword: &str) -> usize {
        self.calls().iter().filter(|(s, k, _)| *s == src && k == keyword).count()
    }
}

#[async_trait]
impl SearchGateway for FakeSearch {
    async fn search(&self, src: u32, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        self.calls.lock().unwrap().push((src, keyword.to_owned(), page));
        match self.handlers.get(&src) {
            Some(handler) => handler(keyword, page),
            None => Ok(vec![]),
        }
    }
}

/// 假磁力网关：默认「拿不到」，这正是核心库取磁力的行为。
#[derive(Default)]
pub struct FakeMagnet {
    pub answer: Option<String>,
    pub delay: Option<Duration>,
    pub calls: Mutex<Vec<(u32, String)>>,
}

impl FakeMagnet {
    pub fn returning(magnet: &str) -> Self {
        Self { answer: Some(magnet.to_owned()), ..Default::default() }
    }

    pub fn calls(&self) -> Vec<(u32, String)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl MagnetGateway for FakeMagnet {
    async fn find_magnet_url(&self, src: u32, detail_url: &str) -> Option<String> {
        self.calls.lock().unwrap().push((src, detail_url.to_owned()));
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        self.answer.clone()
    }
}

pub fn item(src: u32, title: &str) -> TorrentInfo {
    TorrentInfo { src, title: Some(title.to_owned()), ..Default::default() }
}

pub fn http_failure(status: u16) -> SpiderError {
    HttpError::status("GET", "https://example.test/search?q=x", status, "<html>blocked</html>").into()
}

/// 只暴露前 `count` 个真实站点，断言「单源失败不影响整批」时不必数 11 个状态。
pub fn sources_subset(count: usize) -> Vec<TorrentSource> {
    supported_sources().into_iter().take(count).collect()
}

pub struct Harness {
    pub search: Arc<FakeSearch>,
    pub magnet: Arc<FakeMagnet>,
    pub state: AppState,
}

impl Harness {
    pub fn new(search: FakeSearch, magnet: FakeMagnet, sources: Vec<TorrentSource>) -> Self {
        let search = Arc::new(search);
        let magnet = Arc::new(magnet);
        let state = AppState::new(search.clone(), magnet.clone(), sources);
        Self { search, magnet, state }
    }

    pub fn search(search: FakeSearch, sources: Vec<TorrentSource>) -> Self {
        Self::new(search, FakeMagnet::default(), sources)
    }

    pub fn magnet(magnet: FakeMagnet) -> Self {
        Self::new(FakeSearch::default(), magnet, sources_subset(11))
    }

    pub async fn get(&self, uri: &str) -> Reply {
        self.request(uri, None, &[]).await
    }

    /// `peer` 是 TCP 对端地址（`None` 时没有 ConnectInfo，等同于测试直接调 router）。
    pub async fn request(&self, uri: &str, peer: Option<&str>, headers: &[(&str, &str)]) -> Reply {
        let mut builder = Request::builder().uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = builder.body(Body::empty()).unwrap();
        let app = router(self.state.clone());
        let app = match peer {
            Some(peer) => app.layer(MockConnectInfo(peer.parse::<SocketAddr>().unwrap())),
            None => app,
        };
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply { status, headers, body: String::from_utf8(bytes.to_vec()).unwrap() }
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not json ({e}): {}", self.body))
    }

    pub fn code(&self) -> String {
        self.json()["code"].as_str().unwrap().to_owned()
    }

    pub fn content_type(&self) -> &str {
        self.headers.get(header::CONTENT_TYPE).map_or("", |v| v.to_str().unwrap())
    }
}

pub fn encode(value: &str) -> String {
    torrent_spider::net::encoding::url_encode(value)
}

pub fn hashed(src: u32, title: &str, hash: &str) -> TorrentInfo {
    TorrentInfo { hash: Some(hash.to_owned()), ..item(src, title) }
}

pub fn titles_of(reply: &Reply) -> Vec<String> {
    reply.json()["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap().to_owned()).collect()
}

pub fn counts_of(reply: &Reply) -> Vec<u64> {
    reply.json()["sources"].as_array().unwrap().iter().map(|s| s["count"].as_u64().unwrap()).collect()
}

pub fn queries_of(reply: &Reply) -> Vec<String> {
    reply.json()["queries"].as_array().unwrap().iter().map(|q| q.as_str().unwrap().to_owned()).collect()
}
