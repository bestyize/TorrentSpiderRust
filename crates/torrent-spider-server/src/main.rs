use std::io::IsTerminal;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use torrent_spider::{ReqwestHttpClient, TorrentSpiderClient};
use torrent_spider_server::{AppState, ClientGateway, SensitiveFilter, ServerConfig, router};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "\
torrent-spider-server：TorrentSpider 的 HTTP 服务与网页

用法：torrent-spider-server [--version | --help]

配置全部通过环境变量：
  PORT                   监听端口，默认 55555
  HOST                   监听地址，默认 0.0.0.0
  TORRENT_PROXY          出站代理，host:port 或 socks5://host:port
  TRUST_FORWARDED_FOR    部署在反向代理后面时设 1
  SENSITIVE_FILTER       违禁内容过滤总开关，默认 1
  SENSITIVE_ASSUME_CHINA 1 = 把所有请求方当成大陆 IP，只用于本地自测
  SENSITIVE_CACHE_DIR    大陆 IP 段的缓存目录
  SENSITIVE_IP_LIST_URL  大陆 IP 段来源
  RUST_LOG               日志级别，默认 info,tower_http=info
";

#[tokio::main]
async fn main() -> ExitCode {
    // 部署脚本在替换二进制之前会先跑一次 --version，确认新文件真能在这台机器上执行
    if let Some(arg) = std::env::args().nth(1) {
        return match arg.as_str() {
            "-V" | "--version" => {
                println!("torrent-spider-server {}", env!("CARGO_PKG_VERSION"));
                ExitCode::SUCCESS
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                ExitCode::SUCCESS
            }
            _ => {
                eprintln!("未知参数：{arg}。配置通过环境变量传入，见 --help");
                ExitCode::FAILURE
            }
        };
    }

    // 交给 systemd 时输出进 journald，颜色控制符只会变成乱码
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")))
        .with_ansi(std::io::stdout().is_terminal())
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
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("服务异常退出：{e}"))?;
    tracing::info!("服务已退出");
    Ok(())
}

/// Ctrl-C 或 SIGTERM（systemd 停服务时发的）都走优雅退出：不再接新连接，等进行中的请求处理完。
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "无法监听 SIGTERM，只能用 Ctrl-C 优雅退出");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("收到退出信号，等待进行中的请求完成");
}
