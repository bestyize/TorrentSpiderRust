//! magnet 链接解析，以及 info hash、magnet、`.torrent` 地址之间的换算。

pub mod hash;
pub mod torrent_file;

pub use torrent_file::{TorrentFileResolver, TorrentFileSource};

use crate::net::encoding::{url_decode, url_encode};

pub const MAGNET_PREFIX: &str = "magnet:?xt=urn:btih:";
const SCHEME: &str = "magnet:?";
const BTIH_PREFIX: &str = "urn:btih:";

/// 解析后的 magnet 链接。
///
/// 按 URI 参数逐个解析而不是做前缀匹配，因此 `xt` 不在第一位、`dn` 里有转义字符、
/// 多个 `tr`、Base32 形式的 BTIH 都能正确处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnetLink {
    /// 大写十六进制 BTIH。
    pub info_hash: String,
    pub display_name: Option<String>,
    pub trackers: Vec<String>,
}

impl MagnetLink {
    /// 解析 magnet URI；不含合法 BTIH 时返回 `None`。
    pub fn parse(raw: &str) -> Option<Self> {
        let uri = raw.trim();
        if !uri.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("magnet:")) {
            return None;
        }
        let (_, query) = uri.split_once('?')?;
        if query.is_empty() {
            return None;
        }

        let mut info_hash = None;
        let mut display_name: Option<String> = None;
        let mut trackers = Vec::new();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = url_decode(value);
            match url_decode(key).to_ascii_lowercase().as_str() {
                "xt" if info_hash.is_none() => info_hash = btih_of(&value),
                "dn" if display_name.is_none() && !value.trim().is_empty() => display_name = Some(value),
                "tr" if !value.trim().is_empty() => trackers.push(value),
                _ => {}
            }
        }
        info_hash.map(|info_hash| Self { info_hash, display_name, trackers })
    }

    /// 由 info hash 直接构造，哈希不合法时返回 `None`。
    pub fn from_hash(raw: &str) -> Option<Self> {
        hash::normalize(raw).map(|info_hash| Self { info_hash, display_name: None, trackers: vec![] })
    }

    /// 重新拼出 magnet 链接，只保留最小必需参数。
    pub fn to_uri(&self, include_trackers: bool) -> String {
        let mut uri = format!("{SCHEME}xt={BTIH_PREFIX}{}", self.info_hash);
        if let Some(name) = self.display_name.as_deref().filter(|n| !n.trim().is_empty()) {
            uri.push_str("&dn=");
            uri.push_str(&url_encode(name));
        }
        if include_trackers {
            for tracker in &self.trackers {
                uri.push_str("&tr=");
                uri.push_str(&url_encode(tracker));
            }
        }
        uri
    }
}

