//! BtSow：JSON 接口驱动。

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::SearchService;
use crate::error::SpiderError;
use crate::magnet;
use crate::magnet::hash;
use crate::model::{TorrentInfo, opt};
use crate::net::HttpClient;
use crate::net::html;
use crate::site::SiteEndpoint;
use crate::site::TorrentSrc;

const SEARCH_PATH: &str = "/bts/data/api/search";
const PAGE_SIZE: u32 = 50;

pub struct BtSow {
    http: Arc<dyn HttpClient>,
    endpoint: SiteEndpoint,
}

#[derive(Deserialize)]
struct SearchResponse {
    data: Option<Vec<Item>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    name: Option<String>,
    hash: Option<String>,
    size: Option<Value>,
    last_update_time: Option<Value>,
}

impl BtSow {
    pub fn new(http: Arc<dyn HttpClient>, endpoint: SiteEndpoint) -> Self {
        Self { http, endpoint }
    }

    /// 接口把参数当成裸数组塞在 body 里：`[{"search": 关键字}, 每页条数, 页码]`。
    pub fn search_body(keyword: &str, page: u32) -> String {
        serde_json::json!([{ "search": keyword }, PAGE_SIZE, page.max(1)]).to_string()
    }

    pub fn parse_search_response(&self, body: &str) -> Vec<TorrentInfo> {
        let Ok(response) = serde_json::from_str::<SearchResponse>(body) else {
            return Vec::new();
        };
        response
            .data
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| {
                let title = strip_html_tags(item.name.as_deref()?)?;
                // 哈希不合法时保留这一行，只是不带下载链接：
                // 丢掉整行会让用户连「搜到了但拿不到链接」都看不到
                let info_hash = item.hash.as_deref().and_then(hash::normalize);
                Some(TorrentInfo {
                    title: Some(title),
                    magnet_url: info_hash.as_deref().and_then(magnet::hash_to_magnet),
                    torrent_url: info_hash.as_deref().and_then(magnet::hash_to_torrent_url),
                    hash: info_hash,
                    date: item.last_update_time.as_ref().and_then(date_text),
                    size: item.size.as_ref().and_then(as_u64).map(format_size),
                    src: TorrentSrc::BtSow.code(),
                    ..Default::default()
                })
            })
            .collect()
    }

    pub fn parse_detail_page(body: &str) -> Option<String> {
        let document = html::parse(body);
        let root = document.root_element();
        // 详情页把磁力放在 #magnetOpen 的 href 上；找不到就按属性扫
        html::select_first(root, "#magnetOpen")
            .and_then(|el| magnet::validated(html::attr(el, "href")))
            .or_else(|| html::href_with_prefix(root, "magnet:").and_then(|h| magnet::validated(&h)))
    }
}

/// 去掉名字里的高亮标签和零宽字符。
///
/// 站点把文件后缀单独包成 `<em>mkv</em>`，拼接文本节点时不能补空格，
/// 否则标题会多出一个站点上不存在的空格。
fn strip_html_tags(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let fragment = scraper::Html::parse_fragment(raw);
    let text: String = fragment.root_element().text().collect();
    opt(html::without_zero_width(&text))
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => opt(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn as_u64(value: &Value) -> Option<u64> {
    scalar_text(value)?.parse().ok()
}

/// `lastUpdateTime` 有时是 ISO 文本，有时是秒级或毫秒级时间戳。
fn date_text(value: &Value) -> Option<String> {
    let raw = scalar_text(value)?;
    let Ok(timestamp) = raw.parse::<i64>() else {
        return Some(raw);
    };
    let seconds = if raw.len() >= 13 { timestamp / 1000 } else { timestamp };
    Some(utc_date(seconds))
}

/// epoch 秒转 `yyyy-MM-dd`（UTC）。只为这一个格式化需求不值得引入日期库。
fn utc_date(epoch_seconds: i64) -> String {
    // Howard Hinnant 的 civil_from_days
    let z = epoch_seconds.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// 字节数格式化成 `1.5G` 这种一位小数的写法。
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes}B");
    }
    let mut value = bytes as f64 / 1024.0;
    for unit in ["K", "M", "G"] {
        if value < 1024.0 {
            return format!("{:.1}{unit}", (value * 10.0).round() / 10.0);
        }
        value /= 1024.0;
    }
    format!("{:.1}T", (value * 10.0).round() / 10.0)
}

#[async_trait]
impl SearchService for BtSow {
    fn source(&self) -> TorrentSrc {
        TorrentSrc::BtSow
    }

    async fn search(&self, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        let url = self.endpoint.url(SEARCH_PATH);
        let body = Self::search_body(keyword, page);
        let response = self.http.post_text(&url, &body, &[("Content-Type", "text/plain; charset=UTF-8")]).await?;
        Ok(self.parse_search_response(&response))
    }

    async fn find_magnet_url(&self, detail_url: &str) -> Option<String> {
        let body = self.http.get_text(detail_url, &[]).await.ok()?;
        Self::parse_detail_page(&body)
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn size_formatting() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(1023), "1023B");
        assert_eq!(format_size(1536), "1.5K");
        assert_eq!(format_size(336_741_734), "321.1M");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.0G");
        assert_eq!(format_size(3 * 1024_u64.pow(4)), "3.0T");
    }

    #[test]
    fn epoch_dates() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(1_773_532_800), "2026-03-15");
        assert_eq!(utc_date(951_782_400), "2000-02-29");
        assert_eq!(date_text(&Value::from(1_773_532_800_000_i64)).as_deref(), Some("2026-03-15"));
        assert_eq!(date_text(&Value::from("1773532800")).as_deref(), Some("2026-03-15"));
        assert_eq!(date_text(&Value::from("2026-03-15 10:00")).as_deref(), Some("2026-03-15 10:00"));
        assert_eq!(date_text(&Value::Null), None);
    }

    #[test]
    fn search_body_escapes_keyword() {
        assert_eq!(BtSow::search_body("a\"b", 2), r#"[{"search":"a\"b"},50,2]"#);
    }
}
