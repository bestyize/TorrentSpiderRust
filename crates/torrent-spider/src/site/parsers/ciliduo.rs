//! 磁力多：用户看到的域名只返回一段 JS 壳，真实结果由另一个接口域名提供，
//! 且接口强制校验 Referer。

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use base64::Engine;
use regex::Regex;
use scraper::ElementRef;

use super::{BROWSER_UA, SearchService, fill, last_path_segment};
use crate::error::SpiderError;
use crate::magnet;
use crate::magnet::hash;
use crate::model::{TorrentInfo, opt};
use crate::net::HttpClient;
use crate::net::html::{self, DATE_TEXT, SIZE_TEXT};
use crate::site::SiteEndpoint;
use crate::site::TorrentSrc;

const SEARCH_PATH: &str = "/search?word={word}&page={page}";

pub struct Ciliduo {
    http: Arc<dyn HttpClient>,
    endpoint: SiteEndpoint,
}

impl Ciliduo {
    pub fn new(http: Arc<dyn HttpClient>, endpoint: SiteEndpoint) -> Self {
        Self { http, endpoint }
    }

    pub fn search_url(&self, keyword: &str, page: u32) -> String {
        self.endpoint.url(&fill(SEARCH_PATH, keyword, page.max(1)))
    }

    pub fn parse_search_response(&self, body: &str) -> Vec<TorrentInfo> {
        let document = html::parse(body);
        let items = html::select_all_any(document.root_element(), &[".ssbox", "[class*=ssbox]"]);
        items
            .into_iter()
            .filter_map(|item| {
                let anchor = html::select_first_any(item, &[".title h3 a", "h3 a", ".title a"])?;
                // 站点把搜索词拆成相邻的 <em> 逐字高亮，拼接时不能在它们之间补空格
                let title = opt(html::title_text(anchor))?;
                let href = html::attr(anchor, "href");

                // 优先用条目自带的 magnet；没有才用详情页地址反推
                let magnet_url = html::href_with_prefix(item, magnet::MAGNET_PREFIX)
                    .and_then(|h| magnet::validated(&h))
                    .or_else(|| hash_of_doc_url(href).and_then(|h| magnet::hash_to_magnet(&h)));
                let info_hash = magnet_url.as_deref().and_then(magnet::magnet_to_hash);

                let bar = html::select_first(item, ".sbar").unwrap_or(item);
                Some(TorrentInfo {
                    torrent_url: info_hash.as_deref().and_then(magnet::hash_to_torrent_url),
                    hash: info_hash,
                    title: Some(title),
                    detail_url: opt(href).map(|h| self.endpoint.public_url(&h)),
                    magnet_url,
                    // 每项都带中文前缀（添加时间：/大小：），按前缀取值比按下标取值稳
                    date: labeled_value(bar, "添加时间").or_else(|| opt(html::text_matching(bar, &DATE_TEXT, "b"))),
                    size: labeled_value(bar, "大小").or_else(|| opt(html::text_matching(bar, &SIZE_TEXT, "b"))),
                    sender_count: labeled_value(bar, "文件数"),
                    download_count: labeled_value(bar, "热度"),
                    src: TorrentSrc::Ciliduo.code(),
                    ..Default::default()
                })
            })
            .collect()
    }

    /// 站点按 Referer 做来源校验，缺了它连接直接被重置。
    fn headers(&self) -> Vec<(&str, &str)> {
        let mut headers = vec![("User-Agent", BROWSER_UA)];
        if let Some(referer) = self.endpoint.referer_header() {
            headers.push(("Referer", referer));
        }
        headers
    }
}

/// 取出 `/doc/<hash>` 末段的哈希（十六进制或 Base32），统一成大写十六进制。
pub fn hash_of_doc_url(url: &str) -> Option<String> {
    hash::normalize(last_path_segment(url))
}

/// 读取形如 `<span>标签：<b>值</b></span>` 的字段，全角半角冒号都认。
fn labeled_value(scope: ElementRef<'_>, label: &str) -> Option<String> {
    let span = html::select(scope, "span").into_iter().find(|s| html::text(*s).starts_with(label))?;
    if let Some(value) = html::select_first(span, "b").map(html::text).and_then(opt) {
        return Some(value);
    }
    let text = html::text(span);
    let (_, value) = text.split_once(['：', ':'])?;
    opt(value)
}

#[async_trait]
impl SearchService for Ciliduo {
    fn source(&self) -> TorrentSrc {
        TorrentSrc::Ciliduo
    }

    async fn search(&self, keyword: &str, page: u32) -> Result<Vec<TorrentInfo>, SpiderError> {
        let body = self.http.get_text(&self.search_url(keyword, page), &self.headers()).await?;
        Ok(self.parse_search_response(&body))
    }

    /// 详情页地址末段就是 info hash，不必真的请求详情页（实测详情页偶尔卡住几十秒）。
    async fn find_magnet_url(&self, detail_url: &str) -> Option<String> {
        hash_of_doc_url(detail_url).and_then(|h| magnet::hash_to_magnet(&h))
    }
}

static PROXY_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"atob\(\s*['"]([A-Za-z0-9+/=]{16,})['"]\s*\)"#).unwrap());

/// 从磁力多的壳页面里解出接口域名，形如 `https://host:port`（无结尾斜杠）。
///
/// 壳里有一句 `var proxy = atob('...')`，解出来才是真正提供接口的域名。
/// 这类站点接口域名换得很勤，壳里的字符串是唯一权威来源；
/// 换域名时据此更新 [`crate::site::CILIDUO_ENGINE`]。
pub fn engine_url(shell_html: &str) -> Option<String> {
    PROXY_PATTERN
        .captures_iter(shell_html)
        .filter_map(|caps| {
            let bytes = base64::engine::general_purpose::STANDARD.decode(&caps[1]).ok()?;
            String::from_utf8(bytes).ok()
        })
        .find(|decoded| decoded.starts_with("http://") || decoded.starts_with("https://"))
        .map(|url| url.trim_end_matches('/').to_owned())
}