fn btih_of(xt: &str) -> Option<String> {
    let prefix = xt.get(..BTIH_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(BTIH_PREFIX) {
        return None;
    }
    hash::normalize(&xt[BTIH_PREFIX.len()..])
}

/// 原样返回 magnet 链接（去掉首尾空白），没有合法 BTIH 时返回 `None`。
///
/// 刻意不重新拼装：站点给的链接里通常带着 tracker 和显示名，重拼会丢掉。
pub fn validated(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    MagnetLink::parse(trimmed).map(|_| trimmed.to_owned())
}

/// 从 magnet 链接取出大写十六进制 info hash。
pub fn magnet_to_hash(magnet: &str) -> Option<String> {
    MagnetLink::parse(magnet).map(|m| m.info_hash)
}

/// 由 magnet 链接推导 `.torrent` 下载地址（首选缓存源）。
pub fn magnet_to_torrent_url(magnet: &str) -> Option<String> {
    magnet_to_hash(magnet).and_then(|h| hash_to_torrent_url(&h))
}

/// 由 info hash 拼出最小 magnet 链接。
pub fn hash_to_magnet(raw_hash: &str) -> Option<String> {
    hash::normalize(raw_hash).map(|h| format!("{MAGNET_PREFIX}{h}"))
}

/// 由 info hash 推导 `.torrent` 下载地址（首选缓存源）。
///
/// 地址只是推导出来的，不代表缓存源一定有这个文件；
/// 需要确认可用时用 [`crate::TorrentFileResolver`] 探测。
pub fn hash_to_torrent_url(raw_hash: &str) -> Option<String> {
    hash_to_torrent_url_candidates(raw_hash).into_iter().next()
}

/// 由 info hash 推导全部候选 `.torrent` 地址，首选在前。
pub fn hash_to_torrent_url_candidates(raw_hash: &str) -> Vec<String> {
    match hash::normalize(raw_hash) {
        Some(h) => torrent_file::DEFAULT_SOURCES.iter().map(|s| s.url_for(&h)).collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[test]
    fn parses_plain_magnet() {
        let link = MagnetLink::parse(&magnet(HEX_LOWER)).unwrap();
        assert_eq!(link.info_hash, HEX);
        assert_eq!(link.display_name, None);
    }

    #[test]
    fn xt_does_not_have_to_come_first() {
        let raw = format!("magnet:?dn=Some%20Name&tr=udp%3A%2F%2Ft1&xt=urn:btih:{HEX}&tr=udp://t2");
        let link = MagnetLink::parse(&raw).unwrap();
        assert_eq!(link.info_hash, HEX);
        assert_eq!(link.display_name.as_deref(), Some("Some Name"));
        assert_eq!(link.trackers, vec!["udp://t1", "udp://t2"]);
    }

    #[test]
    fn display_name_keeps_escaped_ampersand_and_unicode() {
        let raw = format!("magnet:?xt=urn:btih:{HEX}&dn=A%26B%20%E4%B8%AD%E6%96%87");
        assert_eq!(MagnetLink::parse(&raw).unwrap().display_name.as_deref(), Some("A&B 中文"));
    }

    #[test]
    fn base32_btih_is_normalized() {
        assert_eq!(magnet_to_hash(&magnet(BASE32)).as_deref(), Some(HEX));
    }

    #[test]
    fn scheme_is_case_insensitive_and_other_urns_are_ignored() {
        assert!(MagnetLink::parse(&format!("MAGNET:?xt=urn:btih:{HEX}")).is_some());
        assert!(MagnetLink::parse(&format!("magnet:?xt=URN:BTIH:{HEX}")).is_some());
        assert_eq!(MagnetLink::parse("magnet:?xt=urn:sha1:ABCDEF"), None);
        let raw = format!("magnet:?xt=urn:ed2k:xyz&xt=urn:btih:{HEX}");
        assert_eq!(magnet_to_hash(&raw).as_deref(), Some(HEX));
    }

    #[test]
    fn rejects_broken_input() {
        for bad in ["", "   ", "http://example.com", "magnet:", "magnet:?", "magnet:?xt=urn:btih:SHORT"] {
            assert_eq!(MagnetLink::parse(bad), None, "{bad}");
            assert_eq!(validated(bad), None, "{bad}");
        }
    }

    #[test]
    fn validated_keeps_original_link() {
        let raw = format!("  magnet:?xt=urn:btih:{HEX_LOWER}&dn=x&tr=udp://t  ");
        assert_eq!(validated(&raw).as_deref(), Some(raw.trim()));
    }

    #[test]
    fn to_uri_rebuilds_minimal_link() {
        let link = MagnetLink {
            info_hash: HEX.into(),
            display_name: Some("a b".into()),
            trackers: vec!["udp://t:1/announce".into()],
        };
        assert_eq!(link.to_uri(false), format!("magnet:?xt=urn:btih:{HEX}&dn=a%20b"));
        assert_eq!(link.to_uri(true), format!("magnet:?xt=urn:btih:{HEX}&dn=a%20b&tr=udp%3A%2F%2Ft%3A1%2Fannounce"));
        assert_eq!(MagnetLink::from_hash(BASE32).unwrap().to_uri(false), magnet(HEX));
    }

    #[test]
    fn conversions() {
        assert_eq!(hash_to_magnet(HEX_LOWER), Some(magnet(HEX)));
        assert_eq!(hash_to_magnet("bad"), None);
        assert_eq!(hash_to_torrent_url(HEX_LOWER), Some(torrent_url(HEX)));
        assert_eq!(magnet_to_torrent_url(&magnet(BASE32)), Some(torrent_url(HEX)));
        assert_eq!(magnet_to_torrent_url("magnet:?xt=urn:btih:SHORT"), None);
        assert_eq!(
            hash_to_torrent_url_candidates(HEX),
            vec![
                format!("https://itorrents.net/torrent/{HEX}.torrent"),
                format!("https://itorrents.org/torrent/{HEX}.torrent"),
            ]
        );
    }
}
