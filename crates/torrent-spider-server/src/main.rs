use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use torrent_spider::{ReqwestHttpClient, TorrentSpiderClient};
use torrent_spider_server::{AppState, ClientGateway, SensitiveFilter, ServerConfig, router};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")))
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("启动失败：{message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let config = ServerConfig::from_env()?;

    let mut http = ReqwestHttpClient::builder();
    if let Some(proxy) = &config.proxy {
        http = http.proxy(proxy.clone());
    }
    let http = Arc::new(http.build().map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?);
    let client = Arc::new(TorrentSpiderClient::new(http.clone()));
    let gateway = Arc::new(ClientGateway(client.clone()));

    // 拉 IP 段复用同一个 HTTP 客户端：超时与出站代理只在一处配置
    let sensitive = Arc::new(SensitiveFilter::new(config.sensitive.clone(), http));
    let _refresh = sensitive.spawn_refresh();
    let state = AppState::new(gateway.clone(), gateway, client.sources().to_vec())
        .with_sensitive(sensitive.clone(), config.trust_forwarded_for);

    let addr = config.addr;
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| format!("无法监听 {addr}：{e}"))?;

    println!("TorrentSpider 服务已启动：http://localhost:{}（监听 {addr}）", addr.port());
    if let Some(proxy) = &config.proxy {
        println!("出站代理：{proxy}（来自 TORRENT_PROXY）");
    }
    if sensitive.config().force_china {
        println!("违禁过滤：所有请求方都按大陆处理（SENSITIVE_ASSUME_CHINA=1，仅用于本地自测）");
    }
    if config.trust_forwarded_for {
        println!(
            "违禁过滤：只认反代给的 X-Forwarded-For（TRUST_FORWARDED_FOR=1）。直连部署不要开，客户端加一行该头就能绕过过滤"
        );
    }

    axum::serve(listener, router(state).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| format!("服务异常退出：{e}"))
}
