//! 违禁过滤的装配与降级行为（移植自 Kotlin `SensitiveFilterTest`）。
//!
//! 词表和 IP 段**来源不同**，测试也分开覆盖：
//! - **词表**编在二进制里，不联网。多数用例注入小词表测装配，不依赖真实词库内容
//!   ——否则断言就得依赖词库恰好有哪些词，它一更新测试就红；
//! - **IP 段**仍然联网拉。降级链（缓存 → 旧内存表）都落在它身上，因为它是唯一还会失败的那份数据。
//!
//! 最要紧的两条性质：
//! 1. **拉不到数据不能阻断服务**——回退到磁盘缓存、再回退到旧的内存表；
//! 2. **IP 段拿不到时不过滤任何人**。这是有意的取舍：合规兜底不该让所有人搜不了。
//!
//! 与 Kotlin 版的差异：来源地址是 `Option<IpAddr>`（Kotlin 收主机名字符串，畸形地址在
//! [`client_ip`] 那一步就变成 `None`）；词表在构造时装上，没有 `loadBuiltin`；`start` 对应
//! [`SensitiveFilter::spawn_refresh`]。

use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use torrent_spider::TorrentInfo;
use torrent_spider::testing::FakeHttp;
use torrent_spider_server::sensitive::client_ip;
use torrent_spider_server::sensitive::lexicon::SensitiveLexicon;
use torrent_spider_server::{SensitiveConfig, SensitiveFilter};

/// 注入用的词表。用词库里的真实词，但不依赖打包的那份。
const TEST_WORDS: [&str; 2] = ["奶子", "写真"];

/// 一份最小的「大陆 IP 段」：1.2.3.0/24。
const IP_LIST: &str = "1.2.3.0/24\n";

const IP_LIST_URL: &str = "https://example.test/ips.txt";

/// 缓存文件名，与 `src/sensitive.rs` 保持一致。
const CACHE_FILE: &str = "china_ip_list.txt";

/// 只认 IP 段这一条路由。故意不给词库配路由：真有人加回联网拉词库，请求记录里会看得到。
fn serving_ips(ips: &str) -> Arc<FakeHttp> {
    FakeHttp::new().route(IP_LIST_URL, ips)
}

// ---------- 装配 ----------

fn config(cache_dir: &Path, enabled: bool, force_china: bool) -> SensitiveConfig {
    SensitiveConfig {
        enabled,
        force_china,
        ip_list_url: IP_LIST_URL.to_owned(),
        cache_dir: cache_dir.to_path_buf(),
        ..SensitiveConfig::default()
    }
}

/// 注入测试词表的过滤器（默认开启、不强制大陆）。
fn filter_of(dir: &TempDir, http: Arc<FakeHttp>) -> SensitiveFilter {
    filter_with(dir, http, true, false, &TEST_WORDS)
}

fn filter_with(
    dir: &TempDir,
    http: Arc<FakeHttp>,
    enabled: bool,
    force_china: bool,
    words: &[&str],
) -> SensitiveFilter {
    SensitiveFilter::with_lexicon(config(dir.path(), enabled, force_china), http, SensitiveLexicon::build(words))
}

/// 走真实构造路径（装内置词库）的过滤器。
fn builtin_filter(dir: &TempDir, http: Arc<FakeHttp>, enabled: bool) -> SensitiveFilter {
    SensitiveFilter::new(config(dir.path(), enabled, false), http)
}

fn item(title: &str) -> TorrentInfo {
    TorrentInfo { title: Some(title.to_owned()), src: 6, ..TorrentInfo::default() }
}

fn ip(s: &str) -> Option<IpAddr> {
    Some(s.parse().unwrap())
}

fn titles(items: &[TorrentInfo]) -> Vec<&str> {
    items.iter().filter_map(|i| i.title.as_deref()).collect()
}

// ---------- 内置词库（不联网） ----------

#[tokio::test]
async fn 装好内置词库后即使完全联不上网过滤照样有词表() {
    // 以前词库靠联网拉，拉不到就一条词都没有、谁都不拦。现在编在二进制里，最差也是用它
    let dir = tempfile::tempdir().unwrap();
    let filter = builtin_filter(&dir, FakeHttp::offline(), true);
    filter.refresh().await; // IP 段拉不到，大陆表为空

    assert!(filter.status(None).lexicon_entries > 0, "内置词库应当已装上");
    // IP 段拿不到 → is_china 恒为 false → 不过滤。这是 IP 段那边的降级，与词库无关
    assert!(!filter.is_china(ip("1.2.3.9")));
}

