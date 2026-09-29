//! 违禁内容过滤：请求方是中国大陆 IP 时，丢掉命中违禁词的结果。
//!
//! 两份数据来源不同，这是刻意的：
//! - 违禁词编在二进制里（[`SensitiveLexicon::builtin`]），不联网，不存在「拉不到」的状态；
//! - 大陆 IP 段启动时联网拉取、落盘缓存、每天刷新。它变化快，失效时表现明确（`is_china` 恒为 false）。
//!
//! 拉取失败不阻断启动，依次回退到落盘缓存、内存里的旧表。这个功能是合规兜底，
//! 不是核心搜索路径；代价是 IP 段拿不到时不会过滤任何人，日志里留 WARN。

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::Serialize;
use torrent_spider::{HttpClient, TorrentInfo};

pub mod ip_range;
pub mod lexicon;

use ip_range::IpRangeSet;
use lexicon::SensitiveLexicon;

/// 17mon/china_ip_list：纯文本 CIDR 清单，每天自动更新。
pub const DEFAULT_IP_LIST_URL: &str = "https://raw.githubusercontent.com/17mon/china_ip_list/master/china_ip_list.txt";

pub const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

const CACHE_FILE: &str = "china_ip_list.txt";
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct SensitiveConfig {
    pub enabled: bool,
    /// 把所有请求方都当成大陆 IP。只用于本地自测。
    pub force_china: bool,
    pub ip_list_url: String,
    pub cache_dir: PathBuf,
    pub refresh_interval: Duration,
}

impl Default for SensitiveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            force_china: false,
            ip_list_url: DEFAULT_IP_LIST_URL.to_owned(),
            cache_dir: std::env::temp_dir().join("torrentspider-sensitive"),
            refresh_interval: DEFAULT_REFRESH_INTERVAL,
        }
    }
}

/// `/api/health` 上的诊断块：这次请求会不会被过滤，以及过滤有没有能力干活。只含聚合量。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FilterStatus {
    pub enabled: bool,
    /// 词表与 IP 段都已就绪。只有一份数据时过滤不出东西，不能报就绪。
    pub ready: bool,
    pub force_china: bool,
    pub lexicon_entries: usize,
    pub china_ranges: usize,
    pub client_ip: Option<String>,
    pub is_china: bool,
}

pub struct SensitiveFilter {
    config: SensitiveConfig,
    http: Arc<dyn HttpClient>,
    lexicon: RwLock<Arc<SensitiveLexicon>>,
    china: RwLock<Arc<IpRangeSet>>,
}

impl SensitiveFilter {
    /// 构造时就装上内置词库：它不联网，没有「启动后词表还是空的」这段窗口。
    pub fn new(config: SensitiveConfig, http: Arc<dyn HttpClient>) -> Self {
        let lexicon = if config.enabled { SensitiveLexicon::builtin().clone() } else { SensitiveLexicon::empty() };
        Self::with_lexicon(config, http, lexicon)
    }

    /// 注入一份词表，测试用：断言不该依赖真实词库里恰好有哪些词。
    pub fn with_lexicon(config: SensitiveConfig, http: Arc<dyn HttpClient>, lexicon: SensitiveLexicon) -> Self {
        Self {
            config,
            http,
            lexicon: RwLock::new(Arc::new(lexicon)),
            china: RwLock::new(Arc::new(IpRangeSet::default())),
        }
    }

    pub fn config(&self) -> &SensitiveConfig {
        &self.config
    }

    fn lexicon(&self) -> Arc<SensitiveLexicon> {
        Arc::clone(&self.lexicon.read().expect("lexicon lock"))
    }

    fn china(&self) -> Arc<IpRangeSet> {
        Arc::clone(&self.china.read().expect("china lock"))
    }

    pub fn is_ready(&self) -> bool {
        !self.lexicon().is_empty() && !self.china().is_empty()
    }

    /// 刷新大陆 IP 段：先联网拉，成功就把原文写进缓存；失败读缓存；再失败沿用当前表。
    ///
    /// 解析出零条当失败处理：把「一条都没解析出来」当成功会让过滤静默失效。
    pub async fn refresh(&self) {
        if !self.config.enabled {
            return;
        }
        let cache = self.config.cache_dir.join(CACHE_FILE);

        let downloaded =
            match tokio::time::timeout(FETCH_TIMEOUT, self.http.get_text(&self.config.ip_list_url, &[])).await {
                Ok(Ok(body)) if !body.trim().is_empty() => Some(body),
                Ok(Ok(_)) => {
                    tracing::warn!(url = %self.config.ip_list_url, "大陆 IP 段拉取结果为空");
                    None
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, "大陆 IP 段拉取失败");
                    None
                }
                Err(_) => {
                    tracing::warn!(url = %self.config.ip_list_url, "大陆 IP 段拉取超时");
                    None
                }
            };

        if let Some(body) = &downloaded
            && let Some(set) = parse_non_empty(body)
        {
            // 缓存写原文而不是解析结果：解析口径以后改了，旧原文还能重新解析
            if let Err(error) = write_cache(&cache, body).await {
                tracing::warn!(%error, path = %cache.display(), "写入 IP 段缓存失败");
            }
            self.install(set, "联网拉取");
            return;
        }

