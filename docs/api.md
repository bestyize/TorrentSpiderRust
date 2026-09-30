# HTTP 接口契约

服务默认监听 `0.0.0.0:55555`，所有接口都是 `GET`，响应都是 JSON（静态页除外）。
字段一律 snake_case；可空字段显式输出 `null`，不省略键。

演进规则：已有字段名、错误码不改；`src` 编号不复用；新字段只追加。

## 数据模型

### TorrentSource

```json
{
  "src": 6,
  "title": "BtSow",
  "sub_title": null,
  "desc": "JSON API driven source",
  "official_url": "https://btsow.live",
  "level": 2
}
```

`src` 是冻结的站点编号（0–10），`level` 为 `1` 表示站点需要浏览器头之类的额外条件。

| src | 站点 | 搜索页自带磁力 |
|---|---|---|
| 0 | Solid（bitsearch.eu） | 是 |
| 1 | YuHuaGe | 否，详情页取 |
| 2 | 海盗湾 | 是 |
| 3 | AcgRip | 只有 `.torrent` 地址 |
| 4 | Zero（0magnet） | 否，详情页取 |
| 5 | 1337 | 否，详情页取 |
| 6 | BtSow | 是 |
| 7 | TorLock | 否，详情页取 |
| 8 | Kitty | 是 |
| 9 | 磁力猫 | 是 |
| 10 | 磁力多 | 是 |

### TorrentInfo

```json
{
  "hash": "E90710273A1E578835CAC80D3E4B3E56A2755663",
  "detail_url": "https://example/detail/abc",
  "magnet_url": "magnet:?xt=urn:btih:E90710273A1E578835CAC80D3E4B3E56A2755663",
  "torrent_url": "https://itorrents.net/torrent/E90710273A1E578835CAC80D3E4B3E56A2755663.torrent",
  "title": "Example Torrent",
  "date": "2026-03-15",
  "last_download_date": null,
  "size": "2.0G",
  "src": 6,
  "download_count": "123",
  "leacher_count": "4",
  "sender_count": "88"
}
```

- `hash` 是规范化后的大写十六进制 BTIH（Base32 写法会被换算）。
- `size`、`date` 与各计数保持源站原文，不做单位换算。
- `torrent_url` 多数是由哈希推导出的缓存地址，不代表缓存里一定有这个文件。
- 空串统一输出为 `null`。

## 接口

### `GET /api/search?q=&page=1&src=all&filter=1&optimize=1`

| 参数 | 说明 |
|---|---|
| `q` | 必填，关键字（首尾空白会被去掉） |
| `page` | 可选，`1`–`100`，默认 `1` |
| `src` | 可选，站点编号或 `all`，默认 `all`（并发聚合全部来源） |
| `filter` | 可选，`1/0` 或 `true/false`，默认 `1`：丢掉与关键词完全沾不上边的条目 |
| `optimize` | 可选，`1/0` 或 `true/false`，默认 `1`：把另一种数字写法也搜一遍再合并 |

```json
{
  "keyword": "复仇者联盟四",
  "page": 1,
  "src": null,
  "items": [ /* TorrentInfo，按相关性排序 */ ],
  "sources": [
    { "src": 0, "title": "Solid", "ok": true, "count": 20, "error": null },
    { "src": 1, "title": "YuHuaGe", "ok": false, "count": 0,
      "error": "GET https://www.yuhuage.win/search/ubuntu-1.html -> HTTP 403（站点启用了 Cloudflare 浏览器验证，纯 HTTP 请求当前无法直接通过）" }
  ],
  "queries": ["复仇者联盟4"]
}
```

- 聚合搜索时 `src` 为 `null`。
- 每个源、每种写法单独计时（25 秒）。单个源失败或超时只标在 `sources` 里，不影响其他源。
- 全部源都失败才返回 502；全部成功但都没结果（或全被过滤掉）是 200 + 空 `items`。
- 单源搜索时这个源失败直接返回 502。

#### 排序

`items` 的顺序有意义，消费方直接按原样展示，不要再按标题重排。单源和聚合都按与 `q` 的相关性排：
依次比较查询词命中比例、命中词的紧凑度、首次命中位置、命中顺序，这几项都打平时才按发布日期新的在前。
比较前会把汉字数字折成阿拉伯数字、全角折成半角（`第十二季` ≡ `第12季`、`ｕｂｕｎｔｕ` ≡ `ubuntu`）。
一条都没命中时保留上游顺序。

#### `filter`：隐藏无关结果

归一化（折数字、折全角、剔掉所有分隔符、小写）后，整段查询要出现在标题里；
查询里最长的那个词（≥4 字符）可以兜底，救回 `the matrix 1999` → `Matrix - 1999 - HMR` 这类标题。
所以 `ubuntu server` 能命中 `ubuntu-server-22.04.iso`，而 `三体` 不会被「标题里恰好有个三字」的条目蹭上。

过滤后 `sources[].count` 仍是上游条数，前端用它与 `items` 长度的差显示「已隐藏 N 条」。

#### `optimize`：补搜另一种数字写法

