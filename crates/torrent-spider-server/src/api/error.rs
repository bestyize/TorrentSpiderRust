//! 错误响应与错误码。

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// 错误码是对外契约的一部分，改动等于破坏兼容。
pub mod codes {
    pub const MISSING_KEYWORD: &str = "MISSING_KEYWORD";
    pub const INVALID_PAGE: &str = "INVALID_PAGE";
    pub const INVALID_SRC: &str = "INVALID_SRC";
    pub const INVALID_FILTER: &str = "INVALID_FILTER";
    pub const INVALID_OPTIMIZE: &str = "INVALID_OPTIMIZE";
    pub const MISSING_URL: &str = "MISSING_URL";
    pub const INVALID_URL: &str = "INVALID_URL";
    pub const INVALID_QUERY: &str = "INVALID_QUERY";
    pub const MAGNET_UNAVAILABLE: &str = "MAGNET_UNAVAILABLE";
    pub const ALL_SOURCES_FAILED: &str = "ALL_SOURCES_FAILED";
    pub const NOT_FOUND: &str = "NOT_FOUND";
    pub const INTERNAL_ERROR: &str = "INTERNAL_ERROR";
}

/// 错误响应：`{code, message, src, detail_url}`。
#[derive(Debug, Serialize)]
pub struct ApiError {
    #[serde(skip)]
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub src: Option<u32>,
    pub detail_url: Option<String>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), src: None, detail_url: None }
    }

    pub(crate) fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self)).into_response()
    }
}
