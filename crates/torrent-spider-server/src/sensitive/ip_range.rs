//! 中国大陆 IP 判定：把 CIDR 清单折成不重叠区间，按起点二分查询。
//!
//! 约 1.1 万条 CIDR 落成两张有序区间表（IPv4 `u32`、IPv6 `u128`）。数据一天才更新一次、
//! 查询每请求一次，二分十几次比较足够，用不着前缀树。
//!
//! 与 Kotlin 版的差别：地址交给 `std::net` 解析（因此也收下 `::ffff:1.2.3.4` 这类内嵌
//! IPv4 写法）；IPv6 存完整 128 位，/64 以上的前缀同样精确（Kotlin 只取前 64 位，
//! 并跳过前缀 > 64 的行）。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// 一条解析成功的 CIDR。`network` 已清掉主机位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    pub network: IpAddr,
    pub prefix: u8,
}

/// 解析一行 `地址/前缀`；空行、注释、畸形地址、前缀越界返回 `None`。
///
/// 公开清单里有边角格式是常态，为一行坏数据中断整次加载会变成「完全没得过滤」，
/// 所以这里只跳过、不报错。前缀只收纯数字（允许两侧空白），`+24` 之类不收。
/// IPv4 的前导零（`01.2.3.4`）由 `std` 拒绝。
pub fn parse_cidr(line: &str) -> Option<Cidr> {
    let text = line.trim();
    if text.is_empty() || text.starts_with('#') || text.starts_with(';') {
        return None;
    }
    let (addr, prefix) = text.split_once('/')?;
    let prefix = prefix.trim();
    if prefix.is_empty() || !prefix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let prefix: u8 = prefix.parse().ok()?;
    let network = match addr.parse::<IpAddr>().ok()? {
        IpAddr::V4(v4) => IpAddr::V4(Ipv4Addr::from(u32::from(v4) & mask_v4(prefix)?)),
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & mask_v6(prefix)?)),
    };
    Some(Cidr { network, prefix })
}

/// 前缀 → 网络掩码；前缀超出地址宽度返回 `None`。`checked_shl` 处理前缀为 0 时移满位宽的情况。
fn mask_v4(prefix: u8) -> Option<u32> {
    (prefix <= 32).then(|| u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0))
}

fn mask_v6(prefix: u8) -> Option<u128> {
    (prefix <= 128).then(|| u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0))
}

/// 一组地址区间（闭区间），IPv4 与 IPv6 分表。不可变，整体替换即可做到无锁读。
///
/// 分表是必须的：合成一张表时 IPv4 的整数值会与某些 IPv6 地址撞上。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IpRangeSet {
    v4: Vec<(u32, u32)>,
    v6: Vec<(u128, u128)>,
}

impl IpRangeSet {
    /// 把 CIDR 折成两张不重叠、不相邻的有序区间表。前缀越界的条目跳过（宁可少拦，不可错拦）。
    pub fn from_cidrs(cidrs: impl IntoIterator<Item = Cidr>) -> Self {
        let mut v4 = Vec::new();
        let mut v6 = Vec::new();
        for cidr in cidrs {
            match cidr.network {
                IpAddr::V4(addr) => {
                    if let Some(mask) = mask_v4(cidr.prefix) {
                        let start = u32::from(addr) & mask;
                        v4.push((start, start | !mask));
                    }
                }
                IpAddr::V6(addr) => {
                    if let Some(mask) = mask_v6(cidr.prefix) {
                        let start = u128::from(addr) & mask;
                        v6.push((start, start | !mask));
                    }
                }
            }
        }
        Self { v4: merge(v4, |end| end.saturating_add(1)), v6: merge(v6, |end| end.saturating_add(1)) }
    }

    /// 逐行解析 CIDR 清单，跳过解析不了的行。
    pub fn parse_list(text: &str) -> Self {
        Self::from_cidrs(text.lines().filter_map(parse_cidr))
    }

    /// IPv4-mapped IPv6（`::ffff:a.b.c.d`）按 IPv4 查：双栈 socket 上的 IPv4 客户端就长这样。
    pub fn contains(&self, addr: IpAddr) -> bool {
        match addr {
            IpAddr::V4(v4) => covers(&self.v4, u32::from(v4)),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                // 清单里若真写了 ::ffff: 段，它落在 v6 表里，也一并查
                Some(v4) => covers(&self.v4, u32::from(v4)) || covers(&self.v6, u128::from(v6)),
                None => covers(&self.v6, u128::from(v6)),
            },
        }
    }

    /// 合并后的区间条数（IPv4 + IPv6）。
    pub fn len(&self) -> usize {
        self.v4.len() + self.v6.len()
    }

    pub fn is_empty(&self) -> bool {
        self.v4.is_empty() && self.v6.is_empty()
    }
}

/// 排序后合并重叠与**相邻**（`end + 1 == 下一条 start`）的区间。
///
/// 相邻也合并：查询结果不变，但少一堆本该连成一体的边界。`succ` 用饱和加法：
/// `end` 已是最大值时后面不可能再有不重叠的区间，合并即正确，也不会溢出。
fn merge<T: Copy + Ord>(mut ranges: Vec<(T, T)>, succ: impl Fn(T) -> T) -> Vec<(T, T)> {
    ranges.sort_unstable();
    let mut merged: Vec<(T, T)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= succ(last.1) => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged.shrink_to_fit();
    merged
}

/// 找最后一个 `start <= value` 的区间，再看 `value` 是否没超过它的 `end`。
/// 区间互不相交且不相邻，这唯一的候选就是全部可能。
fn covers<T: Copy + Ord>(ranges: &[(T, T)], value: T) -> bool {
    let index = ranges.partition_point(|&(start, _)| start <= value);
    index > 0 && value <= ranges[index - 1].1
}
