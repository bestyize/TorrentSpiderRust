# TorrentSpiderRust

[TorrentSpider](https://github.com/bestyize/TorrentSpider)（`parsing-robustness` 分支）的 Rust 实现：
同时搜索 11 个磁力索引站，提供可复用的核心库、HTTP 接口和一个网页。只做磁力**搜索**，不做 BT 下载。

- **搜索优化**（网页上一个开关，默认开）：结果按与关键词的相关性排序；补搜数字的另一种写法
  （`复仇者联盟四` 也搜 `复仇者联盟4`）；隐藏与关键词无关的结果。
- **违禁内容过滤**：请求方是中国大陆 IP 时，丢掉命中违禁词的结果。词表编在二进制里，大陆 IP 段联网拉取并每天刷新。

## 结构

一个 Cargo workspace，两个 crate：核心库不依赖任何 Web 框架，服务端只做 HTTP 这一层。

```
Cargo.toml                       workspace 与共用依赖版本
deploy/                          systemd 单元、配置模板、安装与运维脚本
docs/
  api.md                         HTTP 接口契约
  deployment.md                  部署到 Linux 服务器
  requirements.md                需求说明
crates/
  torrent-spider/                核心库
    src/
      lib.rs                     公开 API 的导出
      client.rs                  TorrentSpiderClient：库的主入口
      model.rs  error.rs         TorrentInfo / TorrentSource、SpiderError
      site.rs   site/            站点：编号与清单（catalog）、域名表（endpoint）、
                                 装配（registry）、每站一个解析器（parsers/）
      search.rs search/          搜索编排：多源并发（aggregate）、相关性排序与过滤（relevance）
      magnet.rs magnet/          磁力链接：MagnetLink、info hash（hash）、.torrent 缓存源（torrent_file）
      net.rs    net/             基础设施：HttpClient trait、reqwest 实现、HTML 取值、百分号编解码
      testing.rs                 测试用假 HTTP 与固定哈希（`testing` feature）
    tests/
      sites/                     11 个站点的离线解析测试（一个测试程序）
      relevance.rs               相关性排序与过滤
    examples/search.rs           命令行搜索示例
  torrent-spider-server/         axum 服务
    src/
      main.rs                    启动：读配置、装配、起服务
      lib.rs                     AppState 与路由
      config.rs                  从环境变量读取配置
      gateway.rs                 接口层依赖的核心库能力（trait，便于测试注入）
      api.rs    api/             每个端点一个模块：search / magnet / meta，以及参数校验、错误码
      sensitive.rs sensitive/    违禁内容过滤：大陆 IP 段（ip_range）、违禁词表（lexicon）
      web.rs                     静态页路由
    static/                      网页（index.html / app.js / styles.css），编译期嵌入
    data/                        内置违禁词表及其来源，见 data/README.md
    tests/
      api/                       接口行为测试（按端点分模块）
      sensitive/                 过滤服务、IP 段、词表的测试
```

单元测试写在各源文件末尾的 `#[cfg(test)] mod tests` 里；`tests/` 下只放从外部调用公开 API 的集成测试。

核心库不绑定 HTTP 实现，所有请求都通过注入的 `HttpClient` trait 发出。
`reqwest` feature 提供开箱即用的实现；关掉它就只剩纯解析逻辑，可以接自己的 HTTP 栈。

## 快速开始

需要 Rust 1.88+。

```sh
cargo run -p torrent-spider-server
# 打开 http://localhost:55555
```

多数站点在墙外，需要代理时：

```sh
TORRENT_PROXY=127.0.0.1:7897 cargo run -p torrent-spider-server
```

| 环境变量 | 默认 | 说明 |
|---|---|---|
| `PORT` | `55555` | 监听端口，非法值直接启动失败 |
| `HOST` | `0.0.0.0` | 监听地址，只想本机访问时设为 `127.0.0.1` |
| `TORRENT_PROXY` | 无 | 出站代理，`host:port`（HTTP）或完整地址如 `socks5://127.0.0.1:7897`；拉 IP 段也走它 |
| `SENSITIVE_FILTER` | `1` | 违禁内容过滤总开关，`0` 关闭 |
| `SENSITIVE_ASSUME_CHINA` | `0` | `1` 把所有请求方都当成大陆 IP，**只用于本地自测**，公网部署开了等于对全世界过滤 |
| `TRUST_FORWARDED_FOR` | `0` | 部署在反向代理后面时设 `1`，改用 `X-Forwarded-For` 最后一段判定来源；直连部署开了它，任何人加一行该头就能绕过过滤 |
| `SENSITIVE_CACHE_DIR` | 系统临时目录下的 `torrentspider-sensitive` | 大陆 IP 段的落盘缓存 |
| `SENSITIVE_IP_LIST_URL` | 17mon/china_ip_list | 大陆 IP 段来源（每行一条 CIDR） |
| `RUST_LOG` | `info,tower_http=info` | 日志级别 |

本机不是大陆 IP，想看过滤效果时用 `SENSITIVE_ASSUME_CHINA=1` 启动；`/api/health` 的 `filter` 块会报出
这次请求被判成了什么。手机和电脑在同一个 WiFi 下时服务端只看得到局域网地址，也判不出大陆。

```sh
curl -s "http://localhost:55555/api/search?q=ubuntu&src=6"
curl -s "http://localhost:55555/api/search?q=ubuntu"          # 聚合全部来源
curl -s "http://localhost:55555/api/magnet?src=7&detail_url=https%3A%2F%2Fwww.torlock.com%2Ftorrent%2F63489835%2Fubuntu-22-04-desktop-amd64.html"
```

接口、字段和错误码见 [docs/api.md](docs/api.md)。

服务没有鉴权。默认监听 `0.0.0.0` 是为了方便局域网访问；放到公网前请加一层反向代理和访问控制。

## 部署到 Linux 服务器

仓库的 Actions 页里有一个手动触发的 workflow「Build Linux server」：点「Run workflow」选目标，
产物在该次运行页面底部的 Artifacts 里。默认的 `x86_64-unknown-linux-musl` 是静态链接，任意 x86_64 发行版可直接运行。
每次构建都会先跑全部测试，再在 runner 的 systemd 上把发布包真装一遍（安装、升级、坏版本回滚、卸载）。

发布包里带了基于 systemd 的运维脚本：

```sh
tar -xzf torrent-spider-server-0.1.0-x86_64-unknown-linux-musl.tar.gz
cd torrent-spider-server-0.1.0-x86_64-unknown-linux-musl
sudo ./torrent-spider.sh install      # 之后用 torrent-spider status / logs / upgrade / rollback
```

配置、升级回滚、反向代理与排查见 [docs/deployment.md](docs/deployment.md)。

## 作为库使用

```toml
[dependencies]
torrent-spider = { path = "crates/torrent-spider", features = ["reqwest"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use std::sync::Arc;
use torrent_spider::{ReqwestHttpClient, TorrentSpiderClient, TorrentSrc};

let http = ReqwestHttpClient::builder().proxy("http://127.0.0.1:7897").build()?;
let client = TorrentSpiderClient::new(Arc::new(http));

// 单个源：请求失败返回 Err，解析不出条目返回空列表
let items = client.search(TorrentSrc::BtSow, "ubuntu", 1).await?;

// 全部源并发：单个源失败只体现在 statuses 里，结果按相关性排序
let result = client.search_all("ubuntu", 1).await;

// 搜索页没有磁力的站点（TorLock、1337 等）按详情页再取一次
let magnet = client.find_magnet_url(TorrentSrc::TorLock, &items[0].detail_url.clone().unwrap()).await;
```

其他常用入口：

- `search::relevance::rank`、`filter_related`、`search_variants`：相关性排序、过滤无关结果、生成数字写法的检索变体。
- `magnet::MagnetLink::parse`、`magnet::magnet_to_hash`、`magnet::hash_to_magnet`：magnet 与 info hash 互转，十六进制 / Base32 都认。
- `TorrentFileResolver`：按 info hash 逐个探测 `.torrent` 缓存源，返回真正可用的地址。
- `TorrentSpiderClient::with_endpoints`：整体替换站点域名表（镜像站、测试桩）。
- `TorrentServiceRegistry::register`：替换某个站点的解析实现。

命令行示例：`cargo run -p torrent-spider --features reqwest --example search -- ubuntu [站点编号]`。

## 开发

```sh
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features
cargo fmt --all
```

测试全部离线：解析器测试喂固定的 HTML / JSON，服务端测试注入假网关。
站点会改版、会 403、会被墙，依赖真站的测试只会带来噪音。
需要假 HTTP 客户端时用 `torrent_spider::testing::FakeHttp`（在 dev-dependencies 里打开 `testing` feature）。

新增站点：

1. 在 `site/catalog.rs` 的 `TorrentSrc` 里分配新的编号（只能追加），补上标题等元数据；
2. 在 `site/endpoint.rs` 的 `DefaultSiteEndpoints` 里配置域名；
3. 在 `site/parsers/` 下新建一个文件实现 `SearchService`，把请求构造和解析分开，解析写成纯函数；
4. 在 `site/parsers.rs` 的 `create_all` 里注册，并在 `tests/sites/` 下补离线解析测试。

## 已知限制

2026-09 实测，YuHuaGe、Zero（0magnet）、磁力猫启用了 Cloudflare 浏览器验证，纯 HTTP 请求会拿到 403；
Solid（bitsearch.eu）搜中文关键字时也返回 403，英文正常。接口会在对应源的 `error` 里说明原因。
