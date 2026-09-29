use super::{DefaultSiteEndpoints, SiteEndpointProvider};
use crate::model::TorrentSource;

/// 支持的站点。编号是对外契约，永不复用、不依赖声明顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TorrentSrc {
    SolidTorrent,
    YuHuaGe,
    PirateBay,
    AcgRip,
    ZeroMagnet,
    X1337,
    BtSow,
    TorLock,
    TorrentKitty,
    Cilimao,
    Ciliduo,
}

impl TorrentSrc {
    pub const ALL: [TorrentSrc; 11] = [
        TorrentSrc::SolidTorrent,
        TorrentSrc::YuHuaGe,
        TorrentSrc::PirateBay,
        TorrentSrc::AcgRip,
        TorrentSrc::ZeroMagnet,
        TorrentSrc::X1337,
        TorrentSrc::BtSow,
        TorrentSrc::TorLock,
        TorrentSrc::TorrentKitty,
        TorrentSrc::Cilimao,
        TorrentSrc::Ciliduo,
    ];

    pub const fn code(self) -> u32 {
        match self {
            TorrentSrc::SolidTorrent => 0,
            TorrentSrc::YuHuaGe => 1,
            TorrentSrc::PirateBay => 2,
            TorrentSrc::AcgRip => 3,
            TorrentSrc::ZeroMagnet => 4,
            TorrentSrc::X1337 => 5,
            TorrentSrc::BtSow => 6,
            TorrentSrc::TorLock => 7,
            TorrentSrc::TorrentKitty => 8,
            TorrentSrc::Cilimao => 9,
            TorrentSrc::Ciliduo => 10,
        }
    }

    /// 未知编号返回 `None`。
    ///
    /// 刻意不回退到某个默认站点：`src=999` 静默变成另一个站的结果，
    /// 调用方拿到的是错的数据却无从察觉。
    pub fn from_code(code: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.code() == code)
    }

    pub const fn title(self) -> &'static str {
        match self {
            TorrentSrc::SolidTorrent => "Solid",
            TorrentSrc::YuHuaGe => "YuHuaGe",
            TorrentSrc::PirateBay => "海盗湾",
            TorrentSrc::AcgRip => "AcgRip",
            TorrentSrc::ZeroMagnet => "Zero",
            TorrentSrc::X1337 => "1337",
            TorrentSrc::BtSow => "BtSow",
            TorrentSrc::TorLock => "TorLock",
            TorrentSrc::TorrentKitty => "Kitty",
            TorrentSrc::Cilimao => "磁力猫",
            TorrentSrc::Ciliduo => "磁力多",
        }
    }

    const fn desc(self) -> &'static str {
        match self {
            TorrentSrc::SolidTorrent => "BitSearch / SolidTorrent style HTML page",
            TorrentSrc::YuHuaGe => "Search page requiring browser-like headers",
            TorrentSrc::PirateBay => "Pirate Bay style listing page",
            TorrentSrc::AcgRip => "Anime torrent feed",
            TorrentSrc::ZeroMagnet => "Search result page with detail follow-up for magnet",
            TorrentSrc::X1337 => "1337x style listing page",
            TorrentSrc::BtSow => "JSON API driven source",
            TorrentSrc::TorLock => "Table-based HTML page",
            TorrentSrc::TorrentKitty => "Classic torrent listing page",
            TorrentSrc::Cilimao => "Chinese torrent index page",
            TorrentSrc::Ciliduo => "Chinese DHT index; front end is a JS shell over a separate API host",
        }
    }

    const fn level(self) -> u32 {
        match self {
            TorrentSrc::YuHuaGe => 1,
            _ => 2,
        }
    }
}

/// 对外暴露的站点清单，`official_url` 取自 [`DefaultSiteEndpoints`]，不另存第二份域名表。
pub fn supported_sources() -> Vec<TorrentSource> {
    sources_for(&DefaultSiteEndpoints)
}

pub(crate) fn sources_for(endpoints: &dyn SiteEndpointProvider) -> Vec<TorrentSource> {
    TorrentSrc::ALL
        .into_iter()
        .map(|src| TorrentSource {
            src: src.code(),
            title: src.title().to_owned(),
            sub_title: None,
            desc: Some(src.desc().to_owned()),
            official_url: Some(endpoints.endpoint_for(src).host().to_owned()),
            level: src.level(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_contiguous_and_round_trip() {
        for (index, src) in TorrentSrc::ALL.into_iter().enumerate() {
            assert_eq!(src.code() as usize, index);
            assert_eq!(TorrentSrc::from_code(src.code()), Some(src));
        }
        assert_eq!(TorrentSrc::from_code(6), Some(TorrentSrc::BtSow));
        assert_eq!(TorrentSrc::from_code(999), None);
    }

    #[test]
    fn catalog_lists_every_source_with_https_url() {
        let sources = supported_sources();
        assert_eq!(sources.len(), TorrentSrc::ALL.len());
        assert!(sources.iter().any(|s| s.src == 6 && s.title == "BtSow"));
        assert!(sources.iter().any(|s| s.src == 1 && s.title == "YuHuaGe" && s.level == 1));
        for source in &sources {
            let url = source.official_url.as_deref().unwrap();
            assert!(url.starts_with("https://"), "{url}");
            assert!(!url.contains("btsow.com"), "stale domain {url}");
        }
    }
}
