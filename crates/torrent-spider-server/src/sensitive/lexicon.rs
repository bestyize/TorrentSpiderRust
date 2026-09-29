//! 违禁词表：命中就丢掉整条结果。
//!
//! 词库来自 konsheng/Sensitive-lexicon（TrChat 格式）+ gogoswift/mingan 色情词库 + 补充的光杆词，
//! 见 `data/README.md`。**直接拿它做子串匹配会误伤普通内容**——实测 966 条真实结果朴素匹配误杀
//! 42 条（4.3%）：词表里本来就有「法院」「大师」「刺激」这类正常词，`youtube` 会命中
//! `YouTubeMovies`，`sm`/`anal` 会命中单词内部。所以匹配口径收窄成三条：
//!
//! 1. **汉字词**：至少 2 个汉字才收，做子串匹配（词库对变体字的覆盖靠的就是子串）。
//! 2. **非汉字词**：至少 4 个字符才收，且必须**整词命中**；`sm`、`flg`、`a4u` 这类超短串是误杀主因。
//! 3. 少于 2 个汉字的混合词（`9评`、`k粉`）不收，理由同上。
//!
//! 代价是有意留出的：拼音缩写、拆字、谐音会漏网。宁可漏掉几个变体，也不能把「法院」这种
//! 正常查询整个拦掉。

use std::collections::HashSet;
use std::sync::LazyLock;

use aho_corasick::AhoCorasick;
use serde::Deserialize;
use torrent_spider::TorrentInfo;
use torrent_spider::search::relevance::is_cjk;

/// 汉字词下限 2 个汉字，非汉字词下限 4 个字符。理由全是误杀，见模块注释。
const MIN_CJK: usize = 2;
const MIN_LATIN: usize = 4;

/// 内置词库原文。Kotlin 版存成 base64 只是为绕开 GraalVM native-image 漏资源；Rust 直接编进二进制。
const BUILTIN_JSON: &str = include_str!("../../data/lexicon.json");

/// 只解析一次：过滤是热路径。解析失败退回空表——合规兜底不该让服务起不来。
static BUILTIN: LazyLock<SensitiveLexicon> = LazyLock::new(|| {
    SensitiveLexicon::parse(BUILTIN_JSON).unwrap_or_else(|| {
        tracing::warn!("内置违禁词库解析失败，按空词表处理");
        SensitiveLexicon::empty()
    })
});

/// 词库的 JSON 形状：`{"lastUpdateDate": "...", "words": [...]}`。只取 `words`，缺失当空表。
#[derive(Deserialize)]
struct LexiconDto {
    #[serde(default)]
    words: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SensitiveLexicon {
    /// 汉字词（已小写、去重、保持原顺序），用于 `len` 与兜底匹配。
    cjk: Vec<String>,
    /// `cjk` 的多模式子串自动机；词表为空（或构建失败）时为 `None`。
    cjk_matcher: Option<AhoCorasick>,
    /// 非汉字词（已小写），与文本切出的字母段 / 数字段做整词比对。
    latin: HashSet<String>,
}

impl SensitiveLexicon {
    pub fn empty() -> Self {
        Self::default()
    }

    /// 按匹配口径收词并去重：汉字词 ≥2 个汉字，拉丁词 ≥4 字符。
    pub fn build<I, S>(words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut cjk = Vec::new();
        let mut seen = HashSet::new();
        let mut latin = HashSet::new();
        for raw in words {
            let word = raw.as_ref().trim().to_lowercase();
            if word.is_empty() {
                continue;
            }
            let cjk_count = word.chars().filter(|&c| is_cjk(c)).count();
            if cjk_count > 0 {
                if cjk_count >= MIN_CJK && seen.insert(word.clone()) {
                    cjk.push(word);
                }
            } else if word.chars().count() >= MIN_LATIN {
                latin.insert(word);
            }
        }
        // 构建只会因规模超限失败；真失败时退回逐词 contains，不丢词
        let cjk_matcher = if cjk.is_empty() { None } else { AhoCorasick::new(&cjk).ok() };
        Self { cjk, cjk_matcher, latin }
    }

    /// 解析 `{"lastUpdateDate": "...", "words": [...]}`；坏数据返回 `None`，由调用方决定留空还是沿用旧表。
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str::<LexiconDto>(json).ok().map(|dto| Self::build(dto.words))
    }

    /// 编进二进制的那份词库（`data/lexicon.json`）。
    pub fn builtin() -> &'static SensitiveLexicon {
        &BUILTIN
    }

    /// 单个文本是否命中（大小写不敏感）。
    pub fn matches(&self, text: &str) -> bool {
        if text.is_empty() || self.is_empty() {
            return false;
        }
        let lowered = text.to_lowercase();
        let cjk_hit = match &self.cjk_matcher {
            Some(matcher) => matcher.is_match(&lowered),
            None => self.cjk.iter().any(|word| lowered.contains(word.as_str())),
        };
        cjk_hit || (!self.latin.is_empty() && latin_tokens(&lowered).any(|token| self.latin.contains(token)))
    }

    /// 查询词本身命中时返回空列表；否则丢掉标题（标题为空时看详情页地址）命中的条目。
    ///
    /// 搜违禁词等于什么都不给：否则换个写法就能把内容捞出来，过滤等于没做。
    /// `keyword` 传原始查询词。空词表时原样返回——宁可不过滤也不能拦光。
    pub fn filter(&self, items: Vec<TorrentInfo>, keyword: &str) -> Vec<TorrentInfo> {
        if self.is_empty() || items.is_empty() {
            return items;
        }
        if self.matches(keyword) {
            return Vec::new();
        }
        items.into_iter().filter(|item| !self.item_matches(item)).collect()
    }

    /// 标题和详情页地址都查：部分站点的详情页 slug 带着完整标题，标题为空时它就是唯一线索。
    fn item_matches(&self, item: &TorrentInfo) -> bool {
        item.title.as_deref().is_some_and(|title| self.matches(title))
            || item.detail_url.as_deref().is_some_and(|url| self.matches(url))
    }

    /// 生效词条数（汉字词 + 拉丁词，去重后）。
    pub fn len(&self) -> usize {
        self.cjk.len() + self.latin.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cjk.is_empty() && self.latin.is_empty()
    }
}

/// 把文本切成字母段与数字段（`abc198964` → `abc`、`198964`），汉字与非字母数字都是分隔符。
///
/// 字母、数字分开切是为了让纯数字词在紧挨字母时也能命中。
fn latin_tokens(text: &str) -> impl Iterator<Item = &str> {
    // 0 = 分隔符，1 = 字母，2 = 数字
    let class = |c: char| match c {
        c if is_cjk(c) || !c.is_alphanumeric() => 0u8,
        c if c.is_numeric() => 2,
        _ => 1,
    };
    let mut rest = text;
    std::iter::from_fn(move || {
        let start = rest.find(|c: char| class(c) != 0)?;
        rest = &rest[start..];
        let kind = rest.chars().next().map(class)?;
        let end = rest.find(|c: char| class(c) != kind).unwrap_or(rest.len());
        let (token, tail) = rest.split_at(end);
        rest = tail;
        Some(token)
    })
}
