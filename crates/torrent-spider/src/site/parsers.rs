//! 各站点的解析器，每站一个文件。
//!
//! 每个站点都把「构造请求」和「解析响应」分开：`parse_*` 是纯函数，
//! 可以直接喂 HTML / JSON 做离线测试，不必碰网络。

use std::sync::Arc;

use super::{SearchService, SiteEndpointProvider, TorrentSrc};
use crate::net::HttpClient;

pub mod acgrip;
pub mod btsow;
pub mod ciliduo;
pub mod cilimao;
pub mod piratebay;
pub mod solidtorrent;
pub mod torlock;
pub mod torrentkitty;
pub mod x1337;
pub mod yuhuage;
pub mod zeromagnet;

pub use acgrip::AcgRip;
pub use btsow::BtSow;
pub use ciliduo::Ciliduo;
pub use cilimao::Cilimao;
pub use piratebay::PirateBay;
pub use solidtorrent::SolidTorrent;
pub use torlock::TorLock;
pub use torrentkitty::TorrentKitty;
pub use x1337::X1337;
pub use yuhuage::YuHuaGe;
pub use zeromagnet::ZeroMagnet;

/// 模拟桌面浏览器的 UA，给会校验 UA 的站点用。
pub(crate) const BROWSER_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// 装配全部站点。
pub fn create_all(http: Arc<dyn HttpClient>, endpoints: &dyn SiteEndpointProvider) -> Vec<Arc<dyn SearchService>> {
    let ep = |src| endpoints.endpoint_for(src);
    vec![
        Arc::new(SolidTorrent::new(http.clone(), ep(TorrentSrc::SolidTorrent))),
        Arc::new(YuHuaGe::new(http.clone(), ep(TorrentSrc::YuHuaGe))),
        Arc::new(PirateBay::new(http.clone(), ep(TorrentSrc::PirateBay))),
        Arc::new(AcgRip::new(http.clone(), ep(TorrentSrc::AcgRip))),
        Arc::new(ZeroMagnet::new(http.clone(), ep(TorrentSrc::ZeroMagnet))),
        Arc::new(X1337::new(http.clone(), ep(TorrentSrc::X1337))),
        Arc::new(BtSow::new(http.clone(), ep(TorrentSrc::BtSow))),
        Arc::new(TorLock::new(http.clone(), ep(TorrentSrc::TorLock))),
        Arc::new(TorrentKitty::new(http.clone(), ep(TorrentSrc::TorrentKitty))),
        Arc::new(Cilimao::new(http.clone(), ep(TorrentSrc::Cilimao))),
        Arc::new(Ciliduo::new(http, ep(TorrentSrc::Ciliduo))),
    ]
}

/// 把 `{word}` / `{page}` 占位符替换成编码后的关键字和页码。
pub(crate) fn fill(template: &str, keyword: &str, page: u32) -> String {
    template.replace("{word}", &crate::net::encoding::url_encode(keyword)).replace("{page}", &page.to_string())
}

/// 取 URL 路径最后一段（去掉查询串、片段、结尾斜杠）。
pub(crate) fn last_path_segment(url: &str) -> &str {
    let path = url.split(['?', '#']).next().unwrap_or("").trim_end_matches('/');
    path.rsplit('/').next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_encodes_keyword() {
        assert_eq!(fill("/s/{word}/{page}", "a b/中", 3), "/s/a%20b%2F%E4%B8%AD/3");
    }

    #[test]
    fn last_segment_ignores_query_fragment_and_slash() {
        assert_eq!(last_path_segment("https://x/doc/ABC/?a=1#f"), "ABC");
        assert_eq!(last_path_segment("https://x/doc/ABC.html"), "ABC.html");
        assert_eq!(last_path_segment(""), "");
    }
}
