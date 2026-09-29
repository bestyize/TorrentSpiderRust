//! 大陆 IP 判定的行为契约（移植自 Kotlin `IpRangeSetTest`）。
//!
//! 这块的价值全在「准」上：判错了就是拦到不该拦的人（或放走该拦的），而出错时表面上
//! 一切正常。所以区间端点、CIDR 折叠、畸形输入都要钉住。
//!
//! 与 Kotlin 版的差别（Rust 版是有意改进）：
//! - 地址交给 `std::net` 解析，内嵌 IPv4 的 IPv6（`::ffff:1.2.3.4`）现在能解析；
//! - IPv6 存完整 128 位，原「IPv6 区间按前 64 位判定」改成「/64 及更长前缀精确判定」；
//! - `readLong` 与「长度不是 4/16 的字节串」两组用例不再适用（没有字节数组入口，
//!   类型系统保证只有 IPv4/IPv6），换成 u32/u128 边界与溢出用例。

use std::net::IpAddr;

use torrent_spider_server::sensitive::ip_range::{Cidr, IpRangeSet, parse_cidr};

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap_or_else(|_| panic!("用例里的地址应当解析得动：{text}"))
}

fn ranges_of(cidrs: &[&str]) -> IpRangeSet {
    IpRangeSet::from_cidrs(cidrs.iter().filter_map(|c| parse_cidr(c)))
}

fn contains(set: &IpRangeSet, addr: &str) -> bool {
    set.contains(ip(addr))
}

// ---------- CIDR 解析 ----------

#[test]
fn 地址族与前缀长度分开报告() {
    // Kotlin 的 `bits` 报告 32/128 用来分表；这里对应 network 的地址族，前缀是另一个概念
    let v4 = parse_cidr("1.2.3.0/24").unwrap();
    assert!(v4.network.is_ipv4());
    assert_eq!(v4.prefix, 24);
    let v6 = parse_cidr("2001:db8::/32").unwrap();
    assert!(v6.network.is_ipv6());
    assert_eq!(v6.prefix, 32);
}

#[test]
fn 地址与区间端点按闭区间算() {
    let set = ranges_of(&["1.2.3.0/24"]);
    assert!(contains(&set, "1.2.3.0"), "网络地址本身在区间内");
    assert!(contains(&set, "1.2.3.255"), "广播地址也在区间内");
    assert!(contains(&set, "1.2.3.128"));
    assert!(!contains(&set, "1.2.4.0"), "紧邻的下一条不该被卷进来");
    assert!(!contains(&set, "1.2.2.255"));
}

#[test]
fn 前缀不是8的整数倍也能算对() {
    let set = ranges_of(&["1.2.0.0/22"]);
    assert!(contains(&set, "1.2.0.1"));
    assert!(contains(&set, "1.2.3.255"), "/22 覆盖 1.2.0.0 - 1.2.3.255");
    assert!(!contains(&set, "1.2.4.0"));
}

#[test]
fn 解析时把主机位清零() {
    let cidr = parse_cidr("1.2.3.4/24").unwrap();
    assert_eq!(cidr.network, ip("1.2.3.0"), "network 应当已清零主机位");
    let set = ranges_of(&["1.2.3.4/24"]);
    assert!(contains(&set, "1.2.3.0"), "主机位必须先清零再取区间");

    let v6 = parse_cidr("2001:db8::1234/32").unwrap();
    assert_eq!(v6.network, ip("2001:db8::"));
}

#[test]
fn 手工构造的脏_cidr_也按掩码折叠() {
    // Cidr 字段是公开的，调用方可能不经 parse_cidr 直接构造
    let set = IpRangeSet::from_cidrs([Cidr { network: ip("1.2.3.4"), prefix: 24 }]);
    assert!(contains(&set, "1.2.3.0"));
    let bad = IpRangeSet::from_cidrs([Cidr { network: ip("1.2.3.4"), prefix: 33 }]);
    assert!(bad.is_empty(), "前缀越界的条目跳过");
}

#[test]
fn 畸形输入一律跳过而不是抛异常() {
    for bad in [
        "",
        "   ",
        "# 注释",
        "; 注释",
        "1.2.3.4",         // 没有 /前缀
        "1.2.3.4/",        // 前缀为空
        "1.2.3.4/abc",     // 前缀不是数字
        "1.2.3.4/+24",     // 带符号的前缀
        "1.2.3.4/-1",      // 负前缀
        "1.2.3.4/33",      // 前缀超出 IPv4 宽度
        "1.2.3.4/256",     // 超出 u8
        "2001:db8::/129",  // 前缀超出 IPv6 宽度
        "1.2.3/24",        // 段数不对
        "1.2.3.256/24",    // 段值越界
        "1.2.3.4.5/24",    // 段数多了
        "1.2.3.-1/24",     // 负数
        "300.1.2.3/24",    // 首段越界
        "0001.2.3.4/24",   // 段超长（Kotlin 靠长度卡，std 同样拒绝）
        "01.2.3.4/24",     // 前导零：std 拒绝（避免与八进制写法混淆）
        "1.2.3.4 /24",     // 地址尾部空白
        "x1.2.3.4/24",     // 地址前有杂字符
        "1.2.3.4/24 junk", // 前缀后有杂字符
        "1.2.3.4/24/8",    // 两个斜杠
        "example.com/24",  // 不是地址
    ] {
        assert_eq!(parse_cidr(bad), None, "应当跳过：{}", if bad.is_empty() { "<空>" } else { bad });
    }
}

