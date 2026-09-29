//! `GET /api/magnet`：按详情页地址解析磁力。

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use torrent_spider::TorrentSource;
use torrent_spider::magnet::{magnet_to_hash, magnet_to_torrent_url};

use super::extract::{Params, params};
use super::params::{normalize_incoming_magnet, parse_magnet_src, require_detail_url};
use super::{ApiError, codes};
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct MagnetResponse {
    pub src: u32,
    pub detail_url: String,
    pub magnet_url: Option<String>,
    pub torrent_url: Option<String>,
    pub hash: Option<String>,
    pub resolved: bool,
}

/// `GET /api/magnet?src=&detail_url=[&magnet_url=]`
///
/// 带了合法的 `magnet_url` 就直接回填、不抓详情页；否则去抓，抓不到是 404 而不是 5xx：
/// 「站点没给磁力」不是服务端故障，报 5xx 会让用户去重试，而重试不会改变结果。
pub async fn magnet_url(State(state): State<AppState>, query: Params) -> Result<Json<MagnetResponse>, ApiError> {
    let params = params(query)?;
    let src = parse_magnet_src(params.get("src").map(String::as_str), &state.sources)?;
    let detail_url = require_detail_url(params.get("detail_url").map(String::as_str))?;

    let magnet_url = match normalize_incoming_magnet(params.get("magnet_url").map(String::as_str)) {
        Some(known) => Some(known),
        None => fetch_magnet(&state, src, &detail_url, state.magnet_timeout).await,
    };
    let Some(magnet_url) = magnet_url else {
        return Err(ApiError {
            src: Some(src),
            detail_url: Some(detail_url),
            ..ApiError::new(
                StatusCode::NOT_FOUND,
                codes::MAGNET_UNAVAILABLE,
                format!(
                    "来源「{}」没有提供磁力链接。该站点可能已改版，或这条资源已下架。",
                    title_of(&state.sources, src)
                ),
            )
        });
    };

    Ok(Json(MagnetResponse {
        src,
        detail_url,
        torrent_url: magnet_to_torrent_url(&magnet_url),
        hash: magnet_to_hash(&magnet_url),
        magnet_url: Some(magnet_url),
        resolved: true,
    }))
}

async fn fetch_magnet(state: &AppState, src: u32, detail_url: &str, timeout: Duration) -> Option<String> {
    let found = tokio::time::timeout(timeout, state.magnet.find_magnet_url(src, detail_url)).await.ok()??;
    let found = found.trim();
    found.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("magnet:")).then(|| found.to_owned())
}

fn title_of(sources: &[TorrentSource], src: u32) -> &str {
    sources.iter().find(|s| s.src == src).map_or("未知来源", |s| s.title.as_str())
}
