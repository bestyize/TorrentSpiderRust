//! TorrentSpider 的 HTTP 服务：JSON 接口 + 零构建步骤的静态单页。
//!
//! - [`api`]：各个 JSON 端点；
//! - [`sensitive`]：违禁内容过滤（大陆 IP 判定 + 违禁词表）；
//! - [`gateway`]：接口层依赖的核心库能力，收窄成 trait 方便测试注入；
//! - `web`：编译期嵌入的静态页；`config`：从环境变量读取启动配置。

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use torrent_spider::TorrentSource;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::SizeAbove;
use tower_http::cors::{Any as AnyOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

pub mod api;
mod config;
pub mod gateway;
pub mod sensitive;
mod web;

pub use config::ServerConfig;
pub use gateway::{ClientGateway, MagnetGateway, SearchGateway};
pub use sensitive::{SensitiveConfig, SensitiveFilter};

/// 默认端口，对外契约的一部分。
pub const DEFAULT_PORT: u16 = 55555;

/// 单个源一次搜索的上限；聚合时每个源单独计时。
pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(25);

/// 单次磁力解析的上限。比搜索短：用户是点着按钮在等的。
pub const MAGNET_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct AppState {
    pub search: Arc<dyn SearchGateway>,
    pub magnet: Arc<dyn MagnetGateway>,
    pub sources: Arc<[TorrentSource]>,
    pub search_timeout: Duration,
    pub magnet_timeout: Duration,
    /// 违禁内容过滤；`None` 表示不启用（测试默认如此，不碰网络与数据）。
    pub sensitive: Option<Arc<SensitiveFilter>>,
    /// 是否信任反代给的 `X-Forwarded-For`。只在反向代理后面打开，见 [`sensitive::client_ip`]。
    pub trust_forwarded_for: bool,
}

impl AppState {
    pub fn new(search: Arc<dyn SearchGateway>, magnet: Arc<dyn MagnetGateway>, sources: Vec<TorrentSource>) -> Self {
        Self {
            search,
            magnet,
            sources: sources.into(),
            search_timeout: SEARCH_TIMEOUT,
            magnet_timeout: MAGNET_TIMEOUT,
            sensitive: None,
            trust_forwarded_for: false,
        }
    }

    #[must_use]
    pub fn with_sensitive(mut self, filter: Arc<SensitiveFilter>, trust_forwarded_for: bool) -> Self {
        self.sensitive = Some(filter);
        self.trust_forwarded_for = trust_forwarded_for;
        self
    }
}

/// 全部路由与中间件。
///
/// 未知路径一律 404 + JSON 错误体，刻意不做 SPA 兜底：兜底会让拼错的接口
/// 返回一页 HTML + 200，前端在 JSON 解析上炸掉，错误信息指向别处。
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(api::health))
        .route("/api/sources", get(api::sources))
        .route("/api/search", get(api::search))
        .route("/api/magnet", get(api::magnet_url))
        .merge(web::routes())
        .fallback(not_found)
        .with_state(state)
        .layer(
            CorsLayer::new().allow_origin(AnyOrigin).allow_methods([Method::GET]).allow_headers([header::CONTENT_TYPE]),
        )
        .layer(CompressionLayer::new().compress_when(SizeAbove::new(1024)))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(CatchPanicLayer::custom(internal_error))
        .layer(TraceLayer::new_for_http())
}

async fn not_found() -> api::ApiError {
    api::ApiError::new(StatusCode::NOT_FOUND, api::codes::NOT_FOUND, "没有这个路径。")
}

/// 走到这里说明是真 bug。只返回通用信息，不把内部细节泄给调用方。
fn internal_error(panic: Box<dyn Any + Send + 'static>) -> Response {
    let detail = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("unknown panic");
    tracing::error!(panic = detail, "未处理的服务端异常");
    api::ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, api::codes::INTERNAL_ERROR, "服务端内部错误").into_response()
}
