# 内置违禁词库

`lexicon.json` 由 `src/sensitive/lexicon.rs` 通过 `include_str!` 编进二进制，形状为 `{"lastUpdateDate": "...", "words": [...]}`，共 2029 条原始词，按匹配口径生效 1929 条（汉字词 1849 + 拉丁词 80）。

来源（原始文件留存在 `lexicon-src/`）：

- `konsheng-lexicon.json`：konsheng/Sensitive-lexicon 的 TrChat 兼容格式（2408 条）
- `gogoswift-porn.txt`：gogoswift/mingan 的色情词库（929 条，每行一条）
- 30 条补充「光杆词」（`色情`、`成人`、`萝莉` 等），见上游 `server/native/gen-lexicon.mjs` 的 `EXTRA_WORDS`
- 2026-09-30 手工补充 15 条（`破处`、`探花`、`啪啪` 等），直接追加在 `lexicon.json` 末尾；其中单字 `肏` 不满足匹配口径，不生效

合并规则与上游 `ref/TorrentSpider-upstream/server/native/gen-lexicon.mjs` 一致：按 konsheng → gogoswift → 补充词的顺序拼接，逐条 `trim`，丢空行，按小写去重（保留首次出现的原写法），`lastUpdateDate` 固定写 `2025/07/25`。

重新生成：用任意脚本按上述规则合并两份源文件与 `EXTRA_WORDS`，输出同形状 JSON 覆盖 `lexicon.json`（上游脚本输出的是 Kotlin 源码里的 base64，需再解码一次）。之后跑 `cargo test -p torrent-spider-server --test sensitive`，条数变了要确认是有意的再改测试里钉住的数字。
