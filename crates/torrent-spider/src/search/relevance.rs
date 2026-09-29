//! 按「与搜索词的相关性」给搜索结果重排、过滤，以及生成数字写法的检索变体。
//!
//! 各站点给的顺序多半按入库时间，聚合多站后更是直接拼接，跟关键词关系很弱。
//! 实测搜 `ubuntu`：美剧 `Celtics City S01E08 Chapter VIII Ubuntu 720p …` 会和
//! `ubuntu-22.04-desktop-amd64.iso` 并列，甚至排在前面。
//!
//! ## 打分口径
//!
//! 标题和查询都切成词，按下面的**字典序**逐项比较，前一项相等才看后一项：
//!
//! 1. `coverage` 查询词命中比例；
//! 2. `density` 命中词挨得紧不紧；
//! 3. `position` 首次命中位置，越靠前越高（专治「关键词埋在长标题尾部」）；
//! 4. `order` 命中顺序是否与查询一致；
//! 5. `date` 发布日期，新的在前。只在前几项打平时起作用：搜 `ubuntu` 时所有镜像
//!    都命中，此时「新的在前」才是用户要的；
//! 6. `noise` 没被命中的标题词数，越少越好；
//! 7. `length` 标题词数，越短越好；
//! 8. 小写标题字典序，只为结果可复现。
//!
//! 逐项比较而不是加权求和：加权要靠调参，每项各占多少说不清；逐项比则每项都能单独解释、单独测。
//!
//! ## 已知取舍
//!
//! - 只按非字母数字切词，不做词干还原、不拆驼峰，后者靠「包含」兜底；
//! - 汉字数字折成阿拉伯数字、全角折半角，见 `fold`；
//! - 中文整段算一个词，只能靠「包含」匹配；
//! - 不做同义词、不按热度加权。要加热度应追加在排序键末尾，而不是塞进现有几项。

use std::cmp::Ordering;
use std::sync::LazyLock;

use regex::Regex;

use crate::model::TorrentInfo;

/// 兜底匹配允许的最短词长（按字符算）。
///
/// `the`、`hd`、`4k` 在标题里遍地都是，拿它们兜底等于不过滤：搜 `the matrix 1999`
/// 若允许 `the` 兜底，`The.Boys`、`The.Suicide.Squad` 会全部留下。
/// 中文整段查询走整段匹配，不受这条限制。
const MIN_FALLBACK_LENGTH: usize = 4;

/// 全角 ASCII 段（`！`..`～`）与到半角的偏移。
const FULLWIDTH_FIRST: u32 = 0xFF01;
const FULLWIDTH_LAST: u32 = 0xFF5E;
const FULLWIDTH_OFFSET: u32 = 0xFEE0;

/// 返回按相关性排序的结果。稳定排序；关键词为空或一条都没命中时原样返回。
pub fn rank(items: Vec<TorrentInfo>, keyword: &str) -> Vec<TorrentInfo> {
    if items.len() < 2 {
        return items;
    }
    let query_tokens = tokenize(keyword);
    if query_tokens.is_empty() {
        return items;
    }

    let scores: Vec<Score> = items.iter().map(|item| score(item, &query_tokens)).collect();
    // 一条都没命中就不动顺序：没有依据可排，硬排只会让用户觉得是随机的；
    // 单源搜索时上游顺序正是站点自己的排序
    if scores.iter().all(|s| s.coverage <= 0.0) {
        return items;
    }

    let mut scored: Vec<(Score, TorrentInfo)> = scores.into_iter().zip(items).collect();
    // sort_by 是稳定排序，分数完全相同的保持原有相对顺序
    scored.sort_by(|(a, _), (b, _)| relevance(a, b));
    scored.into_iter().map(|(_, item)| item).collect()
}

