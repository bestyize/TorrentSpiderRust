//! 相关性排序 / 过滤 / 检索变体的行为契约。
//!
//! 用例里的标题都是真实站点返回的原文（抓自 btsow / torlock），不是编出来的。
//! 排序规则最怕「凭想象写权重」——拿真实数据当断言，规则一偏就会红。

use torrent_spider::TorrentInfo;
use torrent_spider::search::relevance::{filter_related, rank, search_variants};

fn title_src(text: &str, src: u32) -> TorrentInfo {
    TorrentInfo { title: Some(text.to_owned()), src, ..Default::default() }
}

fn title(text: &str) -> TorrentInfo {
    title_src(text, 6)
}

fn dated(text: &str, date: &str) -> TorrentInfo {
    TorrentInfo { title: Some(text.to_owned()), date: Some(date.to_owned()), src: 6, ..Default::default() }
}

fn titles(items: &[TorrentInfo]) -> Vec<String> {
    items.iter().map(|it| it.title.clone().unwrap_or_default()).collect()
}

fn t(item: &TorrentInfo) -> String {
    item.title.clone().unwrap_or_default()
}

fn ranked_titles(items: &[TorrentInfo], keyword: &str) -> Vec<String> {
    titles(&rank(items.to_vec(), keyword))
}

fn kept_titles(items: &[TorrentInfo], keyword: &str) -> Vec<String> {
    titles(&filter_related(items.to_vec(), keyword))
}

#[test]
fn ubuntu_images_rank_above_tv_episode() {
    // 实测 btsow 搜 ubuntu 时，站点自己的顺序把这条美剧放得很靠前：
    // 关键词只是标题尾巴上的一个词，站点并不在意这种事。
    let episode = title("Celtics City S01E08 Chapter VIII Ubuntu 720p AMZN WEB-DL DDP5 1 H 264-RAWR[EZTVx.to].mkv");
    let image22 = title("ubuntu-22.04.5-desktop-amd64.iso");
    let image24 = title("ubuntu-24.04.2-desktop-amd64.iso");

    let ranked = ranked_titles(&[episode, image22, image24], "ubuntu");

    assert!(ranked.last().unwrap().contains("Celtics"), "关键词埋在长标题尾部的条目应该垫底，实际顺序：{ranked:?}");
    assert!(ranked.first().unwrap().starts_with("ubuntu-"), "系统镜像应该排在最前，实际顺序：{ranked:?}");
}

#[test]
fn more_noise_words_rank_lower() {
    // 两条都命中了全部查询词，差别只在 mate / iso / 分隔符这些多余词上。
    // 实测这两种命名在 btsow 上并存，用户要的是不带 mate 的那条在前。
    let plain = title("ubuntu-22.04-desktop-amd64.iso");
    let mate = title("ubuntu-mate-22.04-desktop-amd64.iso");

    let ranked = ranked_titles(&[mate.clone(), plain.clone()], "ubuntu 22.04 desktop amd64");

    assert_eq!(ranked, vec![t(&plain), t(&mate)], "带 mate 的应该排在后面");
}

#[test]
fn query_word_order_counts() {
    // 两个词都命中、都靠前、都紧凑，唯一的区别是出现顺序和查询是否一致
    let reversed = title("server ubuntu 22.04");
    let in_order = title("ubuntu server 22.04");

    let ranked = ranked_titles(&[reversed.clone(), in_order.clone()], "ubuntu server");

    assert_eq!(ranked, vec![t(&in_order), t(&reversed)], "顺序与查询一致的应该在前");
}

#[test]
fn dot_in_version_is_not_a_token_boundary() {
    // 把 22.04 切成 22 和 04，搜 22.04 就再也对不上了——而镜像名里到处是这种版本号
    let dotted = title("ubuntu-22.04-desktop-amd64.iso");
    let spaced = title("ubuntu 22 04 weird name");

    let ranked = ranked_titles(&[spaced, dotted.clone()], "22.04");

    assert_eq!(ranked[0], t(&dotted), "22.04 必须整段参与匹配");
}

#[test]
fn earlier_keyword_position_wins() {
    let head = title("ubuntu-server-22.04.iso");
    let tail = title("the-ubuntu-story-2024.mkv");

    let ranked = ranked_titles(&[tail, head.clone()], "ubuntu");

    assert_eq!(ranked[0], t(&head), "关键词在标题开头的应该在前");
}

