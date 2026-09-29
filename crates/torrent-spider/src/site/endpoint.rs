//! 站点入口地址。
//!
//! 这些站点换域名、被墙、开镜像都很频繁。域名从解析代码里抽出来之后，
//! 换域名只改配置，不必动解析逻辑。

use crate::site::TorrentSrc;

/// 单个站点的入口。
///
/// - `mirrors` 默认留空：没实测过的镜像写进来只会制造「配了就能用」的错觉。
/// - `request_host` 用于「前端只是个壳、接口挂在别处」的站点（磁力多）。
///   `host` 仍是对外展示的地址，`request_host` 才是实际请求的目标。
/// - `referer` 是站点强制的来源校验头。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteEndpoint {
    hosts: Vec<String>,
    request_base: String,
    referer: Option<String>,
}

impl SiteEndpoint {
    pub fn new(host: &str) -> Self {
        Self::with_mirrors(host, &[])
    }

    pub fn with_mirrors(host: &str, mirrors: &[&str]) -> Self {
        let mut hosts: Vec<String> = Vec::new();
        for candidate in std::iter::once(host).chain(mirrors.iter().copied()) {
            let cleaned = candidate.trim().trim_end_matches('/').to_owned();
            if !cleaned.is_empty() && !hosts.contains(&cleaned) {
                hosts.push(cleaned);
            }
        }
        assert!(!hosts.is_empty(), "SiteEndpoint 需要至少一个 host");
        let request_base = hosts[0].clone();
        Self { hosts, request_base, referer: None }
    }

    /// 实际请求改打另一个域名，对外展示仍用 `host`。
    #[must_use]
    pub fn request_host(mut self, request_host: &str) -> Self {
        let cleaned = request_host.trim().trim_end_matches('/');
        if !cleaned.is_empty() {
            self.request_base = cleaned.to_owned();
        }
        self
    }

    #[must_use]
    pub fn referer(mut self, referer: &str) -> Self {
        self.referer = Some(referer.to_owned());
        self
    }

    /// 对外展示的首选地址。
    pub fn host(&self) -> &str {
        &self.hosts[0]
    }

    /// 首选在前、去重后的全部 host。
    pub fn hosts(&self) -> &[String] {
        &self.hosts
    }

    pub fn request_base(&self) -> &str {
        &self.request_base
    }

    pub fn referer_header(&self) -> Option<&str> {
        self.referer.as_deref()
    }

    /// 按路径拼出实际请求地址；已经是绝对地址时原样返回。
    pub fn url(&self, path: &str) -> String {
        join(&self.request_base, path)
    }

    /// 按路径拼出对用户展示的地址，始终基于 `host`。
    ///
    /// 存进 `detail_url` 的应该是这个：接口域名是内部细节，
    /// 通常不可直接访问，而且会随投放轮换。
    pub fn public_url(&self, path: &str) -> String {
        join(self.host(), path)
    }

    /// 按路径拼出全部候选地址，首选在前。
    pub fn candidates(&self, path: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for base in std::iter::once(&self.request_base).chain(self.hosts.iter().skip(1)) {
            let url = join(base, path);
            if !out.contains(&url) {
                out.push(url);
            }
        }
        out
    }
}

fn join(base: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_owned();
    }
    if path.starts_with("//") {
        let scheme = base.split_once("://").map_or("https", |(s, _)| s);
        return format!("{scheme}:{path}");
    }
    if path.is_empty() || path.starts_with('/') || path.starts_with('?') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

/// 按站点提供入口地址。替换实现即可整体切换域名策略（镜像站、测试桩）。
pub trait SiteEndpointProvider: Send + Sync {
    fn endpoint_for(&self, source: TorrentSrc) -> SiteEndpoint;
}

impl<F> SiteEndpointProvider for F
where
    F: Fn(TorrentSrc) -> SiteEndpoint + Send + Sync,
{
    fn endpoint_for(&self, source: TorrentSrc) -> SiteEndpoint {
        self(source)
    }
}

