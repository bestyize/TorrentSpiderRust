//! 服务元信息：探活与站点清单。

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use torrent_spider::TorrentSource;

use super::ClientIp;
use crate::AppState;
use crate::sensitive::FilterStatus;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    /// 违禁过滤的诊断块；没装过滤器时为 null。
    pub filter: Option<FilterStatus>,
}

#[derive(Debug, Serialize)]
pub struct SourcesResponse<'a> {
    pub sources: &'a [TorrentSource],
}

/// `GET /api/health`。装了过滤器时带上这次请求的判定结果，供排查「过滤怎么没生效」。
pub async fn health(State(state): State<AppState>, ClientIp(client): ClientIp) -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok", filter: state.sensitive.as_ref().map(|f| f.status(client)) })
}

pub async fn sources(State(state): State<AppState>) -> Response {
    Json(SourcesResponse { sources: &state.sources }).into_response()
}