#[test]
fn chinese_title_ranks_by_containment() {
    // 中文整段是一个词，切不出更细的单元，只能靠包含判断
    let matched = title("复仇者联盟4：终局之战");
    let unrelated = title("某部无关的电影 2024");

    let ranked = ranked_titles(&[unrelated, matched.clone()], "复仇者联盟");

    assert_eq!(ranked[0], t(&matched), "中文查询也要能命中并前移");
}

// Kotlin 用 assertSame 断言「原样返回同一个 List」。Rust 按值传入、按值返回，
// 这里退一步断言内容与顺序完全不变。

#[test]
fn blank_keyword_returns_input_unchanged() {
    let items = vec![title("banana"), title("Apple")];

    assert_eq!(rank(items.clone(), ""), items, "没有查询词就不该重排");
    assert_eq!(rank(items.clone(), "   "), items, "纯空白等同没查询");
}

#[test]
fn no_hit_keeps_upstream_order() {
    // 之所以不退回标题字典序：单源搜索时上游顺序正是站点自己的排序，
    // 比我们凭空编一个顺序更有信息量
    let items = vec![title("banana"), title("Apple"), title("cherry")];

    assert_eq!(rank(items.clone(), "zzz"), items, "无命中时不该动顺序");
}

#[test]
fn fewer_than_two_items_returned_unchanged() {
    let single = vec![title("ubuntu-22.04.iso")];

    assert_eq!(rank(single.clone(), "ubuntu"), single);
    assert_eq!(rank(Vec::new(), "ubuntu"), Vec::<TorrentInfo>::new());
}

#[test]
fn rank_does_not_mutate_input() {
    // Kotlin 断言入参列表不被修改；Rust 里 rank 拿走所有权，这条由类型系统保证。
    // 能测的只剩「同一输入重复排序结果一致」，没有隐藏状态。
    let items = vec![title("banana"), title("Apple")];

    let first = ranked_titles(&items, "apple");
    let second = ranked_titles(&items, "apple");

    assert_eq!(first, second);
    assert_eq!(first, vec!["Apple", "banana"]);
}

#[test]
fn repeated_ranking_is_deterministic() {
    // 分数全相同的条目不能靠哈希顺序决定名次，否则同一个查询两次结果会漂
    let items = vec![title("ubuntu-22.04-desktop-amd64.iso"), title("ubuntu-24.04-desktop-amd64.iso")];

    let first = ranked_titles(&items, "ubuntu");
    let second = ranked_titles(&items, "ubuntu");

    assert_eq!(first, second);
    assert_eq!(first, vec!["ubuntu-22.04-desktop-amd64.iso", "ubuntu-24.04-desktop-amd64.iso"]);
}

#[test]
fn blank_title_does_not_crash_and_ranks_after_hits() {
    let blank = title("");
    let matched = title("ubuntu-22.04.iso");

    let ranked = ranked_titles(&[blank.clone(), matched.clone()], "ubuntu");

    assert_eq!(ranked, vec![t(&matched), t(&blank)]);
}

// ---------- 打平时的日期兜底 ----------
//
// 搜单个词时（ubuntu）所有标题都命中，相关性完全打平，
// 此时的顺序由日期决定。下面几条用的都是实测抓到的真实日期写法。

#[test]
fn newer_first_when_relevance_ties() {
    let newer = dated("ubuntu-24.04-desktop-amd64.iso", "2026-03-25");
    let older = dated("ubuntu-14.10-desktop-amd64.iso", "2014-10-23");

    let ranked = ranked_titles(&[older.clone(), newer.clone()], "ubuntu");

    assert_eq!(ranked, vec![t(&newer), t(&older)], "打平时新的应该在前");
}

#[test]
fn four_real_date_formats_are_comparable() {
    // 实测并存于同一批结果的四种写法，必须都能解析成同一个坐标系。
    // 直接拿字符串比会把 "9/20/2020" 判成比 "2025-04-02" 更大。
    let slashed = dated("ubuntu 9/20/2020 版", "9/20/2020"); // solid / torlock
    let dashed = dated("ubuntu 05-18 2022 版", "05-18 2022"); // piratebay
    let iso = dated("ubuntu 2025-04-02 版", "2025-04-02"); // btsow / kitty
    let month_name = dated("ubuntu Dec. 23rd 20 版", "Dec. 23rd 20"); // x1337

    let ranked = ranked_titles(&[slashed.clone(), dashed.clone(), month_name.clone(), iso.clone()], "ubuntu");

    // 2025-04-02 > 2022-05-18 > 2020-12-23 > 2020-09-20
    assert_eq!(ranked, vec![t(&iso), t(&dashed), t(&month_name), t(&slashed)]);
}