/// 丢掉与关键词完全沾不上边的条目。关键词为空或纯符号时原样返回。
///
/// 站点会附送一堆无关的东西：TorrentKitty 每次都混进一行表头 `Torrent Description`；
/// 搜 `三体` 的 266 条里有 227 条是靠标题里恰好有个「三」字蹭上来的。
/// [`rank`] 只能把它们沉到末尾，翻页时照样会翻到，所以这里直接剔掉。
///
/// 判定：查询和标题都 `normalize`（折算后只留字母数字、小写，剔掉所有分隔符）后做包含判断。
/// 忽略分隔符让 `ubuntu server` 能命中 `ubuntu-server-22.04.iso`、
/// `复仇者联盟` 能命中 `[复 仇 者 联盟4] …`。
///
/// **整段查询必须出现**，不拆成「命中任一查询词」：拆开后 `三体` 变成「三」「体」，
/// 带「三」字的无关条目会全部留下。唯一放宽的是查询里最长且 ≥4 字符的那个词可兜底——
/// 用户常把限定词漏在标题外，`the matrix 1999` 应命中 `Matrix - 1999 - HMR`。
///
/// 宁严勿松：宁可错杀个别真结果（换个写法重搜即可），也不放过成片的无关内容。
pub fn filter_related(items: Vec<TorrentInfo>, keyword: &str) -> Vec<TorrentInfo> {
    let query = normalize(keyword);
    // 空查询原样返回而不是全部丢弃：那是数据丢失而不是过滤。纯标点归一化后也是空串
    if query.is_empty() {
        return items;
    }

    // 兜底词：最长的那个（并列取第一个），太短的不算；与整段查询相同时不必重复比
    let fallback = longest_first(tokenize(keyword))
        .filter(|token| token.chars().count() >= MIN_FALLBACK_LENGTH)
        .map(|token| normalize(&token))
        .filter(|token| !token.is_empty() && *token != query);

    items
        .into_iter()
        .filter(|item| {
            let title = normalize(item.title.as_deref().unwrap_or_default());
            title.contains(&query) || fallback.as_deref().is_some_and(|f| title.contains(f))
        })
        .collect()
}

/// 同一查询意图的其它数字写法，不含 `keyword` 本身；可能为空。
///
/// 站点多半按字面搜：搜 `复仇者联盟4` 只返回含 `4` 的，写成 `复仇者联盟四` 的一条不给
/// （实测 BtSow：`4` 出 50 条、`四` 出 3 条，几乎不相交）。本地折算救不回来，
/// 得把两种写法都发给上游再合并。
///
/// 只在「有汉字 + 数字紧挨汉字 + 数值 1..99」时折：
/// - `第十二季` ↔ `第12季` 这类序号两种写法严格等价；
/// - 纯拉丁查询一概不折：`windows 11` 折成 `windows 十一` 会捞进 `Windows 95` 一堆垃圾；
/// - 四位年份不折：`复仇者联盟二千零一十九` 毫无意义。
pub fn search_variants(keyword: &str) -> Vec<String> {
    // 没有汉字就不折，也把纯英文查询挡在最前面，连扫描都省了
    if !keyword.chars().any(is_cjk) {
        return Vec::new();
    }
    let chars: Vec<char> = keyword.chars().collect();
    let runs = adjacent_numeral_runs(&chars);
    if runs.is_empty() {
        return Vec::new();
    }
    let digit_form = replace_numeral_runs(&chars, &runs, to_digit_form);
    let chinese_form = replace_numeral_runs(&chars, &runs, to_chinese_form);
    [digit_form, chinese_form].into_iter().filter(|form| !form.trim().is_empty() && form != keyword).collect()
}

/// 是不是汉字（中日韩统一表意文字，含扩展 A 区）。违禁词匹配与这里共用同一组区间。
pub fn is_cjk(c: char) -> bool {
    matches!(u32::from(c), 0x4E00..=0x9FFF | 0x3400..=0x4DBF)
}

/// 最长的那个；并列时取第一个（与 Kotlin `maxByOrNull` 一致，`max_by_key` 会取最后一个）。
fn longest_first(tokens: Vec<String>) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for token in tokens {
        let len = token.chars().count();
        if best.as_ref().is_none_or(|(best_len, _)| len > *best_len) {
            best = Some((len, token));
        }
    }
    best.map(|(_, token)| token)
}

// ---------- 折算：全角 → 半角、汉字数字 → 阿拉伯数字 ----------

/// 数位：`三` -> 3。`两` 也按 2 算——`两个` 和 `2个` 是一回事。
fn chinese_digit(c: char) -> Option<i64> {
    Some(match c {
        '零' | '〇' => 0,
        '一' => 1,
        '二' | '两' => 2,
        '三' => 3,
        '四' => 4,
        '五' => 5,
        '六' => 6,
        '七' => 7,
        '八' => 8,
        '九' => 9,
        _ => return None,
    })
}

/// 权位：`十` -> 10。
fn chinese_unit(c: char) -> Option<i64> {
    Some(match c {
        '十' => 10,
        '百' => 100,
        '千' => 1000,
        _ => return None,
    })
}