/// 磁力多的接口域名：壳页面用 `atob` 解出来指向它，实测只有 39988 端口能应答。
pub const CILIDUO_ENGINE: &str = "https://doc2.htmcdn.com:39988";

/// 磁力多接口强制的来源头，缺失时连接直接被重置（104）。
pub const CILIDUO_REFERER: &str = "https://qu.cilido.top/";

/// 内置默认域名表。
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultSiteEndpoints;

impl SiteEndpointProvider for DefaultSiteEndpoints {
    fn endpoint_for(&self, source: TorrentSrc) -> SiteEndpoint {
        match source {
            // bitsearch.to 已 301 到 bitsearch.eu，直接配 .eu 少一跳
            TorrentSrc::SolidTorrent => SiteEndpoint::new("https://bitsearch.eu"),
            TorrentSrc::YuHuaGe => SiteEndpoint::new("https://www.yuhuage.win"),
            TorrentSrc::PirateBay => SiteEndpoint::new("https://thepiratebay.party"),
            TorrentSrc::AcgRip => SiteEndpoint::new("https://acg.rip"),
            TorrentSrc::ZeroMagnet => SiteEndpoint::new("https://0magnet.com"),
            TorrentSrc::X1337 => SiteEndpoint::new("https://www.1337xx.to"),
            TorrentSrc::BtSow => SiteEndpoint::new("https://btsow.live"),
            TorrentSrc::TorLock => SiteEndpoint::new("https://www.torlock.com"),
            TorrentSrc::TorrentKitty => SiteEndpoint::new("https://www.torrentkitty.tv"),
            TorrentSrc::Cilimao => SiteEndpoint::new("https://clmclm.com"),
            TorrentSrc::Ciliduo => {
                SiteEndpoint::new("https://qu.cilido.top").request_host(CILIDUO_ENGINE).referer(CILIDUO_REFERER)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_trimmed_and_deduplicated() {
        let endpoint = SiteEndpoint::with_mirrors("https://a.test/", &["https://b.test", " https://a.test ", ""]);
        assert_eq!(endpoint.hosts(), ["https://a.test", "https://b.test"]);
        assert_eq!(endpoint.url("/search?q=x"), "https://a.test/search?q=x");
        assert_eq!(endpoint.candidates("/p"), vec!["https://a.test/p".to_owned(), "https://b.test/p".to_owned()]);
    }

    #[test]
    fn request_host_changes_requests_but_not_public_urls() {
        let endpoint = SiteEndpoint::new("https://shell.test").request_host("https://api.test:39988/");
        assert_eq!(endpoint.host(), "https://shell.test");
        assert_eq!(endpoint.url("/search"), "https://api.test:39988/search");
        assert_eq!(endpoint.public_url("/doc/x"), "https://shell.test/doc/x");
        assert_eq!(endpoint.candidates("/s")[0], "https://api.test:39988/s");
    }

    #[test]
    fn absolute_and_relative_paths_are_joined_sensibly() {
        let endpoint = SiteEndpoint::new("https://a.test");
        assert_eq!(endpoint.url("https://other.test/x"), "https://other.test/x");
        assert_eq!(endpoint.url("//cdn.test/x"), "https://cdn.test/x");
        assert_eq!(endpoint.url("detail/1"), "https://a.test/detail/1");
        assert_eq!(endpoint.public_url("http://plain.test/y"), "http://plain.test/y");
    }

    #[test]
    fn default_table_covers_every_source() {
        for src in TorrentSrc::ALL {
            assert!(DefaultSiteEndpoints.endpoint_for(src).host().starts_with("https://"));
        }
        let ciliduo = DefaultSiteEndpoints.endpoint_for(TorrentSrc::Ciliduo);
        assert_eq!(ciliduo.request_base(), CILIDUO_ENGINE);
        assert_eq!(ciliduo.referer_header(), Some(CILIDUO_REFERER));
    }

    #[test]
    fn closures_can_provide_endpoints() {
        let provider = |_: TorrentSrc| SiteEndpoint::new("http://127.0.0.1:1");
        assert_eq!(provider.endpoint_for(TorrentSrc::BtSow).host(), "http://127.0.0.1:1");
    }
}