#[test]
fn unparseable_date_is_not_treated_as_newest() {
    // 拿不准的条目不该因为一个猜出来的值被顶到最前面
    let unparseable = dated("ubuntu-24.04-desktop-amd64.iso", "刚刚");
    let dated2020 = dated("ubuntu-20.04-desktop-amd64.iso", "2020-01-01");

    let ranked = ranked_titles(&[unparseable.clone(), dated2020.clone()], "ubuntu");

    assert_eq!(ranked, vec![t(&dated2020), t(&unparseable)], "解析不出日期按最旧处理");
}

#[test]
fn invalid_date_is_not_counted_as_a_day() {
    let nonsense = dated("ubuntu-24.04-desktop-amd64.iso", "2020-13-45");
    let real = dated("ubuntu-16.04-desktop-amd64.iso", "2016-04-21");

    let ranked = ranked_titles(&[nonsense.clone(), real.clone()], "ubuntu");

    assert_eq!(ranked, vec![t(&real), t(&nonsense)]);
}

#[test]
fn date_never_overrides_relevance() {
    // 这条美剧日期很新，但它只是关键词命中得差——相关性必须优先于日期
    let episode = dated("Celtics City S01E08 Chapter VIII Ubuntu 720p WEB-DL.mkv", "2026-08-01");
    let old_image = dated("ubuntu-14.10-desktop-amd64.iso", "2014-10-23");

    let ranked = ranked_titles(&[episode, old_image.clone()], "ubuntu");

    assert_eq!(ranked[0], t(&old_image), "日期再新也不能盖过相关性");
}

// ---------- 剔除完全不相关的条目 ----------
//
// 下面几条用的都是实测抓到的原文，涵盖三类必须分清的输入：
// 真正无关的（该丢）、只命中一部分查询词的（该留）、
// 单字查询下站点按子串返回的（该留——那是用户本来的意图）。

#[test]
fn drops_items_without_any_query_hit() {
    // 实测：TorrentKitty 每次搜索都会混进一行表头 "Torrent Description"。
    // 搜 xyzzyplugh 这种不存在的词时，七个站点只剩两条结果，其中一条就是它。
    let header = title_src("Torrent Description", 8);
    let real = title("ubuntu-24.04-desktop-amd64.iso");

    assert_eq!(kept_titles(&[header, real.clone()], "ubuntu"), vec![t(&real)], "表头行该被丢掉");
}

#[test]
fn dash_or_dot_separated_title_still_matches() {
    // 归一化会把分隔符全部剔掉，所以 `ubuntu server` 能命中这几种写法。
    // 按原文比的话它们一个都匹配不上，而那些镜像名几乎全是连字符写法。
    let dashed = title("ubuntu-server-22.04.iso");
    let dotted = title("Ubuntu.Server.22.04.LTS");
    let unrelated = title("Debian 12 netinst iso");

    assert_eq!(
        kept_titles(&[dashed.clone(), dotted.clone(), unrelated], "ubuntu server"),
        vec![t(&dashed), t(&dotted)]
    );
}

#[test]
fn longest_query_word_rescues_title_missing_qualifier() {
    // 实测原文：这条少了查询开头的 `the`，整段比会误杀，但它的 `matrix` 一定在。
    // 兜底词取查询里最长的那个，且需 ≥4 字符。
    let short_title = title("Matrix - 1999 - HMR - MR4K - 10bit - SDR.mkv");
    let unrelated = title("The.Boys.S04E04.1080p.WEB.h264-ETHEL");

    assert_eq!(kept_titles(&[short_title.clone(), unrelated], "the matrix 1999"), vec![t(&short_title)]);
}

#[test]
fn too_short_words_do_not_act_as_fallback() {
    // 搜 the matrix 1999 时若允许 `the` 兜底，The.Boys、The.Suicide.Squad
    // 会全部留下——而那正是这条查询要滤掉的批次。
    // 这是实测抓到的原文：the matrix 1999 的结果里混着这些
    let boys = title("The.Boys.S04E04.1080p.WEB.h264-ETHEL[TGx]");
    let squad = title("The.Suicide.Squad.2021.1080p.WEBRip.x264");
    let matrix = title("Matrix - 1999 - HMR - MR4K - 10bit - SDR.mkv");

    assert_eq!(
        kept_titles(&[boys, squad, matrix.clone()], "the matrix 1999"),
        vec![t(&matrix)],
        "靠 the 蒙混过关的都该丢掉"
    );
}

