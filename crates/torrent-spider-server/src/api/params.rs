//! 查询参数的校验与归一化。取值不合法一律 400 + 对应错误码，不静默回落到默认值。

use torrent_spider::TorrentSource;
use torrent_spider::magnet::magnet_to_hash;

use super::{ApiError, codes};

pub const MAX_PAGE: u32 = 100;
pub const ALL_SOURCES: &str = "all";

/// 解析 `src`。`None` 表示聚合全部来源。
///
/// 越界必须在这里挡掉：不认识的编号静默落到某个默认站点，
/// 调用方拿到的是别的源的数据却无从察觉。
pub(crate) fn parse_search_src(raw: Option<&str>, known: &[TorrentSource]) -> Result<Option<u32>, ApiError> {
    let value = raw.map(str::trim).unwrap_or("");
    if value.is_empty() || value.eq_ignore_ascii_case(ALL_SOURCES) {
        return Ok(None);
    }
    parse_code(value, known, "，或 all").map(Some)
}

/// 解析 `/api/magnet` 的 `src`：必填，且不接受 `all`。
pub(crate) fn parse_magnet_src(raw: Option<&str>, known: &[TorrentSource]) -> Result<u32, ApiError> {
    let value = raw.map(str::trim).unwrap_or("");
    if value.is_empty() {
        return Err(ApiError::bad_request(codes::INVALID_SRC, "缺少 src，获取磁力必须指定来源站点。"));
    }
    if value.eq_ignore_ascii_case(ALL_SOURCES) {
        return Err(ApiError::bad_request(codes::INVALID_SRC, "src=all 不支持获取磁力：解析磁力必须指定具体站点。"));
    }
    parse_code(value, known, "")
}

fn parse_code(value: &str, known: &[TorrentSource], suffix: &str) -> Result<u32, ApiError> {
    let max = known.iter().map(|s| s.src).max().unwrap_or(0);
    let code: u32 = value.parse().map_err(|_| {
        ApiError::bad_request(
            codes::INVALID_SRC,
            format!("src 只能是 0 到 {max} 之间的站点编号{suffix}，收到：{value}"),
        )
    })?;
    if !known.iter().any(|s| s.src == code) {
        return Err(ApiError::bad_request(
            codes::INVALID_SRC,
            format!("src 越界：{code}。可用范围是 0 到 {max}{suffix}。"),
        ));
    }
    Ok(code)
}

pub(crate) fn parse_page(raw: Option<&str>) -> Result<u32, ApiError> {
    let value = raw.map(str::trim).unwrap_or("");
    if value.is_empty() {
        return Ok(1);
    }
    let out_of_range =
        || ApiError::bad_request(codes::INVALID_PAGE, format!("page 越界：{value}。可用范围是 1 到 {MAX_PAGE}。"));
    if !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::bad_request(codes::INVALID_PAGE, format!("page 必须是数字，收到：{value}")));
    }
    // 超长数字与越界走同一个口径，不区分「解析溢出」和「超出范围」
    let page: u32 = value.parse().map_err(|_| out_of_range())?;
    if !(1..=MAX_PAGE).contains(&page) {
        return Err(out_of_range());
    }
    Ok(page)
}

/// 布尔参数只认 `1/0`、`true/false`（不分大小写），空值取默认。写错就 400：
/// 参数拼错却看着一切正常，是排查成本最高的一类问题。
pub(crate) fn parse_flag(raw: Option<&str>, default: bool, name: &str, code: &'static str) -> Result<bool, ApiError> {
    let value = raw.map(str::trim).unwrap_or("");
    match value.to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => Err(ApiError::bad_request(code, format!("{name} 只能是 1/0 或 true/false，收到：{value}"))),
    }
}

pub(crate) fn require_keyword(raw: Option<&str>) -> Result<String, ApiError> {
    match raw.map(str::trim) {
        Some(keyword) if !keyword.is_empty() => Ok(keyword.to_owned()),
        _ => Err(ApiError::bad_request(codes::MISSING_KEYWORD, "缺少搜索关键字 q。")),
    }
}