        match tokio::fs::read_to_string(&cache).await {
            Ok(body) => match parse_non_empty(&body) {
                Some(set) => self.install(set, "落盘缓存"),
                None => tracing::warn!(path = %cache.display(), "IP 段缓存解析不出任何区间，沿用当前表"),
            },
            Err(error) => tracing::warn!(
                %error,
                path = %cache.display(),
                current = self.china().len(),
                "大陆 IP 段拉取与缓存都不可用，沿用当前表"
            ),
        }
    }

    fn install(&self, set: IpRangeSet, from: &str) {
        tracing::info!(ranges = set.len(), from, "中国大陆 IP 段已加载");
        *self.china.write().expect("china lock") = Arc::new(set);
    }

    /// 起后台刷新任务：立即刷一次，之后按间隔刷。返回的句柄随服务一起结束即可。
    pub fn spawn_refresh(self: &Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        if !self.config.enabled {
            tracing::info!("违禁内容过滤已关闭（SENSITIVE_FILTER=0）");
            return None;
        }
        tracing::info!(entries = self.lexicon().len(), "违禁词表已加载（内置，不联网）");
        let filter = Arc::clone(self);
        Some(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(filter.config.refresh_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                filter.refresh().await;
            }
        }))
    }

    /// 这个请求方是不是大陆 IP。拿不到或解析不了地址时一律按「不是」处理：宁可漏拦，不可误拦。
    pub fn is_china(&self, client: Option<IpAddr>) -> bool {
        if self.config.force_china {
            return true;
        }
        match client {
            Some(ip) if self.config.enabled => self.china().contains(ip),
            _ => false,
        }
    }

    pub fn status(&self, client: Option<IpAddr>) -> FilterStatus {
        FilterStatus {
            enabled: self.config.enabled,
            ready: self.is_ready(),
            force_china: self.config.force_china,
            lexicon_entries: self.lexicon().len(),
            china_ranges: self.china().len(),
            client_ip: client.map(|ip| ip.to_string()),
            is_china: self.is_china(client),
        }
    }

    /// 丢掉命中违禁词的结果；非大陆请求方原样返回。
    ///
    /// 调用方必须在统计每源条数之前调用：被拦下的条目连数量都不能透露。
    pub fn filter(&self, items: Vec<TorrentInfo>, keyword: &str, client: Option<IpAddr>) -> Vec<TorrentInfo> {
        if !self.config.enabled || !self.is_china(client) {
            return items;
        }
        self.lexicon().filter(items, keyword)
    }
}

fn parse_non_empty(body: &str) -> Option<IpRangeSet> {
    let set = IpRangeSet::parse_list(body);
    (!set.is_empty()).then_some(set)
}

async fn write_cache(path: &std::path::Path, body: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    // 先写临时文件再改名：进程在写到一半时被杀，下次启动也不会读到半份清单
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, body).await?;
    tokio::fs::rename(&tmp, path).await
}

/// 请求方地址。
///
/// 默认只看 TCP 对端地址，客户端伪造不了。`trust_forwarded_for` 只该在反向代理后面打开：
/// 这时对端是反代自己，真实来源在 `X-Forwarded-For` 的**最后一段**（反代把它追加在末尾，
/// 前面几段可能是客户端自己塞的）。直连部署打开它，任何人加一行 XFF 就能绕过过滤。
/// 假定只有一层可信反代。
pub fn client_ip(peer: Option<SocketAddr>, forwarded_for: Option<&str>, trust_forwarded_for: bool) -> Option<IpAddr> {
    let direct = peer.map(|addr| canonical(addr.ip()));
    if !trust_forwarded_for {
        return direct;
    }
    forwarded_for
        .and_then(|value| value.rsplit(',').map(str::trim).find(|part| !part.is_empty()))
        .and_then(|last| last.parse::<IpAddr>().ok())
        .map(canonical)
        .or(direct)
}

/// 双栈监听时 IPv4 客户端会以 `::ffff:a.b.c.d` 出现，折回 IPv4 才能命中 IPv4 段。
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> Option<SocketAddr> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn peer_address_is_used_by_default_and_xff_is_ignored() {
        assert_eq!(client_ip(addr("1.2.3.4:5"), Some("8.8.8.8"), false), Some("1.2.3.4".parse().unwrap()));
        assert_eq!(client_ip(None, Some("8.8.8.8"), false), None);
    }

    #[test]
    fn trusted_xff_uses_the_last_hop() {
        let peer = addr("10.0.0.1:80");
        assert_eq!(client_ip(peer, Some("9.9.9.9, 1.2.3.4"), true), Some("1.2.3.4".parse().unwrap()));
        assert_eq!(client_ip(peer, Some("9.9.9.9, "), true), Some("9.9.9.9".parse().unwrap()));
        // XFF 缺失或畸形时退回对端地址
        assert_eq!(client_ip(peer, None, true), Some("10.0.0.1".parse().unwrap()));
        assert_eq!(client_ip(peer, Some("not-an-ip"), true), Some("10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn ipv4_mapped_peers_are_folded_to_ipv4() {
        assert_eq!(client_ip(addr("[::ffff:1.2.3.4]:80"), None, false), Some("1.2.3.4".parse().unwrap()));
        assert_eq!(client_ip(addr("[2001:db8::1]:80"), None, false), Some("2001:db8::1".parse().unwrap()));
    }
}