#[test]
fn only_titles_really_containing_san_ti_are_kept() {
    // 实测搜「三体」拿到 266 条，其中 227 条是靠标题里恰好有个「三」字蹭上来的。
    // 判定要求整段查询词出现，所以只沾一个「三」字的那些会被丢掉——
    // 包括这里的三国演义，它带了「三」却不是《三体》，丢掉是对的。
    let real = title("三体 电视剧 2023 4K HDR");
    let has_only_san = title("三国演义 1994 全84集");
    let unrelated = title("HEYZO 3964 アナル初体験");

    assert_eq!(kept_titles(&[real.clone(), has_only_san, unrelated], "三体"), vec![t(&real)]);
}

#[test]
fn single_char_query_still_matches_whole() {
    // 词长下限（≥4 字符）只管兜底那一路，不挡整段匹配，
    // 所以用户真输一个字时照样能过滤：标题里有这个字就留。
    // 注意 `三` 现在会先折成 `3`，所以这条无关条目的编号里刻意不含 3。
    let has_char = title("三国演义 1994 全84集");
    let unrelated = title("HEYZO 4964 アナル初体験");

    assert_eq!(kept_titles(&[has_char.clone(), unrelated], "三"), vec![t(&has_char)], "整段匹配不受词长下限限制");
}

// ---------- 汉字数字 ≡ 阿拉伯数字、全角 ≡ 半角 ----------
//
// 站点对同一部剧一会儿写「第二季」一会儿写「第2季」，年份也时写「二零二四」
// 时写「2024」。不折的话这两半永远对不上，等于漏掉一半结果。

#[test]
fn chinese_and_arabic_numerals_are_the_same_number() {
    // 查询写汉字、标题写阿拉伯数字（以及反过来）都要能互相命中
    let arabic_season = title("某某剧 第12季 1080p WEB-DL");
    let chinese_season = title("某某剧 第十二季 1080p WEB-DL");
    let other_season = title("某某剧 第八季 1080p WEB-DL");

    assert_eq!(
        kept_titles(&[arabic_season.clone(), other_season.clone()], "第十二季"),
        vec![t(&arabic_season)],
        "汉字查询要能命中阿拉伯数字写的季数"
    );
    assert_eq!(
        kept_titles(&[chinese_season.clone(), other_season], "第12季"),
        vec![t(&chinese_season)],
        "阿拉伯数字查询要能命中汉字写的季数"
    );
}

#[test]
fn positional_numerals_fold_by_place_value() {
    // `十二` 是 12 不是 102，`两千` 是 2000——不能拿一张替换表逐字换
    let twelve = title("十二生肖 2012 1080p BluRay");
    let other = title("十四罗汉 2018 1080p");

    assert_eq!(kept_titles(&[twelve.clone(), other], "12生肖"), vec![t(&twelve)]);
}

#[test]
fn years_fold_digit_by_digit() {
    // `二零二四` 没有权位字，逐位读成 2024；有权位字才按位值累加
    let y2024 = title("2024 年度合集 4K HDR");
    let y2025 = title("2025 年度合集 4K HDR");

    assert_eq!(kept_titles(&[y2024.clone(), y2025], "二零二四"), vec![t(&y2024)]);
}

#[test]
fn lone_wan_or_yi_is_not_folded_to_zero() {
    // 单个 `万` 按位值算会得 0，那是个错的数字，还会去匹配标题里的 `0`
    let zero_title = title("某资源 1080p 0 声道");
    let unit_title = title("万 有引力理论");

    assert_eq!(
        kept_titles(&[unit_title.clone(), zero_title], "万"),
        vec![t(&unit_title)],
        "`万` 该原样保留，不该变成 0 去匹配 1080p"
    );
}

#[test]
fn full_width_equals_half_width() {
    let full_width = title("ｕｂｕｎｔｕ　２２．０４ 桌面版");
    let half_width = title("ubuntu-22.04-desktop");
    let other = title("debian 12 netinst");

    assert_eq!(
        kept_titles(&[full_width.clone(), half_width.clone(), other], "ubuntu 22.04"),
        vec![t(&full_width), t(&half_width)],
        "全角只是输入法顺手打出来的，语义上和半角没有区别"
    );
}