/// 节权位：`万` -> 10000。
fn chinese_section(c: char) -> Option<i64> {
    Some(match c {
        '万' => 10_000,
        '亿' => 100_000_000,
        _ => return None,
    })
}

fn is_chinese_numeral(c: char) -> bool {
    chinese_digit(c).is_some() || chinese_unit(c).is_some() || chinese_section(c).is_some()
}

/// 折成「比较用」的写法：全角转半角 + 汉字数字折成阿拉伯数字，其余原样透传。
///
/// 它是 [`normalize`] 与 [`tokenize`] 的共同前置步骤，这样过滤和排序对同一段标题的
/// 理解不会分叉。`第十二季` 与 `第12季`、`ｕｂｕｎｔｕ` 与 `ubuntu` 语义上没有区别；
/// 全角标点也一并折成半角，让 [`tokenize`] 把它当普通分隔符。
fn fold(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if is_chinese_numeral(c) {
            let end = index + chars[index..].iter().take_while(|&&c| is_chinese_numeral(c)).count();
            out.push_str(&chinese_to_digits(&chars[index..end]));
            index = end;
            continue;
        }
        let code = u32::from(c);
        let folded = if (FULLWIDTH_FIRST..=FULLWIDTH_LAST).contains(&code) {
            char::from_u32(code - FULLWIDTH_OFFSET).unwrap_or(c)
        } else {
            c
        };
        out.push(folded);
        index += 1;
    }
    out
}

/// 把一串汉字数字折成阿拉伯数字。
///
/// 不能逐字替换：`十二` 是 12 不是 102，`二零二四` 是 2024 不是 4。
/// - 整串没有权位字（十/百/千/万/亿）⇒ 逐位读：`二零二四` -> `2024`、`零五` -> `05`；
/// - 有 ⇒ 按位值累加：`十二` -> `12`、`一百零五` -> `105`、`一万二千` -> `12000`。
///
/// 只认常用数字字，不认 `壹贰叁`、`廿卅`：漏折只是维持不匹配，不会误匹配。
fn chinese_to_digits(run: &[char]) -> String {
    // 单个 `万`/`亿` 按位值算会得 0，那是个错的数字（还会去匹配标题里的 `0`），原样返回。
    // `十` 不在其列——`十` 本身就是「十」
    if let [only] = run
        && chinese_section(*only).is_some()
    {
        return only.to_string();
    }

    let has_unit = run.iter().any(|&c| chinese_unit(c).is_some() || chinese_section(c).is_some());
    if !has_unit {
        let mut digits = String::with_capacity(run.len());
        for &c in run {
            match chinese_digit(c) {
                Some(d) => digits.push_str(&d.to_string()),
                None => return run.iter().collect(),
            }
        }
        return digits;
    }

    // 标题是不可信输入，`九亿亿亿…` 这种会溢出；用 wrapping 与 Kotlin Long 的溢出语义一致且不 panic
    let (mut total, mut section, mut number) = (0i64, 0i64, 0i64);
    for &c in run {
        if let Some(d) = chinese_digit(c) {
            number = d;
        } else if let Some(unit) = chinese_unit(c) {
            // `十二` 的 `十` 前面没有数位，按 1 算；`二十` 前面是 2，那就是 20
            let base = if number == 0 { 1 } else { number };
            section = section.wrapping_add(base.wrapping_mul(unit));
            number = 0;
        } else if let Some(scale) = chinese_section(c) {
            section = section.wrapping_add(number).wrapping_mul(scale);
            total = total.wrapping_add(section);
            section = 0;
            number = 0;
        }
    }
    total.wrapping_add(section).wrapping_add(number).to_string()
}

