//! BitTorrent info hash（BTIH）的规范化。
//!
//! magnet 里的 BTIH 有两种等价写法：40 位十六进制，或 32 位 Base32（RFC 4648，无填充）。
//! 库内统一存成大写十六进制：`.torrent` 缓存服务只认十六进制，
//! 同一个种子用两种写法出现时也不会变成两条记录。
//!
//! 长度或字符集不合法一律返回 `None`。宁可判定为「没有哈希」，
//! 也不要把一段残缺字符串拼进 `.torrent` 地址——那只会得到一个必然 404 的链接。

pub const HEX_LENGTH: usize = 40;
pub const BASE32_LENGTH: usize = 32;

const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// 解析任意形式的 info hash，返回大写十六进制；无法识别时返回 `None`。
pub fn normalize(raw: &str) -> Option<String> {
    let value = raw.trim();
    if is_hex(value) {
        return Some(value.to_ascii_uppercase());
    }
    if value.len() == BASE32_LENGTH {
        return base32_to_hex(value);
    }
    None
}

pub fn is_hex(value: &str) -> bool {
    value.len() == HEX_LENGTH && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn is_base32(value: &str) -> bool {
    value.len() == BASE32_LENGTH && value.bytes().all(|b| base32_digit(b).is_some())
}

/// 32 个 Base32 字符恰好 160 位 = 20 字节，重新编码成大写十六进制。
pub fn base32_to_hex(value: &str) -> Option<String> {
    if value.len() != BASE32_LENGTH {
        return None;
    }
    let mut out = String::with_capacity(HEX_LENGTH);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in value.bytes() {
        buffer = (buffer << 5) | base32_digit(byte)?;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            let octet = (buffer >> bits) & 0xFF;
            out.push(HEX_DIGITS[(octet >> 4) as usize] as char);
            out.push(HEX_DIGITS[(octet & 0x0F) as usize] as char);
        }
    }
    Some(out)
}

/// 十六进制反解成 Base32，[`base32_to_hex`] 的逆运算。
///
/// 有些站点（如磁力多）的详情页地址用 Base32 写法，回拼地址时需要换回去。
pub fn hex_to_base32(value: &str) -> Option<String> {
    let hex = value.trim();
    if !is_hex(hex) {
        return None;
    }
    let mut out = String::with_capacity(BASE32_LENGTH);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for pair in hex.as_bytes().chunks(2) {
        let octet = u32::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        buffer = (buffer << 8) | octet;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(BASE32_ALPHABET[((buffer >> bits) & 0x1F) as usize] as char);
        }
    }
    Some(out)
}

fn base32_digit(byte: u8) -> Option<u32> {
    match byte.to_ascii_uppercase() {
        upper @ b'A'..=b'Z' => Some(u32::from(upper - b'A')),
        digit @ b'2'..=b'7' => Some(u32::from(digit - b'2') + 26),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[test]
    fn hex_is_uppercased() {
        assert_eq!(normalize(HEX_LOWER).as_deref(), Some(HEX));
        assert_eq!(normalize(&format!("  {HEX}  ")).as_deref(), Some(HEX));
    }

    #[test]
    fn base32_is_converted_to_hex() {
        assert_eq!(normalize(BASE32).as_deref(), Some(HEX));
        assert_eq!(normalize(&BASE32.to_ascii_lowercase()).as_deref(), Some(HEX));
        assert_eq!(normalize(OTHER_BASE32).as_deref(), Some(OTHER_HEX));
    }

    #[test]
    fn invalid_values_are_rejected() {
        for bad in ["", "SHORT", &HEX[..39], &format!("{HEX}0"), "Z".repeat(40).as_str(), "1".repeat(32).as_str()] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
    }

    #[test]
    fn hex_and_base32_round_trip() {
        assert_eq!(hex_to_base32(HEX).as_deref(), Some(BASE32));
        assert_eq!(hex_to_base32(HEX_LOWER).as_deref(), Some(BASE32));
        assert_eq!(hex_to_base32(OTHER_HEX).as_deref(), Some(OTHER_BASE32));
        assert_eq!(hex_to_base32("nothex"), None);
        assert!(is_base32(BASE32));
        assert!(!is_base32(HEX));
        assert!(is_hex(HEX));
    }
}