#[test]
fn ranking_also_treats_chinese_and_arabic_numerals_as_equal() {
    // 汉字写法的那条如果没被折，就一个查询词都命中不了，会被无条件沉到后面
    let arabic_season = title("Some Show 第12季 1080p WEB-DL");
    let wrong_season = title("Some Show 第9季 1080p WEB-DL");

    let ranked = ranked_titles(&[wrong_season, arabic_season.clone()], "第十二季");

    assert_eq!(ranked[0], t(&arabic_season), "命中季数的条目要凭相关性排到前面");
}

#[test]
fn meaningless_symbols_do_not_affect_similarity() {
    // 符号在切词时只是分隔符，不制造词，也不该打断命中
    let decorated = title("ubuntu ★★★ server ??? 22.04");
    let partial = title("debian server 12");

    let ranked = ranked_titles(&[partial, decorated.clone()], "ubuntu server");

    assert_eq!(ranked[0], t(&decorated), "两个查询词都命中的条目要排在只命中一个的前面");
}

#[test]
fn query_is_not_split_into_single_chars_for_filtering() {
    // 这是刻意选的口径：拆词会把过滤放得太松。搜「三体」时它会被拆成
    // 「三」和「体」两个字，于是标题里只要有「三」就算命中——
    // (同人誌) [三崎 (二峰跨人)] 这种跟《三体》毫无关系的条目会全部留下，
    // 而那 227 条噪音正是这个成因。整段匹配才是能真正滤掉它们的做法。
    let unrelated_but_contains_san = title("(同人誌) [三崎 (二峰跨人)] 震える夜に…");
    let unrelated = title("HEYZO 3964 アナル初体験");

    let kept = kept_titles(&[unrelated_but_contains_san, unrelated], "三体");

    assert!(kept.is_empty(), "只有「三」字、没有「三体」这种写法的都该丢掉，实际：{kept:?}");
}

#[test]
fn chinese_title_with_inserted_spaces_still_matches() {
    // 实测抓到的原文，每个字之间都插了空格。归一化剔掉空格后就能匹配上——
    // 这正是「忽略分隔符」这条口径要救回来的东西（按原文比是比不上的）。
    let spaced = title("[复 仇 者 联盟4] Endgame 2019 BD 1080P x264 DD 5.1 英文 特效中英-FFans@chaospa");
    let clean = title("复仇者联盟4：终局之战 2019 4K HDR");
    let unrelated = title("[HD] SSNI-703 plump big Tits boss wife");

    assert_eq!(kept_titles(&[spaced.clone(), clean.clone(), unrelated], "复仇者联盟"), vec![t(&spaced), t(&clean)]);
}

#[test]
fn matching_is_case_insensitive() {
    let upper = title("Ubuntu-24.04-Desktop-amd64.iso");
    let lower = title("ubuntu-22.04-desktop-amd64.iso");
    let other = title("Debian 12 netinst");

    assert_eq!(kept_titles(&[upper.clone(), lower.clone(), other], "Ubuntu"), vec![t(&upper), t(&lower)]);
}

#[test]
fn blank_or_symbol_only_titles_are_dropped() {
    // 空标题不可能与任何查询相关；留着一个没有标题的卡片对用户毫无价值
    let blank = title("");
    let symbols = title("*** // ---");
    let real = title("ubuntu-22.04.iso");

    assert_eq!(kept_titles(&[blank, symbols, real.clone()], "ubuntu"), vec![t(&real)]);
}

#[test]
fn blank_keyword_drops_nothing() {
    // 切不出查询词就没有判定依据，此时必须原样返回——
    // 否则一次空关键字的调用会把整个列表清空，那是数据丢失而不是过滤。
    let items = vec![title("banana"), title(""), title("Apple")];

    assert_eq!(filter_related(items.clone(), ""), items, "没有查询词就不该丢东西");
    assert_eq!(filter_related(items.clone(), "   "), items);
    assert_eq!(filter_related(items.clone(), "///"), items, "纯符号同样切不出词");
}

#[test]
fn filter_does_not_mutate_input() {
    // Kotlin 断言入参列表不被修改；Rust 里 filter_related 拿走所有权，这条由类型系统保证。
    // 能测的只剩「同一输入重复过滤结果一致」，没有隐藏状态。
    let items = vec![title("ubuntu-24.04.iso"), title("Torrent Description")];

    let first = kept_titles(&items, "ubuntu");
    let second = kept_titles(&items, "ubuntu");

    assert_eq!(first, second);
    assert_eq!(first, vec!["ubuntu-24.04.iso"]);
}

