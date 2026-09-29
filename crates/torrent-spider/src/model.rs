use serde::{Deserialize, Serialize};

/// 一条搜索结果。
///
/// 字段名即对外 JSON 契约（snake_case），只增不改。
/// 所有文本字段在库内就把空串 / 纯空白归一成 `None`，调用方只需判一次空。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TorrentInfo {
    /// 规范化后的大写十六进制 BTIH。
    pub hash: Option<String>,
    pub detail_url: Option<String>,
    pub magnet_url: Option<String>,
    pub torrent_url: Option<String>,
    pub title: Option<String>,
    pub date: Option<String>,
    pub last_download_date: Option<String>,
    /// 尺寸文本，保持源站写法。
    pub size: Option<String>,
    pub src: u32,
    pub download_count: Option<String>,
    pub leacher_count: Option<String>,
    pub sender_count: Option<String>,
}

impl TorrentInfo {
    /// 把所有空串 / 纯空白字段归一成 `None`。
    ///
    /// 库内的解析器已经保证这一点；这个方法给「数据来自别处」的调用方兜底。
    #[must_use]
    pub fn normalized(self) -> Self {
        Self {
            hash: blank_to_none(self.hash),
            detail_url: blank_to_none(self.detail_url),
            magnet_url: blank_to_none(self.magnet_url),
            torrent_url: blank_to_none(self.torrent_url),
            title: blank_to_none(self.title),
            date: blank_to_none(self.date),
            last_download_date: blank_to_none(self.last_download_date),
            size: blank_to_none(self.size),
            src: self.src,
            download_count: blank_to_none(self.download_count),
            leacher_count: blank_to_none(self.leacher_count),
            sender_count: blank_to_none(self.sender_count),
        }
    }
}

/// 站点清单里的一项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TorrentSource {
    /// 稳定站点编号，永不复用。
    pub src: u32,
    pub title: String,
    pub sub_title: Option<String>,
    pub desc: Option<String>,
    pub official_url: Option<String>,
    /// 稳定度 / 优先级；`1` 表示需要浏览器头之类的额外条件。
    pub level: u32,
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

/// 解析器内部用：空串 / 纯空白视为「没有这个值」，其余去掉首尾空白。
pub(crate) fn opt(value: impl AsRef<str>) -> Option<String> {
    let trimmed = value.as_ref().trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_with_snake_case_contract_and_explicit_nulls() {
        let info = TorrentInfo {
            detail_url: Some("https://detail".into()),
            last_download_date: Some("2026-03-16".into()),
            sender_count: Some("88".into()),
            src: 6,
            ..Default::default()
        };
        let json = serde_json::to_string(&info).unwrap();
        for key in [
            "\"detail_url\"",
            "\"magnet_url\":null",
            "\"torrent_url\":null",
            "\"last_download_date\"",
            "\"sender_count\"",
            "\"src\":6",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
    }

    #[test]
    fn normalized_turns_blank_into_none() {
        let info = TorrentInfo {
            title: Some("keep".into()),
            hash: Some(String::new()),
            date: Some("   ".into()),
            ..Default::default()
        }
        .normalized();
        assert_eq!(info.title.as_deref(), Some("keep"));
        assert_eq!(info.hash, None);
        assert_eq!(info.date, None);
    }

    #[test]
    fn opt_trims_and_drops_blank() {
        assert_eq!(opt("  a b "), Some("a b".to_owned()));
        assert_eq!(opt(" \n"), None);
    }
}
