use std::collections::BTreeMap;
use std::sync::Arc;

use super::{DefaultSiteEndpoints, SearchService, SiteEndpointProvider, TorrentSrc, catalog, parsers};
use crate::model::TorrentSource;
use crate::net::HttpClient;

/// 全部站点解析器的装配点。
#[derive(Clone)]
pub struct TorrentServiceRegistry {
    services: BTreeMap<TorrentSrc, Arc<dyn SearchService>>,
    sources: Vec<TorrentSource>,
}

impl TorrentServiceRegistry {
    /// 用内置域名表装配全部站点。
    pub fn new(http: Arc<dyn HttpClient>) -> Self {
        Self::with_endpoints(http, &DefaultSiteEndpoints)
    }

    /// 整体替换域名表，用来指向镜像站或测试桩。
    pub fn with_endpoints(http: Arc<dyn HttpClient>, endpoints: &dyn SiteEndpointProvider) -> Self {
        let services =
            parsers::create_all(http, endpoints).into_iter().map(|service| (service.source(), service)).collect();
        Self { services, sources: catalog::sources_for(endpoints) }
    }

    /// 替换某个站点的实现（例如宿主自己写了更好的解析器）。
    pub fn register(&mut self, service: Arc<dyn SearchService>) {
        self.services.insert(service.source(), service);
    }

    pub fn service(&self, src: TorrentSrc) -> Arc<dyn SearchService> {
        Arc::clone(self.services.get(&src).expect("every TorrentSrc is registered"))
    }

    /// 站点清单，按编号排序。
    pub fn sources(&self) -> &[TorrentSource] {
        &self.sources
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHttp;

    #[test]
    fn every_source_is_wired() {
        let registry = TorrentServiceRegistry::new(FakeHttp::new());
        for src in TorrentSrc::ALL {
            assert_eq!(registry.service(src).source(), src);
        }
        assert_eq!(registry.sources().len(), TorrentSrc::ALL.len());
    }
}
