//! 基于 reqwest 的默认 [`HttpClient`] 实现。

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::CONTENT_TYPE;

use super::{HeadResponse, Header, HttpClient, HttpError};

const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

#[derive(Debug, Clone)]
pub struct ReqwestHttpClient {
    inner: reqwest::Client,
}

impl ReqwestHttpClient {
    pub fn builder() -> ReqwestHttpClientBuilder {
        ReqwestHttpClientBuilder::default()
    }

    /// 包装一个现成的 reqwest 客户端，超时 / 代理等策略完全由宿主决定。
    pub fn from_client(inner: reqwest::Client) -> Self {
        Self { inner }
    }

    async fn send(&self, method: &str, request: reqwest::RequestBuilder, url: &str) -> Result<String, HttpError> {
        let response = request.send().await.map_err(|e| transport(method, url, &e))?;
        let status = response.status();
        let body = response.text().await.map_err(|e| transport(method, url, &e))?;
        if !status.is_success() {
            return Err(HttpError::status(method, url, status.as_u16(), &body));
        }
        Ok(body)
    }
}

fn transport(method: &str, url: &str, error: &reqwest::Error) -> HttpError {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        message.push_str(": ");
        message.push_str(&inner.to_string());
        source = inner.source();
    }
    HttpError::transport(method, url, message, error.is_timeout())
}

fn apply_headers(mut request: reqwest::RequestBuilder, headers: &[Header<'_>]) -> reqwest::RequestBuilder {
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn get_text(&self, url: &str, headers: &[Header<'_>]) -> Result<String, HttpError> {
        let request = apply_headers(self.inner.get(url), headers);
        self.send("GET", request, url).await
    }

    async fn post_text(&self, url: &str, body: &str, headers: &[Header<'_>]) -> Result<String, HttpError> {
        let request = apply_headers(self.inner.post(url).body(body.to_owned()), headers);
        self.send("POST", request, url).await
    }

    async fn head(&self, url: &str) -> Result<HeadResponse, HttpError> {
        let response = self.inner.head(url).send().await.map_err(|e| transport("HEAD", url, &e))?;
        let content_type = response.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_owned);
        Ok(HeadResponse { status: response.status().as_u16(), content_type })
    }
}

/// 默认值：请求 15s、连接 10s 超时，跟随重定向，浏览器 UA。
#[derive(Debug, Clone)]
pub struct ReqwestHttpClientBuilder {
    timeout: Duration,
    connect_timeout: Duration,
    proxy: Option<String>,
    user_agent: String,
}

impl Default for ReqwestHttpClientBuilder {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(15),
            connect_timeout: Duration::from_secs(10),
            proxy: None,
            user_agent: DEFAULT_USER_AGENT.to_owned(),
        }
    }
}

impl ReqwestHttpClientBuilder {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// 所有请求走这个代理，例如 `http://127.0.0.1:7897` 或 `socks5://127.0.0.1:7897`。
    pub fn proxy(mut self, url: impl Into<String>) -> Self {
        self.proxy = Some(url.into());
        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn build(self) -> Result<ReqwestHttpClient, reqwest::Error> {
        let mut builder = reqwest::Client::builder()
            .timeout(self.timeout)
            .connect_timeout(self.connect_timeout)
            .user_agent(self.user_agent)
            .redirect(reqwest::redirect::Policy::limited(10));
        if let Some(proxy) = self.proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
        Ok(ReqwestHttpClient { inner: builder.build()? })
    }
}
