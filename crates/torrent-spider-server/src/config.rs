//! 从环境变量读取启动配置。
//!
//! 端口、监听地址、代理写错直接启动失败，不悄悄回落到默认值：「看起来起来了」比启动失败更难排查。
//! 违禁过滤的开关则相反，取值怪异时按默认处理，见 [`env_flag`]。

use std::net::{IpAddr, SocketAddr};

use crate::DEFAULT_PORT;
use crate::sensitive::SensitiveConfig;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    /// 出站代理的完整地址，如 `http://127.0.0.1:7897`。
    pub proxy: Option<String>,
    /// 是否信任反代给的 `X-Forwarded-For`，见 [`crate::sensitive::client_ip`]。
    pub trust_forwarded_for: bool,
    pub sensitive: SensitiveConfig,
}

impl ServerConfig {
    /// 读取 `PORT`、`HOST`、`TORRENT_PROXY`、`TRUST_FORWARDED_FOR` 与 `SENSITIVE_*`。
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            addr: SocketAddr::new(env_host()?, env_port()?),
            proxy: env_proxy()?,
            trust_forwarded_for: env_flag("TRUST_FORWARDED_FOR", false),
            sensitive: sensitive_config(),
        })
    }
}

fn sensitive_config() -> SensitiveConfig {
    let defaults = SensitiveConfig::default();
    SensitiveConfig {
        enabled: env_flag("SENSITIVE_FILTER", true),
        force_china: env_flag("SENSITIVE_ASSUME_CHINA", false),
        ip_list_url: env_var("SENSITIVE_IP_LIST_URL").unwrap_or(defaults.ip_list_url),
        cache_dir: env_var("SENSITIVE_CACHE_DIR").map_or(defaults.cache_dir, Into::into),
        refresh_interval: defaults.refresh_interval,
    }
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

fn env_port() -> Result<u16, String> {
    match env_var("PORT") {
        None => Ok(DEFAULT_PORT),
        Some(raw) => raw
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or_else(|| format!("环境变量 PORT 必须是 1 到 65535 之间的整数，收到：{raw}")),
    }
}

fn env_host() -> Result<IpAddr, String> {
    match env_var("HOST") {
        None => Ok(IpAddr::from([0, 0, 0, 0])),
        Some(raw) => raw.parse().map_err(|_| format!("环境变量 HOST 必须是 IP 地址，收到：{raw}")),
    }
}

/// `TORRENT_PROXY`：`host:port`（按 HTTP 代理处理）或带 scheme 的完整地址，如 `socks5://127.0.0.1:7897`。
fn env_proxy() -> Result<Option<String>, String> {
    let Some(raw) = env_var("TORRENT_PROXY") else {
        return Ok(None);
    };
    if raw.contains("://") {
        return Ok(Some(raw));
    }
    let valid =
        raw.rsplit_once(':').is_some_and(|(host, port)| !host.is_empty() && port.parse::<u16>().is_ok_and(|p| p != 0));
    if !valid {
        return Err(format!("TORRENT_PROXY 需要写成 host:port，例如 127.0.0.1:7897，收到：{raw}"));
    }
    Ok(Some(format!("http://{raw}")))
}

/// 开关类环境变量：`1/true/yes/on` 开、`0/false/no/off` 关，其它值按默认。
///
/// 与 `PORT` 的严格不同：这几个开关与合规相关，取值怪异时退回默认比让服务起不来更合适。
fn env_flag(name: &str, default: bool) -> bool {
    match std::env::var(name).map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Ok("1" | "true" | "yes" | "on") => true,
        Ok("0" | "false" | "no" | "off") => false,
        _ => default,
    }
}