#[test]
fn 整行首尾空白与前缀两侧空白可以容忍() {
    // 与 Kotlin 一致：整行先 trim，前缀部分再 trim
    assert_eq!(parse_cidr("  1.2.3.0/24\t"), parse_cidr("1.2.3.0/24"));
    assert_eq!(parse_cidr("1.2.3.0/ 24"), parse_cidr("1.2.3.0/24"));
    assert!(parse_cidr("0.0.0.0/0").is_some());
    assert!(parse_cidr("::/0").is_some());
    assert!(parse_cidr("::/128").is_some());
}

// ---------- 区间折叠 ----------

#[test]
fn 重叠区间合并成一条() {
    let set = ranges_of(&["1.2.3.0/24", "1.2.0.0/16"]);
    assert_eq!(set.len(), 1, "被后者完全包含，应当只剩一条");
    assert!(contains(&set, "1.2.99.99"));
}

#[test]
fn 相邻区间也合并() {
    let set = ranges_of(&["1.2.0.0/24", "1.2.1.0/24"]);
    assert_eq!(set.len(), 1);
    assert!(contains(&set, "1.2.0.255"));
    assert!(contains(&set, "1.2.1.0"));
}

#[test]
fn 不相邻的区间保持分开() {
    let set = ranges_of(&["1.1.0.0/24", "1.3.0.0/24"]);
    assert_eq!(set.len(), 2);
    assert!(!contains(&set, "1.2.0.0"), "中间的空档不该被填上");
}

#[test]
fn 乱序输入也能折叠() {
    let set = ranges_of(&["1.2.5.0/24", "1.2.0.0/16", "1.2.3.0/24"]);
    assert_eq!(set.len(), 1, "/16 把两条 /24 都吞了，应当只剩一条");
    assert!(contains(&set, "1.2.0.0"));
    assert!(contains(&set, "1.2.255.255"), "/16 的末尾");
    assert!(!contains(&set, "1.3.0.0"));
}

#[test]
fn 相邻但不相交的段不会误合并() {
    // 合并判据写成「同前缀」就会把它们连起来，凭空拦下 1.1.1.0/24
    let set = ranges_of(&["1.1.0.0/24", "1.2.0.0/24"]);
    assert_eq!(set.len(), 2);
    assert!(!contains(&set, "1.1.1.0"), "中间的空档不该被填上");
}

