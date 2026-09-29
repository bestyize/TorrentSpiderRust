//! 百分号编解码。

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode, utf8_percent_encode};

/// RFC 3986 unreserved 之外的字符一律转义。
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// 百分号编码。空格编成 `%20` 而不是 `+`：magnet 的 `dn` 里 `%20` 不会被误读。
pub fn url_encode(value: &str) -> String {
    utf8_percent_encode(value, UNRESERVED).to_string()
}

/// 百分号解码，`+` 按查询串惯例还原成空格。
///
/// 按字节解码后整体做 UTF-8 解码，`dn` 里的中文不会被拆坏；非法序列替换成 U+FFFD。
pub fn url_decode(value: &str) -> String {
    if !value.contains(['%', '+']) {
        return value.to_owned();
    }
    let spaced = value.replace('+', " ");
    percent_decode(spaced.as_bytes()).decode_utf8_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_everything_but_unreserved() {
        assert_eq!(url_encode("a b&c=d/中"), "a%20b%26c%3Dd%2F%E4%B8%AD");
        assert_eq!(url_encode("Az09-_.~"), "Az09-_.~");
    }

    #[test]
    fn decodes_percent_and_plus() {
        assert_eq!(url_decode("a%20b+c"), "a b c");
        assert_eq!(url_decode("%E4%B8%AD%E6%96%87"), "中文");
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(url_decode("%zz"), "%zz");
        assert_eq!(url_decode("plain"), "plain");
    }

    #[test]
    fn round_trips_unicode() {
        let raw = "微微一笑很倾城 1080p & more";
        assert_eq!(url_decode(&url_encode(raw)), raw);
    }
}