站点大多按字面检索，`复仇者联盟四` 与 `复仇者联盟4` 的结果几乎不相交。开启后两种写法都发给上游，
按 `hash` > `detail_url` > 小写标题去重合并，保留原写法的那条。`queries` 列出额外多打的写法。

规则很窄：查询里要有汉字，且只折紧挨汉字、数值 1–99 的数字（`第2季` ↔ `第二季`）。
纯拉丁查询（`windows 11`）和四位年份不折。折不出变体时只打一次上游。

`filter` 与 `optimize` 默认值必须一致：网页端是同一个「搜索优化」开关，两者分开传只是为了接口层可以单独关。

#### 违禁内容过滤（服务端行为，没有参数）

请求方是中国大陆 IP 时，标题或详情页地址命中违禁词的条目整条不出现；搜违禁词本身返回 200 + 空 `items`。
被拦的条目连数量都不透露：过滤发生在统计 `sources[].count` 之前，`count - items.length` 里不含它们。

- 请求方地址默认取 TCP 对端地址。只有配了 `TRUST_FORWARDED_FOR=1`（反向代理部署）才改看
  `X-Forwarded-For` 的最后一段。
- 违禁词表编在二进制里，不联网；大陆 IP 段启动时拉取、落盘缓存、每 24 小时刷新。
  IP 段拿不到时不过滤任何人，日志里有 WARN。
- 匹配口径：汉字词至少 2 个汉字，按子串匹配；非汉字词至少 4 个字符，必须整词命中
  （`anal` 不会命中 `analysis`）。拼音缩写、拆字、谐音会漏网，这是为了不误伤正常标题。

### `GET /api/magnet?src=&detail_url=[&magnet_url=]`

TorLock、1337 等站点的搜索结果里没有磁力，要按需抓详情页。

| 参数 | 说明 |
|---|---|
| `src` | 必填，站点编号，不接受 `all` |
| `detail_url` | 必填，只接受 `http(s)`（避免服务端被当成任意请求发起器） |
| `magnet_url` | 可选，搜索结果里已带的磁力；能解出 BTIH 就直接回填，不再抓详情页 |

```json
{
  "src": 7,
  "detail_url": "https://www.torlock.com/torrent/63489835/ubuntu-22-04-desktop-amd64.html",
  "magnet_url": "magnet:?xt=urn:btih:2C6B6858D61DA9543D4231A71DB4B1C9264B0685&dn=...",
  "torrent_url": "https://itorrents.net/torrent/2C6B6858D61DA9543D4231A71DB4B1C9264B0685.torrent",
  "hash": "2C6B6858D61DA9543D4231A71DB4B1C9264B0685",
  "resolved": true
}
```

抓取超时为 15 秒。拿不到磁力返回 404 + `MAGNET_UNAVAILABLE`，而不是 5xx：
「站点没给磁力」不是服务端故障，重试也不会改变结果。

### `GET /api/sources`

`{"sources": [TorrentSource, ...]}`，按 `src` 排序。

### `GET /api/health`

```json
{
  "status": "ok",
  "filter": {
    "enabled": true, "ready": true, "force_china": false,
    "lexicon_entries": 1929, "china_ranges": 4744,
    "client_ip": "192.168.18.169", "is_china": false
  }
}
```

`filter` 是违禁过滤的诊断块，没装过滤器时为 `null`。`is_china` 是这次请求会不会被过滤的结论，
`client_ip` 是服务端看到的来源；`ready` 表示词表和 IP 段都已就绪。
`client_ip` 是局域网地址时 `is_china` 必然为 `false`：同一个 WiFi 下服务端看不到你的公网出口。

### 静态页

`/` 302 到 `/index.html`；`/index.html`、`/styles.css`、`/app.js` 编译期嵌进二进制。
其余路径一律 404，不做 SPA 兜底。

## 错误

```json
{ "code": "MAGNET_UNAVAILABLE", "message": "…", "src": 7, "detail_url": "https://…" }
```

`src`、`detail_url` 只在与具体站点相关的错误里有值，其余为 `null`。

| `code` | HTTP | 含义 |
|---|---|---|
| `MISSING_KEYWORD` | 400 | 缺少 `q` |
| `INVALID_PAGE` | 400 | `page` 不是 1–100 的整数 |
| `INVALID_SRC` | 400 | `src` 未知，或在 `/api/magnet` 上缺失 / 传了 `all` |
| `INVALID_FILTER` | 400 | `filter` 不是 `1/0` 或 `true/false` |
| `INVALID_OPTIMIZE` | 400 | `optimize` 不是 `1/0` 或 `true/false` |
| `MISSING_URL` | 400 | 缺少 `detail_url` |
| `INVALID_URL` | 400 | `detail_url` 不是 http(s) |
| `INVALID_QUERY` | 400 | 查询串本身无法解析 |
| `MAGNET_UNAVAILABLE` | 404 | 参数正确，但站点没给出磁力 |
| `NOT_FOUND` | 404 | 没有这个路径 |
| `ALL_SOURCES_FAILED` | 502 | 搜索的全部来源都失败 |
| `INTERNAL_ERROR` | 500 | 未处理的服务端异常 |