/// 归一化：先 [`fold`]，再只留字母数字并小写，用来做忽略分隔符的包含判断。
///
/// 折在前、剔在后：反过来的话 `１２` 这类全角数字会在折之前就被剔掉。
fn normalize(text: &str) -> String {
    fold(text).chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// 切成小写词。
///
/// 先 [`fold`]，与 [`filter_related`] 的判定口径一致。手写扫描是为了守住版本号里的点：
/// `22.04` 若被切成 `22` 和 `04`，查 `22.04` 就再也对不上了。
fn tokenize(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let folded: Vec<char> = fold(text).chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    for (index, &c) in folded.iter().enumerate() {
        if c.is_alphanumeric() {
            current.extend(c.to_lowercase());
            continue;
        }
        let digit_dot_digit = c == '.'
            && current.chars().last().is_some_and(char::is_numeric)
            && folded.get(index + 1).is_some_and(|next| next.is_numeric());
        if digit_dot_digit {
            current.push(c);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

// ---------- search_variants 的数字串处理 ----------

/// 阿拉伯数字的数值。Kotlin `isDigit()` 认全部 Unicode Nd 数字；这里只认 ASCII 与
/// 全角（输入法里真会出现的两种），其它文字的数字在查询里罕见，不认只是少一个变体。
fn decimal_digit(c: char) -> Option<u32> {
    match c {
        '0'..='9' => Some(u32::from(c) - u32::from('0')),
        '０'..='９' => Some(u32::from(c) - u32::from('０')),
        _ => None,
    }
}

fn is_decimal_digit(c: char) -> bool {
    decimal_digit(c).is_some()
}

fn is_numeral_char(c: char) -> bool {
    is_decimal_digit(c) || is_chinese_numeral(c)
}

/// 找出「紧挨汉字」、数值在 1..99 的数字串在 `chars` 里的区间。
///
/// 一段数字串要么整段阿拉伯数字、要么整段汉字数字；混写（如 `4四`）会被切成两段各自判断。
fn adjacent_numeral_runs(chars: &[char]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if !is_numeral_char(chars[index]) {
            index += 1;
            continue;
        }
        let arabic = is_decimal_digit(chars[index]);
        let end =
            index + chars[index..].iter().take_while(|&&c| is_numeral_char(c) && is_decimal_digit(c) == arabic).count();
        let touches_cjk = (index > 0 && is_cjk(chars[index - 1])) || chars.get(end).is_some_and(|&c| is_cjk(c));
        if touches_cjk && numeral_value(&chars[index..end]).is_some() {
            runs.push((index, end));
        }
        index = end;
    }
    runs
}

/// 纯阿拉伯数字串的数值；溢出时为 `None`（同 Kotlin `toIntOrNull`）。前导零照认：`007` -> 7。
fn arabic_value(run: &[char]) -> Option<i32> {
    run.iter().try_fold(0i32, |acc, &c| acc.checked_mul(10)?.checked_add(decimal_digit(c)? as i32))
}

/// 把数字串折算成 1..99 的数值；折不出或超出范围时返回 `None`。
fn numeral_value(run: &[char]) -> Option<i32> {
    let value = if run.iter().all(|&c| is_decimal_digit(c)) {
        arabic_value(run)
    } else {
        chinese_to_digits(run).parse::<i32>().ok()
    };
    value.filter(|v| (1..=99).contains(v))
}

fn replace_numeral_runs(chars: &[char], runs: &[(usize, usize)], convert: fn(&[char]) -> String) -> String {
    let mut out = String::with_capacity(chars.len() * 3);
    let mut cursor = 0;
    for &(start, end) in runs {
        out.extend(&chars[cursor..start]);
        out.push_str(&convert(&chars[start..end]));
        cursor = end;
    }
    out.extend(&chars[cursor..]);
    out
}

fn to_digit_form(run: &[char]) -> String {
    if run.iter().all(|&c| is_decimal_digit(c)) { run.iter().collect() } else { chinese_to_digits(run) }
}

fn to_chinese_form(run: &[char]) -> String {
    // 进到这里的数字串都已过 numeral_value，取值一定成功；兜底原样返回只为不 panic
    match run.iter().all(|&c| is_decimal_digit(c)).then(|| arabic_value(run)).flatten() {
        Some(value) => number_to_chinese(value),
        None => run.iter().collect(),
    }
}

/// 把 1..99 折成汉字写法。只服务 [`search_variants`]，范围由 [`numeral_value`] 保证。
fn number_to_chinese(value: i32) -> String {
    const DIGITS: [char; 10] = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
    let digit = |n: i32| DIGITS[n.rem_euclid(10) as usize];
    if value < 10 {
        return digit(value).to_string();
    }
    let mut out = String::new();
    let tens = value / 10;
    // `十二` 而不是 `一十二`：十位上的一不写出来
    if tens > 1 {
        out.push(digit(tens));
    }
    out.push('十');
    let ones = value % 10;
    if ones > 0 {
        out.push(digit(ones));
    }
    out
}

// ---------- 打分 ----------

struct Score {
    coverage: f64,
    density: f64,
    position: f64,
    order: f64,
    date: i64,
    noise: usize,
    length: usize,
    /// 第 8 项的比较键，预先算好免得排序时反复小写化。
    lower_title: String,
}

/// 逐项比较，顺序即模块文档里的 1..8。
fn relevance(a: &Score, b: &Score) -> Ordering {
    b.coverage
        .total_cmp(&a.coverage)
        .then_with(|| b.density.total_cmp(&a.density))
        .then_with(|| b.position.total_cmp(&a.position))
        .then_with(|| b.order.total_cmp(&a.order))
        .then_with(|| b.date.cmp(&a.date))
        .then_with(|| a.noise.cmp(&b.noise))
        .then_with(|| a.length.cmp(&b.length))
        .then_with(|| a.lower_title.cmp(&b.lower_title))
}

fn score(item: &TorrentInfo, query_tokens: &[String]) -> Score {
    let title = item.title.as_deref().unwrap_or_default();
    let date = epoch_day(item.date.as_deref());
    let lower_title = title.to_lowercase();
    let empty = |noise, length| Score {
        coverage: 0.0,
        density: 0.0,
        position: 0.0,
        order: 0.0,
        date,
        noise,
        length,
        lower_title: lower_title.clone(),
    };

    let title_tokens = tokenize(title);
    if title_tokens.is_empty() {
        return empty(0, 0);
    }

    // 按查询词各自的顺序记下命中的标题下标，没命中的不记
    let matched: Vec<usize> = query_tokens.iter().filter_map(|token| first_match(token, &title_tokens)).collect();
    let (Some(&earliest), Some(&latest)) = (matched.iter().min(), matched.iter().max()) else {
        return empty(title_tokens.len(), title_tokens.len());
    };

    // matched 按查询顺序排，区间必须另取 min/max：查询 `a b` 命中到 [5, 2] 时首尾相减是负数
    let span = latest - earliest + 1;
    let mut distinct = matched.clone();
    distinct.sort_unstable();
    distinct.dedup();

    Score {
        coverage: matched.len() as f64 / query_tokens.len() as f64,
        // 同一个标题词命中多个查询词时 span 会小于命中数（`ubuntu-server` 被 `ubuntu server`
        // 同时命中），夹一下免得密度大于 1
        density: matched.len().min(span) as f64 / span as f64,
        position: if title_tokens.len() <= 1 { 1.0 } else { 1.0 - earliest as f64 / (title_tokens.len() - 1) as f64 },
        order: order_score(&matched),
        date,
        noise: title_tokens.len() - distinct.len(),
        length: title_tokens.len(),
        lower_title,
    }
}

/// `query_token` 在标题词里第一次出现的位置。
///
/// 先找完全相等的，位置才反映真实词序；找不到再退一步用「包含」，兜住 `UbuntuServer`
/// 这种没切开的驼峰，以及中文标题整段是一个词的情况。
fn first_match(query_token: &str, title_tokens: &[String]) -> Option<usize> {
    if let Some(exact) = title_tokens.iter().position(|t| t == query_token) {
        return Some(exact);
    }
    // 单字符不做包含：否则 `1` 会命中 `1080p`、`22.04.1` 一大片
    if query_token.chars().count() < 2 {
        return None;
    }
    title_tokens.iter().position(|t| t.contains(query_token))
}

/// 命中词在标题里的先后是否与查询一致。少于两个词时这项不区分，记满分。
fn order_score(indices_in_query_order: &[usize]) -> f64 {
    if indices_in_query_order.len() < 2 {
        return 1.0;
    }
    let ascending = indices_in_query_order.windows(2).filter(|pair| pair[1] > pair[0]).count();
    ascending as f64 / (indices_in_query_order.len() - 1) as f64
}

// ---------- 日期 ----------
//
// 正则里用 `[0-9]` 与 ASCII `\s`：Kotlin（JVM 正则）的 `\d`/`\s` 默认只认 ASCII，
// 而 `regex` crate 默认按 Unicode 解释。

/// ① `2025-04-02` / `2025/04/02`（btsow、torrentkitty）。
static YEAR_FIRST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9]{4})[-/]([0-9]{1,2})[-/]([0-9]{1,2})$").unwrap());

/// ② `05-18 2022` / `23.12.2023` / `05-18-2022`（piratebay 等）。
static MONTH_DAY_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9]{1,2})[-.]([0-9]{1,2})[\t\n\x0B\f\r -]([0-9]{4})$").unwrap());