#[tokio::test]
async fn 词库不联网_不会向_lexicon_地址发请求() {
    // 钉住「词库来源已经改成本地」：哪天有人加回联网逻辑，请求记录里就会多出来
    let dir = tempfile::tempdir().unwrap();
    let http = serving_ips(IP_LIST);
    let filter = builtin_filter(&dir, Arc::clone(&http), true);
    filter.refresh().await;

    assert!(filter.status(None).lexicon_entries > 0, "词表应当来自内置那份，而不是网络");
    let urls = http.requested_urls();
    assert!(urls.iter().all(|u| !u.contains("lexicon")), "不该请求词库：{urls:?}");
    assert_eq!(urls, vec![IP_LIST_URL.to_owned()], "只该拉 IP 段");
}

#[tokio::test]
async fn 关掉开关时不装载词库也不过滤任何人() {
    let dir = tempfile::tempdir().unwrap();
    let http = FakeHttp::offline();
    let filter = builtin_filter(&dir, Arc::clone(&http), false);
    filter.refresh().await;

    assert_eq!(filter.status(None).lexicon_entries, 0, "关掉开关就不该装词库");
    let items = vec![item("奶子大合集")];
    assert_eq!(filter.filter(items.clone(), "linux", ip("1.2.3.9")), items);
    assert!(http.requested_urls().is_empty(), "关掉开关连 IP 段都不该拉");
}

// ---------- IP 段的刷新与降级 ----------

#[tokio::test]
async fn ip_段拉取成功后就绪() {
    let dir = tempfile::tempdir().unwrap();
    let filter = filter_of(&dir, serving_ips(IP_LIST));
    assert!(!filter.is_ready(), "还没拉 IP 段，不该是就绪状态");

    filter.refresh().await;

    assert!(filter.is_ready());
    assert!(filter.is_china(ip("1.2.3.9")), "1.2.3.0/24 里的地址应当判定为大陆");
    assert!(!filter.is_china(ip("8.8.8.8")));
}

#[tokio::test]
async fn ip_段拉取失败时落盘缓存兜底() {
    let dir = tempfile::tempdir().unwrap();
    // 第一次成功，把 IP 段写进缓存
    filter_of(&dir, serving_ips(IP_LIST)).refresh().await;

    // 第二次网络全挂：应当从缓存读到同样的数据，仍然就绪
    let offline = filter_of(&dir, FakeHttp::offline());
    offline.refresh().await;

    assert!(offline.is_ready(), "网络不通时应当回退到磁盘缓存");
    assert!(offline.is_china(ip("1.2.3.9")));
}

#[tokio::test]
async fn 缓存也没有时不就绪但也不崩() {
    let dir = tempfile::tempdir().unwrap();
    let filter = filter_of(&dir, FakeHttp::offline());
    filter.refresh().await;

    assert!(!filter.is_ready(), "IP 段拿不到就不算就绪");
    assert!(!filter.is_china(ip("1.2.3.9")));
}
#[tokio::test]
async fn ip_段解析出零条时当失败处理_不覆盖已经生效的表() {
    // 「一条都没解析出来」和「拿到了生效的表」是两回事。当成成功会让过滤静默失效，
    // 而日志上一切正常——那正是最难发现的一类故障
    let dir = tempfile::tempdir().unwrap();
    let http = serving_ips(IP_LIST);
    let filter = filter_of(&dir, Arc::clone(&http));
    filter.refresh().await;
    assert!(filter.is_china(ip("1.2.3.9")));

    // 第二次拉到一份全是垃圾的 IP 清单：旧表必须留着，不能被空表顶掉
    let garbage = "not an ip\n# 注释\n\n";
    http.set_route(IP_LIST_URL, garbage);
    filter.refresh().await;
    assert!(filter.is_china(ip("1.2.3.9")), "解析不出区间时应当保留旧表，而不是换成空表");

    // Rust 版只在解析成功后才写缓存，上面那次其实是缓存兜住的。删掉缓存再来一次，
    // 钉住最后一层「沿用内存旧表」，也钉住垃圾原文不会落盘
    let cache = dir.path().join(CACHE_FILE);
    assert_eq!(std::fs::read_to_string(&cache).unwrap(), IP_LIST, "垃圾清单不该覆盖缓存");
    std::fs::remove_file(&cache).unwrap();
    filter.refresh().await;
    assert!(filter.is_china(ip("1.2.3.9")), "缓存也没有时应当沿用内存里的旧表");
    assert!(!cache.exists(), "解析不出区间的原文不该写进缓存");
}

