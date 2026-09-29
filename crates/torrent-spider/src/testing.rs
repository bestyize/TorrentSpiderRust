//! 测试工具：按 URL 路由的假 HTTP 客户端与真实格式的固定哈希。
//!
//! 只在本 crate 的测试里和 `testing` feature 打开时编译，下游 crate 在 dev-dependencies 里
//! 打开这个 feature 即可复用，不必各写一份假客户端。

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::net::{HeadResponse, Header, HttpClient, HttpError};
use crate::site::SiteEndpoint;

/// 40 位十六进制，大写。假哈希（长度、字符集不对）会掩盖「哈希完全不校验」这类问题。
pub const HEX: &str = "E90710273A1E578835CAC80D3E4B3E56A2755663";
pub const HEX_LOWER: &str = "e90710273a1e578835cac80d3e4b3e56a2755663";
/// [`HEX`] 的等价 Base32 写法。
pub const BASE32: &str = "5EDRAJZ2DZLYQNOKZAGT4SZ6K2RHKVTD";
/// 第二个互不相同的哈希，用于需要区分多行的 fixture。
pub const OTHER_HEX: &str = "DD8255ECDC7CA55FB0BBF81323D87062DB1F6D1C";
pub const OTHER_BASE32: &str = "3WBFL3G4PSSV7MF37AJSHWDQMLNR63I4";

pub fn magnet(hash: &str) -> String {
    format!("magnet:?xt=urn:btih:{hash}")
}

pub fn magnet_named(hash: &str, display_name: &str) -> String {
    format!("magnet:?xt=urn:btih:{hash}&dn={display_name}")
}

/// 首选缓存源上的 `.torrent` 地址；跟着配置走，不在测试里写死域名。
pub fn torrent_url(hash: &str) -> String {
    crate::magnet::torrent_file::DEFAULT_SOURCES[0].url_for(hash)
}

/// 指向测试桩的站点入口。
pub fn endpoint(host: &str) -> SiteEndpoint {
    SiteEndpoint::new(host)
}

/// 假客户端收到的一次请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub body: String,
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

struct Route {
    exact: bool,
    pattern: String,
    status: u16,
    body: String,
}

/// 按 URL 路由的假站点，并记录每次请求。
///
/// 路由按注册顺序匹配，先注册的优先；没命中的请求返回 404。按前缀而不是全等匹配：
/// 测试盯的是解析逻辑，URL 里的分页参数调整不该让测试跟着改。
#[derive(Default)]
pub struct FakeHttp {
    offline: bool,
    routes: Mutex<Vec<Route>>,
    heads: Mutex<Vec<(String, HeadResponse)>>,
    requests: Mutex<Vec<Request>>,
}

impl FakeHttp {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 任何 GET/POST 都返回 `body`。
    pub fn serving(body: &str) -> Arc<Self> {
        Self::new().route("", body)
    }

    /// 所有请求都 404。
    pub fn not_found() -> Arc<Self> {
        Self::new()
    }

    /// 所有请求都在网络层失败（连不上），模拟断网。
    pub fn offline() -> Arc<Self> {
        Arc::new(Self { offline: true, ..Self::default() })
    }

    /// URL 以 `prefix` 开头即命中。
    pub fn route(self: Arc<Self>, prefix: &str, body: &str) -> Arc<Self> {
        self.push(false, prefix, 200, body);
        self
    }

    /// URL 完全相等才命中；用于区分共用前缀的多个地址。
    pub fn exact(self: Arc<Self>, url: &str, body: &str) -> Arc<Self> {
        self.push(true, url, 200, body);
        self
    }

    pub fn route_with_status(self: Arc<Self>, prefix: &str, status: u16, body: &str) -> Arc<Self> {
        self.push(false, prefix, status, body);
        self
    }

    pub fn head_route(self: Arc<Self>, prefix: &str, response: HeadResponse) -> Arc<Self> {
        self.heads.lock().unwrap().push((prefix.to_owned(), response));
        self
    }

    /// 替换某个前缀的响应体（没有就新增）。用于测试中途让站点换一份内容。
    pub fn set_route(&self, prefix: &str, body: &str) {
        self.routes.lock().unwrap().retain(|r| r.exact || r.pattern != prefix);
        self.push(false, prefix, 200, body);
    }

    fn push(&self, exact: bool, pattern: &str, status: u16, body: &str) {
        self.routes.lock().unwrap().push(Route { exact, pattern: pattern.to_owned(), status, body: body.to_owned() });
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    pub fn requested_urls(&self) -> Vec<String> {
        self.requests().into_iter().map(|r| r.url).collect()
    }

    pub fn last_request(&self) -> Option<Request> {
        self.requests().pop()
    }

    fn record(&self, method: &str, url: &str, body: &str, headers: &[Header<'_>]) {
        self.requests.lock().unwrap().push(Request {
            method: method.to_owned(),
            url: url.to_owned(),
            body: body.to_owned(),
            headers: headers.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
        });
    }

    fn respond(&self, method: &str, url: &str) -> Result<String, HttpError> {
        if self.offline {
            return Err(HttpError::transport(method, url, "网络不通", false));
        }
        let routes = self.routes.lock().unwrap();
        let hit = routes.iter().find(|r| if r.exact { url == r.pattern } else { url.starts_with(&r.pattern) });
        match hit {
            Some(route) if (200..300).contains(&route.status) => Ok(route.body.clone()),
            Some(route) => Err(HttpError::status(method, url, route.status, &route.body)),
            None => Err(HttpError::status(method, url, 404, "<html><body>not found</body></html>")),
        }
    }
}

#[async_trait]
impl HttpClient for FakeHttp {
    async fn get_text(&self, url: &str, headers: &[Header<'_>]) -> Result<String, HttpError> {
        self.record("GET", url, "", headers);
        self.respond("GET", url)
    }

    async fn post_text(&self, url: &str, body: &str, headers: &[Header<'_>]) -> Result<String, HttpError> {
        self.record("POST", url, body, headers);
        self.respond("POST", url)
    }

    async fn head(&self, url: &str) -> Result<HeadResponse, HttpError> {
        self.record("HEAD", url, "", &[]);
        if self.offline {
            return Err(HttpError::transport("HEAD", url, "网络不通", false));
        }
        let heads = self.heads.lock().unwrap();
        Ok(heads
            .iter()
            .find(|(prefix, _)| url.starts_with(prefix))
            .map(|(_, response)| response.clone())
            .unwrap_or(HeadResponse { status: 404, content_type: Some("text/html".into()) }))
    }
}