/// ③ `Dec. 23rd 23`（x1337；年份可能是两位）。
static MONTH_NAME_FIRST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([A-Za-z]{3,})\.?[\t\n\x0B\f\r ]+([0-9]{1,2})(?:st|nd|rd|th)?[\t\n\x0B\f\r ]+([0-9]{2,4})$").unwrap()
});

/// ④ 按空白与逗号切字段。
static FIELD_SEPARATOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\t\n\x0B\f\r ,]+").unwrap());

/// ⑤ `9/20/2020` / `25/12/2020`（solid、torlock）。
static SLASHED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([0-9]{1,2})/([0-9]{1,2})/([0-9]{4})$").unwrap());

/// 英文月份前三位 -> 月份。
fn month_of(name: &str) -> Option<u32> {
    let prefix: String = name.to_lowercase().chars().take(3).collect();
    let months = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    months.iter().position(|m| *m == prefix).map(|i| i as u32 + 1)
}

/// 把各站点五花八门的日期解析成可比较的 epoch day，解析不出来返回 0。
///
/// 不能直接拿字符串比：同一批结果里并存 `2025-04-02`、`9/20/2020`、`05-18 2022`、
/// `Dec. 23rd 23` 四种写法，按字符串排会把 `9/20/2020` 判成比 `2025-04-02` 还大。
/// 解析不出来记 0（最旧），即不参与「新的在前」的竞争——拿不准的条目不该被猜出来的值顶到前面。
fn epoch_day(raw: Option<&str>) -> i64 {
    let text = raw.unwrap_or_default().trim();
    if text.is_empty() {
        return 0;
    }
    // 各分组都是限长 ASCII 数字，解析不会失败；失败按 0 走合法性校验
    let num = |caps: &regex::Captures<'_>, i: usize| caps[i].parse::<i32>().unwrap_or(0);

    if let Some(caps) = YEAR_FIRST.captures(text) {
        return civil_to_epoch_day(num(&caps, 1), num(&caps, 2), num(&caps, 3));
    }
    if let Some(caps) = MONTH_DAY_YEAR.captures(text) {
        return month_day_or_day_month(num(&caps, 1), num(&caps, 2), num(&caps, 3));
    }
    if let Some(caps) = MONTH_NAME_FIRST.captures(text) {
        let Some(month) = month_of(&caps[1]) else { return 0 };
        return civil_to_epoch_day(expand_year(num(&caps, 3)), month as i32, num(&caps, 2));
    }

    // ④ `23 Dec 2023` / `Dec 23 2023`：月名和数字的相对位置不定，逐个字段认
    let (mut month, mut day, mut year) = (0, 0, 0);
    for part in FIELD_SEPARATOR.split(text).filter(|p| !p.is_empty()) {
        match month_of(part) {
            Some(m) if month == 0 => month = m as i32,
            _ => {
                if let Ok(number) = part.parse::<i32>() {
                    let starts_with_digit = part.starts_with(|c: char| c.is_ascii_digit());
                    if day == 0 && starts_with_digit && (1..=31).contains(&number) && part.len() <= 2 {
                        day = number;
                    } else if year == 0 && (1000..=9999).contains(&number) {
                        year = number;
                    }
                }
            }
        }
    }
    if month > 0 && day > 0 && year > 0 {
        return civil_to_epoch_day(year, month, day);
    }

    if let Some(caps) = SLASHED.captures(text) {
        return month_day_or_day_month(num(&caps, 1), num(&caps, 2), num(&caps, 3));
    }
    0
}

/// 两个数字里哪个月哪一天。
///
/// 只有「第一个数大于 12 而第二个不大于」时才确定是日-月，其余一律按月-日（这些站点普遍是美式写法）。
/// 判不出来时不猜，交给 [`civil_to_epoch_day`] 的合法性校验去挡。
fn month_day_or_day_month(first: i32, second: i32, year: i32) -> i64 {
    if first > 12 && second <= 12 {
        civil_to_epoch_day(year, second, first)
    } else {
        civil_to_epoch_day(year, first, second)
    }
}

/// `23` -> `2023`。两位数年份按 2000 年代补全。
fn expand_year(year: i32) -> i32 {
    if year < 100 { 2000 + year } else { year }
}

/// 只有全部字段合法才认，避免把 `2020-13-45` 也算成一天。
fn civil_to_epoch_day(year: i32, month: i32, day: i32) -> i64 {
    if !(1900..=2200).contains(&year) || !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return 0;
    }
    let y = i64::from(if month <= 2 { year - 1 } else { year });
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let month_shift = i64::from(month + if month > 2 { -3 } else { 9 });
    let day_of_year = (153 * month_shift + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap_year(year) => 29,
        _ => 28,
    }
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}