#[tokio::test]
async fn 缓存写的是原文_解析口径改了还能重新解析() {
    let dir = tempfile::tempdir().unwrap();
    filter_of(&dir, serving_ips(IP_LIST)).refresh().await;

    // 缓存里应当是那份 IP 清单原文，而不是某种二进制快照
    let cached = std::fs::read_to_string(dir.path().join(CACHE_FILE)).unwrap();
    assert_eq!(cached, IP_LIST);
}

// ---------- 过滤 ----------

#[tokio::test]
async fn 大陆_ip_会被过滤_境外_ip_原样返回() {
    let dir = tempfile::tempdir().unwrap();
    let filter = filter_of(&dir, serving_ips(IP_LIST));
    filter.refresh().await;

    let items = vec![item("奶子大合集"), item("ubuntu-24.04.iso")];

    assert_eq!(titles(&filter.filter(items.clone(), "linux", ip("1.2.3.9"))), vec!["ubuntu-24.04.iso"]);
    assert_eq!(filter.filter(items.clone(), "linux", ip("8.8.8.8")), items, "境外来源不受影响");
}

#[tokio::test]
async fn 来源地址缺失或畸形时不当作大陆_不拦() {
    // 拿不准就不拦。硬解一个畸形地址可能拦到无辜用户。
    // Rust 版的解析在 `client_ip` 那一步：畸形值（含 `localhost` 这类主机名）在那里就变成 `None`，
    // 过滤器只会看到「没有来源」。`/api/health` 的 `client_ip` 就是为看穿这点留的
    let dir = tempfile::tempdir().unwrap();
    let filter = filter_of(&dir, serving_ips(IP_LIST));
    filter.refresh().await;

    let items = vec![item("奶子大合集")];
    assert_eq!(filter.filter(items.clone(), "linux", None), items, "没有来源时不该拦");
    for host in ["", "   ", "not-an-ip", "1.2.3", "999.1.2.3", "localhost"] {
        // 没有对端地址、只有一段畸形 XFF：解析不出来就是 None，不会硬凑出一个地址
        let client = client_ip(None, Some(host), true);
        assert_eq!(client, None, "畸形来源不该解析出地址：{host:?}");
        assert_eq!(filter.filter(items.clone(), "linux", client), items, "不该拦来源：{host:?}");
    }
}

#[tokio::test]
async fn 大陆_ip_上搜违禁词返回空列表() {
    let dir = tempfile::tempdir().unwrap();
    let filter = filter_of(&dir, serving_ips(IP_LIST));
    filter.refresh().await;

    let items = vec![item("随便什么"), item("无关内容")];
    assert_eq!(filter.filter(items, "写真", ip("1.2.3.9")), Vec::<TorrentInfo>::new());
}

#[tokio::test]
async fn ip_段没就绪时一个都不拦() {
    // 「拉不到就不过滤任何人」的代价要如实钉住：这是有意选的默认，不是「碰巧没拦住」
    let dir = tempfile::tempdir().unwrap();
    let filter = filter_of(&dir, FakeHttp::offline());
    filter.refresh().await;

    let items = vec![item("奶子大合集"), item("写真集")];
    assert_eq!(filter.filter(items.clone(), "任何词", ip("1.2.3.9")), items);
}
// ---------- force_china ----------

#[tokio::test]
async fn force_china_下不联网也把任何来源当大陆() {
    // 本地自测用：localhost 不是大陆 IP，没有这个开关就没法在本地验过滤
    let dir = tempfile::tempdir().unwrap();
    let http = FakeHttp::offline();
    let filter = filter_with(&dir, Arc::clone(&http), true, true, &TEST_WORDS);
    assert!(filter.is_china(None), "连来源都没有时也算大陆");
    assert!(filter.is_china(ip("8.8.8.8")));
    assert!(filter.is_china(ip("::1")));
    assert!(http.requested_urls().is_empty(), "判定不该触发任何请求");
}

// ---------- 诊断状态 ----------

