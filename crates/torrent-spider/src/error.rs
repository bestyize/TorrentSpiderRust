use std::time::Duration;

use crate::net::HttpError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpiderError {
    #[error(transparent)]
    Http(#[from] HttpError),

    #[error("请求超时（{}s）", .0.as_secs())]
    Timeout(Duration),

    #[error("未知的站点编号：{0}")]
    UnknownSource(u32),
}

impl SpiderError {
    /// 一行短描述，不含响应体，适合放进接口返回的 `error` 字段。
    pub fn describe(&self) -> String {
        match self {
            SpiderError::Http(error) => error.summary(),
            SpiderError::Timeout(_) => "请求超时".to_owned(),
            other => other.to_string(),
        }
    }
}