#[test]
fn filter_then_rank_compose() {
    // 服务端与网页端就是这个用法：先滤掉无关的，再在剩下的里面按相关性排。
    // 两步各自都保持相对顺序，所以先后无所谓；这里顺带钉住最终形状。
    let irrelevant = title_src("Torrent Description", 8);
    let weak = title_src("The Official Ubuntu Server Book (Second Edition)", 6);
    let strong = title_src("ubuntu-24.04.2-desktop-amd64.iso", 6);

    let related = filter_related(vec![irrelevant, weak.clone(), strong.clone()], "ubuntu");
    let ranked = rank(related, "ubuntu");

    assert_eq!(titles(&ranked), vec![t(&strong), t(&weak)]);
}

// ---------- search_variants：给上游多送一种写法 ----------

#[test]
fn arabic_and_chinese_numerals_are_each_others_variant() {
    // 实测 btsow：搜 `复仇者联盟4` 出 50 条、`复仇者联盟四` 出 3 条，几乎不相交。
    // 两个方向都要能折，用户输入哪种写法都该拿到两边。
    assert_eq!(search_variants("复仇者联盟四"), vec!["复仇者联盟4"]);
    assert_eq!(search_variants("复仇者联盟4"), vec!["复仇者联盟四"]);
}

#[test]
fn two_digit_numbers_omit_leading_yi_for_tens() {
    // `第十二季` ↔ `第12季`。汉字写法是「十二」而不是「一十二」——
    // 后者不是中文里的说法，真发出去只会空手而归。
    assert_eq!(search_variants("第12季"), vec!["第十二季"]);
    assert_eq!(search_variants("第十二季"), vec!["第12季"]);
}

#[test]
fn pure_latin_query_yields_no_variant() {
    // 拉丁文里的数字是版本号，换个写法不是同一个东西。
    // 实测 `windows 11` 折成 `windows 十一` 会捞进 Windows 95 / Windows Update 一堆垃圾。
    // 空列表还意味着多发一次上游请求都省了。
    assert!(search_variants("windows 11").is_empty());
    assert!(search_variants("ubuntu 22.04").is_empty());
}

#[test]
fn four_digit_years_are_not_folded() {
    // `复仇者联盟二千零一十九` 毫无意义，而且年份是紧挨汉字的四位数，
    // 只有「1~2 位」这条限制能挡住它。
    assert!(search_variants("复仇者联盟2019").is_empty());
}

#[test]
fn digits_not_adjacent_to_chinese_are_not_folded() {
    // `12 怒汉`（中间有空格）与 `2012年` 里的数字处理口径不同：
    // 这里要的是「紧挨」——隔了符号就不是序号那种等价关系了。
    assert!(search_variants("12 怒汉").is_empty());
}

#[test]
fn no_numerals_no_variants() {
    assert!(search_variants("复仇者联盟").is_empty());
}

#[test]
fn variants_never_contain_the_keyword_itself() {
    // 调用方自己会把原查询放在最前面，这里再带一份就是重复打一次上游。
    for keyword in ["复仇者联盟四", "复仇者联盟4", "第十二季", "第12季"] {
        assert!(!search_variants(keyword).iter().any(|v| v == keyword), "变体里不该出现原查询：{keyword}");
    }
}

#[test]
fn all_numeral_runs_in_one_query_fold_together() {
    // 只折一处会让「第十二季第3集」变成「第12季第3集」这种半折品，
    // 两边都搜不到完整匹配。
    assert_eq!(search_variants("第十二季第三集"), vec!["第12季第3集"]);
}

// ---------- 以下为 Rust 版补充 ----------

#[test]
fn huge_chinese_numeral_run_does_not_panic() {
    // 标题是不可信输入；按位值累加时 `九亿亿亿亿` 会溢出 i64，
    // 必须与 Kotlin Long 一样静默回绕而不是 panic（debug 构建下整数溢出会 panic）
    let items = vec![title("九亿亿亿亿亿 合集"), title("ubuntu 九千亿亿亿亿")];

    let _ = rank(items.clone(), "九亿亿亿亿亿");
    let _ = filter_related(items, "九亿亿亿亿亿");
    let _ = search_variants("第九亿亿亿亿亿季");
}

#[test]
fn full_width_digits_count_as_arabic_in_variants() {
    // Kotlin `isDigit()` 认全角数字，输入法下打出 `复仇者联盟４` 也要能拿到汉字写法
    assert_eq!(search_variants("复仇者联盟４"), vec!["复仇者联盟四"]);
}