#[test]
fn 地址空间末尾的合并不会溢出() {
    // end + 1 在 u32::MAX / u128::MAX 处会溢出；饱和加法下合并照常
    let set = ranges_of(&["255.255.255.254/32", "255.255.255.255/32", "0.0.0.0/1"]);
    assert_eq!(set.len(), 2);
    assert!(contains(&set, "255.255.255.255"));
    assert!(!contains(&set, "255.255.255.253"));

    let all = ranges_of(&["255.255.255.0/24", "0.0.0.0/0", "255.255.255.255/32"]);
    assert_eq!(all.len(), 1);
    assert!(contains(&all, "0.0.0.0"));
    assert!(contains(&all, "255.255.255.255"));

    let v6 = ranges_of(&["ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffe/128", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128"]);
    assert_eq!(v6.len(), 1);
    assert!(contains(&v6, "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"));
    assert!(!contains(&v6, "ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffd"));

    let v6_all = ranges_of(&["::/0", "ffff::/16"]);
    assert_eq!(v6_all.len(), 1);
    assert!(contains(&v6_all, "::"));
    assert!(contains(&v6_all, "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"));
}

#[test]
fn parse_list_跳过坏行() {
    let set = IpRangeSet::parse_list("# 大陆段\n1.2.0.0/24\n\ngarbage\n1.2.1.0/24\r\n2001:db8::/32\n1.2.3.4/99\n");
    assert_eq!(set.len(), 2, "两条相邻 IPv4 合并 + 一条 IPv6");
    assert!(contains(&set, "1.2.1.200"));
    assert!(contains(&set, "2001:db8::1"));
}

// ---------- 地址族 ----------

#[test]
fn ipv4_与_ipv6_分两张表互不命中() {
    let only_v4 = ranges_of(&["1.2.3.0/24"]);
    assert!(!contains(&only_v4, "::1:203:0"), "IPv6 不该命中 IPv4 区间");
    assert!(!contains(&only_v4, "::102:300"), "IPv4-compatible（非 mapped）写法也不该命中");

    let only_v6 = ranges_of(&["1:203:0::/48"]);
    assert!(!contains(&only_v6, "1.2.3.0"), "IPv4 不该命中 IPv6 区间");
}

#[test]
fn ipv4_mapped_ipv6_按_ipv4_表判定() {
    // 双栈 socket 上的 IPv4 客户端地址长成 ::ffff:a.b.c.d
    let set = ranges_of(&["1.2.3.0/24"]);
    assert!(contains(&set, "::ffff:1.2.3.4"));
    assert!(contains(&set, "::ffff:102:3ff"), "十六进制写法是同一个地址");
    assert!(!contains(&set, "::ffff:1.2.4.0"));
}

#[test]
fn 内嵌_ipv4_的_ipv6_可以解析() {
    // Kotlin 手写解析器不支持而跳过；std 解析是准确的，所以这里收下
    let cidr = parse_cidr("::ffff:1.2.3.0/120").unwrap();
    assert_eq!(cidr.network, ip("::ffff:102:300"));
    let set = ranges_of(&["::ffff:1.2.3.0/120"]);
    assert!(contains(&set, "::ffff:1.2.3.9"));
}

#[test]
fn 空集合不包含任何地址() {
    let empty = IpRangeSet::from_cidrs([]);
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert!(!contains(&empty, "1.2.3.4"));
    assert!(!contains(&empty, "::1"));
    assert_eq!(empty, IpRangeSet::default());
}

// ---------- IPv6 ----------

#[test]
fn ipv6_完整写法可以解析() {
    let cidr = parse_cidr("2001:0db8:0000:0000:0000:0000:0000:0001/128").unwrap();
    assert_eq!(cidr.network, ip("2001:db8::1"));
    let set = IpRangeSet::from_cidrs([cidr]);
    assert!(contains(&set, "2001:db8::1"));
    assert!(!contains(&set, "2001:db8::2"));
}

#[test]
fn ipv6_缩写写法可以解析() {
    assert_eq!(parse_cidr("2001:db8::1/128").unwrap().network, ip("2001:0db8:0:0:0:0:0:1"));
    assert_eq!(parse_cidr("::1/128").unwrap().network, ip("0:0:0:0:0:0:0:1"));
    assert_eq!(parse_cidr("::/0").unwrap().network, ip("0:0:0:0:0:0:0:0"));
}

#[test]
fn ipv6_畸形写法跳过() {
    for bad in [
        "2001:db8::1::2",    // 两个 ::
        "1:2:3:4:5:6:7",     // 没有 :: 却只有 7 组
        "1:2:3:4:5:6:7:8:9", // 9 组
        "1:2:3:4:5:6:7:8::", // :: 没压缩任何东西
        "12345::1",          // 单组超 4 位
        "gggg::1",           // 非十六进制
    ] {
        assert_eq!(parse_cidr(&format!("{bad}/64")), None, "应当解析失败：{bad}");
    }
}

#[test]
fn ipv6_32_前缀覆盖整段() {
    let set = ranges_of(&["2001:db8::/32"]);
    assert!(contains(&set, "2001:db8::1"));
    assert!(contains(&set, "2001:db8:ffff:ffff:ffff:ffff:ffff:ffff"));
    assert!(!contains(&set, "2001:db9::1"), "下一个 /32 不该被卷进来");
    assert!(!contains(&set, "2001:db7:ffff:ffff:ffff:ffff:ffff:ffff"));
}

#[test]
fn ipv6_64_及更长前缀精确判定() {
    // 取代 Kotlin「IPv6 区间按前 64 位判定」：完整 128 位后，前 64 位相同的地址也能分开
    let set64 = ranges_of(&["2001:db8:1:2::/64"]);
    assert!(contains(&set64, "2001:db8:1:2::"));
    assert!(contains(&set64, "2001:db8:1:2:ffff:ffff:ffff:ffff"));
    assert!(!contains(&set64, "2001:db8:1:3::"));

    let set80 = ranges_of(&["2001:db8::1:0:0:0/80"]);
    assert!(contains(&set80, "2001:db8::1:0:0:0"));
    assert!(contains(&set80, "2001:db8::1:ffff:ffff:ffff"));
    assert!(!contains(&set80, "2001:db8::2:0:0:0"), "前 64 位相同，但不在 /80 里");
    assert!(!contains(&set80, "2001:db8::0:ffff:ffff:ffff"));

    let set128 = ranges_of(&["2001:db8::5/128"]);
    assert_eq!(set128.len(), 1);
    assert!(contains(&set128, "2001:db8::5"));
    assert!(!contains(&set128, "2001:db8::4"));
    assert!(!contains(&set128, "2001:db8::6"));
}

#[test]
fn ipv6_相邻区间合并() {
    let set = ranges_of(&["2001:db8::/33", "2001:db8:8000::/33", "2001:db9::/32"]);
    assert_eq!(set.len(), 1);
    assert!(contains(&set, "2001:db9:ffff::"));
    let apart = ranges_of(&["2001:db8::/32", "2001:dba::/32"]);
    assert_eq!(apart.len(), 2);
    assert!(!contains(&apart, "2001:db9::"));
}
