//! 静态单页，编译期嵌进二进制：部署只需要一个可执行文件。

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;

use crate::AppState;

const INDEX_HTML: &str = include_str!("../static/index.html");
const STYLES_CSS: &str = include_str!("../static/styles.css");
const APP_JS: &str = include_str!("../static/app.js");

pub fn routes() -> Router<AppState> {
    Router::new()
        // 根路径显式 302，而不是把 index.html 当兜底：见 `crate::router` 的说明
        .route("/", get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/index.html")]) }))
        .route("/index.html", get(|| asset("text/html; charset=utf-8", INDEX_HTML)))
        .route("/styles.css", get(|| asset("text/css; charset=utf-8", STYLES_CSS)))
        .route("/app.js", get(|| asset("text/javascript; charset=utf-8", APP_JS)))
}

async fn asset(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "no-cache")], body)
}
