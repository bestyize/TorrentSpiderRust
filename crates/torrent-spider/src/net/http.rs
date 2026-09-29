//! HTTP 抽象层。
//!
//! 解析器只依赖 [`HttpClient`] 这个 trait，宿主可以注入自己的实现
//! （统一代理、连接池、日志）；测试注入假实现，不碰网络。

use std::fmt;

use async_trait::async_trait;

/// 一个请求头，`(名, 值)`。
pub type Header<'a> = (&'a str, &'a str);

/// `HEAD` 请求的结果，只保留判断「是不是种子文件」需要的部分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadResponse {
    pub status: u16,
    pub content_type: Option<String>,
}

#[async_trait]
pub trait HttpClient: Send + Sync {
    /// `GET` 并返回响应体文本；非 2xx 返回 [`HttpError`]。
    async fn get_text(&self, url: &str, headers: &[Header<'_>]) -> Result<String, HttpError>;

    /// `POST` 一段文本并返回响应体文本；非 2xx 返回 [`HttpError`]。
    async fn post_text(&self, url: &str, body: &str, headers: &[Header<'_>]) -> Result<String, HttpError>;

    /// `HEAD`，任何状态码都算成功返回，由调用方判断。
    async fn head(&self, url: &str) -> Result<HeadResponse, HttpError>;
}

/// 一次失败的 HTTP 请求：网络层失败（`status` 为空）或非 2xx 响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpError {
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    /// 响应体开头的一小段，已压成单行。
    pub snippet: String,
    /// 网络层错误描述（超时、DNS、TLS 等）。
    pub cause: Option<String>,
    pub timed_out: bool,
    /// 403 且响应是 Cloudflare 验证页。按完整响应体判断，特征不一定落在 `snippet` 里。
    pub cloudflare_challenge: bool,
}

const CLOUDFLARE_CHALLENGE: &str = "站点启用了 Cloudflare 浏览器验证，纯 HTTP 请求当前无法直接通过";

impl HttpError {
    pub fn status(method: &str, url: &str, status: u16, body: &str) -> Self {
        Self {
            method: method.to_owned(),
            url: url.to_owned(),
            status: Some(status),
            snippet: to_snippet(body, 280),
            cause: None,
            timed_out: false,
            cloudflare_challenge: status == 403 && looks_like_cloudflare(body),
        }
    }

    pub fn transport(method: &str, url: &str, cause: impl fmt::Display, timed_out: bool) -> Self {
        Self {
            method: method.to_owned(),
            url: url.to_owned(),
            status: None,
            snippet: String::new(),
            cause: Some(cause.to_string()),
            timed_out,
            cloudflare_challenge: false,
        }
    }

    /// 简短描述，不含响应体：`GET https://x -> HTTP 403`。
    pub fn summary(&self) -> String {
        let reason = match (self.status, self.timed_out, &self.cause) {
            (Some(status), _, _) if self.cloudflare_challenge => format!("HTTP {status}（{CLOUDFLARE_CHALLENGE}）"),
            (Some(status), _, _) => format!("HTTP {status}"),
            (None, true, _) => "请求超时".to_owned(),
            (None, false, Some(cause)) => cause.clone(),
            (None, false, None) => "请求失败".to_owned(),
        };
        format!("{} {} -> {reason}", self.method, self.url)
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())?;
        if !self.snippet.is_empty() && !self.cloudflare_challenge {
            write!(f, " | body={}", self.snippet)?;
        }
        Ok(())
    }
}

impl std::error::Error for HttpError {}

fn looks_like_cloudflare(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    ["just a moment", "cloudflare", "cf-chl", "attention required"].iter().any(|marker| body.contains(marker))
}

fn to_snippet(body: &str, limit: usize) -> String {
    body.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_error_message_contains_status_and_snippet() {
        let error = HttpError::status("GET", "https://x/s", 503, "<html>\n  down   now </html>");
        assert_eq!(error.to_string(), "GET https://x/s -> HTTP 503 | body=<html> down now </html>");
        assert_eq!(error.summary(), "GET https://x/s -> HTTP 503");
    }

    #[test]
    fn cloudflare_challenge_is_explained() {
        let error = HttpError::status("GET", "https://x", 403, "<title>Just a moment...</title>");
        assert!(error.to_string().contains("Cloudflare"), "{error}");
        assert!(error.summary().starts_with("GET https://x -> HTTP 403（"), "{}", error.summary());
        // 特征在 280 字之后也要认出来
        let late = format!("{}<title>Attention Required! | Cloudflare</title>", " <!-- x -->".repeat(60));
        assert!(HttpError::status("GET", "https://x", 403, &late).cloudflare_challenge);
        assert!(!HttpError::status("GET", "https://x", 503, "cloudflare").cloudflare_challenge);
        let plain = HttpError::status("GET", "https://x", 403, "forbidden");
        assert!(!plain.to_string().contains("Cloudflare"));
    }

    #[test]
    fn transport_errors_describe_cause() {
        let timeout = HttpError::transport("POST", "https://x", "deadline", true);
        assert_eq!(timeout.summary(), "POST https://x -> 请求超时");
        let dns = HttpError::transport("GET", "https://x", "dns error", false);
        assert_eq!(dns.to_string(), "GET https://x -> dns error");
    }

    #[test]
    fn snippet_is_truncated() {
        let error = HttpError::status("GET", "u", 500, &"a".repeat(1000));
        assert_eq!(error.snippet.len(), 280);
    }
}