#[tokio::test]
async fn status_报出这次请求看到的来源与判定结论() {
    // 排查「过滤怎么没生效」全靠这几个字段：来源地址 + 算出来的结论，以及两份数据的条数
    let dir = tempfile::tempdir().unwrap();
    let filter = filter_of(&dir, serving_ips(IP_LIST));
    filter.refresh().await;

    let status = filter.status(ip("1.2.3.9"));
    assert!(status.enabled);
    assert!(status.ready);
    assert!(!status.force_china);
    assert_eq!(status.lexicon_entries, 2, "注入的词表里「奶子」「写真」两条");
    assert_eq!(status.china_ranges, 1, "1.2.3.0/24 合并成一条区间");
    assert_eq!(status.client_ip.as_deref(), Some("1.2.3.9"));
    assert!(status.is_china);

    // 同一个过滤器换一个来源，结论应当跟着变——这正是内网手机那个场景
    let foreign = filter.status(ip("192.168.18.169"));
    assert_eq!(foreign.client_ip.as_deref(), Some("192.168.18.169"));
    assert!(!foreign.is_china, "内网地址匹配不上公网大陆段，如实报 false");
    assert!(foreign.ready, "能力没变，只是这次请求不在过滤范围内");
}

#[tokio::test]
async fn status_在_ip_段没就绪时如实报_false() {
    let dir = tempfile::tempdir().unwrap();
    let filter = filter_of(&dir, FakeHttp::offline());
    filter.refresh().await;

    let status = filter.status(ip("1.2.3.9"));
    assert!(!status.ready, "IP 段没拿到就不算就绪");
    assert!(!status.is_china, "没数据就拦不了，别报成 true 让人以为在生效");
}

#[tokio::test]
async fn status_在词库为空时也不报就绪() {
    // 词库和 IP 段缺一份都过滤不出东西。这里词库空、IP 段有，仍然不该报就绪
    let dir = tempfile::tempdir().unwrap();
    let filter = filter_with(&dir, serving_ips(IP_LIST), true, false, &[]);
    filter.refresh().await;

    let status = filter.status(ip("1.2.3.9"));
    assert!(!status.ready, "词库为空时不该报就绪");
    assert!(filter.is_china(ip("1.2.3.9")), "IP 段能力本身没变");
}

#[tokio::test]
async fn force_china_下_status_仍报真实来源_只是结论为_true() {
    // 自测开关不该把诊断一起糊掉：来源照实报，结论才是被强行改写的那个
    let dir = tempfile::tempdir().unwrap();
    let filter = filter_with(&dir, FakeHttp::offline(), true, true, &TEST_WORDS);

    let status = filter.status(ip("8.8.8.8"));
    assert_eq!(status.client_ip.as_deref(), Some("8.8.8.8"));
    assert!(status.is_china);
    assert!(status.force_china);
}

#[tokio::test]
async fn 关掉开关时_status_报未启用() {
    let dir = tempfile::tempdir().unwrap();
    let filter = filter_with(&dir, FakeHttp::offline(), false, false, &TEST_WORDS);

    let status = filter.status(ip("1.2.3.9"));
    assert!(!status.enabled);
    assert!(!status.is_china, "开关关掉就不过滤，哪怕来源是大陆");
}

// ---------- 后台刷新 ----------

#[tokio::test]
async fn spawn_refresh_在关掉开关时不起任何任务() {
    let dir = tempfile::tempdir().unwrap();
    let http = FakeHttp::offline();
    let filter = Arc::new(builtin_filter(&dir, Arc::clone(&http), false));

    assert!(filter.spawn_refresh().is_none(), "关掉开关就不该起后台任务");
    assert!(!filter.is_ready());
    assert!(http.requested_urls().is_empty());
}

#[tokio::test]
async fn spawn_refresh_会立刻刷一次_ip_段() {
    let dir = tempfile::tempdir().unwrap();
    let filter = Arc::new(builtin_filter(&dir, serving_ips(IP_LIST), true));

    // 刷新间隔是默认的一天，所以这一轮验证的确实是「启动时那一次」
    let handle = filter.spawn_refresh().expect("开启时应当起后台任务");

    // 轮询而不是掐一个固定 sleep：固定值小了会随机红，大了白等
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !filter.is_ready() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handle.abort();

    assert!(filter.status(None).lexicon_entries > 0, "应当装上内置词库");
    assert!(filter.is_ready(), "应当立刻刷一次 IP 段，而不是等第一个刷新周期");
}