/// 详情页地址只收 http(s)：放开其他 scheme 等于把服务端变成任意请求发起器（SSRF 跳板）。
pub(crate) fn require_detail_url(raw: Option<&str>) -> Result<String, ApiError> {
    let value = raw.map(str::trim).unwrap_or("");
    if value.is_empty() {
        return Err(ApiError::bad_request(codes::MISSING_URL, "缺少 detail_url。"));
    }
    let lower = value.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return Err(ApiError::bad_request(
            codes::INVALID_URL,
            format!("detail_url 必须是 http(s) 地址，收到：{value}"),
        ));
    }
    Ok(value.to_owned())
}

/// 客户端传来的已有磁力：只认 `magnet:` 开头且能解出 BTIH 的值，
/// 不让前端把任意字符串当成「已有的磁力」塞进响应。
pub(crate) fn normalize_incoming_magnet(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    magnet_to_hash(value).map(|_| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<TorrentSource> {
        torrent_spider::supported_sources()
    }

    #[test]
    fn search_src_accepts_all_and_known_codes() {
        assert_eq!(parse_search_src(None, &known()).unwrap(), None);
        assert_eq!(parse_search_src(Some(" ALL "), &known()).unwrap(), None);
        assert_eq!(parse_search_src(Some("6"), &known()).unwrap(), Some(6));
        assert_eq!(parse_search_src(Some("999"), &known()).unwrap_err().code, codes::INVALID_SRC);
        assert_eq!(parse_search_src(Some("-1"), &known()).unwrap_err().code, codes::INVALID_SRC);
        assert_eq!(parse_search_src(Some("abc"), &known()).unwrap_err().code, codes::INVALID_SRC);
    }

    #[test]
    fn magnet_src_is_required_and_rejects_all() {
        assert_eq!(parse_magnet_src(Some("7"), &known()).unwrap(), 7);
        for bad in [None, Some(""), Some("all"), Some("11")] {
            assert_eq!(parse_magnet_src(bad, &known()).unwrap_err().code, codes::INVALID_SRC, "{bad:?}");
        }
    }

    #[test]
    fn flags_accept_only_boolean_spellings() {
        let flag = |raw| parse_flag(raw, true, "filter", codes::INVALID_FILTER);
        assert!(flag(None).unwrap());
        assert!(flag(Some(" ")).unwrap());
        assert!(!flag(Some("0")).unwrap());
        assert!(!flag(Some("FALSE")).unwrap());
        assert!(flag(Some("True")).unwrap());
        for bad in ["yes", "2", "off"] {
            assert_eq!(flag(Some(bad)).unwrap_err().code, codes::INVALID_FILTER, "{bad}");
        }
    }

    #[test]
    fn page_rules() {
        assert_eq!(parse_page(None).unwrap(), 1);
        assert_eq!(parse_page(Some("100")).unwrap(), 100);
        for bad in ["0", "101", "-1", "1.5", "abc", "2147483647", "99999999999999999999"] {
            assert_eq!(parse_page(Some(bad)).unwrap_err().code, codes::INVALID_PAGE, "{bad}");
        }
    }

    #[test]
    fn detail_url_must_be_http() {
        assert!(require_detail_url(Some("https://x/y")).is_ok());
        assert!(require_detail_url(Some("HTTP://x/y")).is_ok());
        assert_eq!(require_detail_url(None).unwrap_err().code, codes::MISSING_URL);
        for bad in ["file:///etc/passwd", "gopher://x", "ftp://x", "//x/y"] {
            assert_eq!(require_detail_url(Some(bad)).unwrap_err().code, codes::INVALID_URL, "{bad}");
        }
    }

    #[test]
    fn incoming_magnet_must_carry_a_btih() {
        let good = "magnet:?xt=urn:btih:2C6B6858D61DA9543D4231A71DB4B1C9264B0685&dn=x";
        assert_eq!(normalize_incoming_magnet(Some(good)).as_deref(), Some(good));
        assert_eq!(normalize_incoming_magnet(Some("not-a-magnet")), None);
        assert_eq!(normalize_incoming_magnet(Some("xt=urn:btih:2C6B6858D61DA9543D4231A71DB4B1C9264B0685")), None);
        assert_eq!(normalize_incoming_magnet(None), None);
    }
}
