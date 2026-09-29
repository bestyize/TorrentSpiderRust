//! 请求提取：查询串与请求方地址。

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use axum::extract::rejection::QueryRejection;
use axum::extract::{ConnectInfo, FromRequestParts, Query};
use axum::http::request::Parts;

use super::{ApiError, codes};
use crate::AppState;

/// 查询串解析失败时不直接交给 axum 回 400 文本，而是转成统一的 JSON 错误体。
pub(crate) type Params = Result<Query<HashMap<String, String>>, QueryRejection>;

pub(crate) fn params(query: Params) -> Result<HashMap<String, String>, ApiError> {
    query.map(|Query(map)| map).map_err(|rejection| ApiError::bad_request(codes::INVALID_QUERY, rejection.body_text()))
}

/// 请求方地址，用来判定是不是大陆 IP。取法见 [`crate::sensitive::client_ip`]。
///
/// 服务没有通过 `into_make_service_with_connect_info` 启动时（例如测试直接调 router）对端地址为空。
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub Option<IpAddr>);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        // 走 ConnectInfo 自己的提取器：它同时认测试里用 MockConnectInfo 注入的地址
        let peer = ConnectInfo::<SocketAddr>::from_request_parts(parts, state).await.ok().map(|info| info.0);
        let forwarded = parts.headers.get("x-forwarded-for").and_then(|v| v.to_str().ok());
        Ok(Self(crate::sensitive::client_ip(peer, forwarded, state.trust_forwarded_for)))
    }
}
