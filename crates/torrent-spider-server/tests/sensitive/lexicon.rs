//! 违禁词匹配的行为契约（移植自 Kotlin `SensitiveLexiconTest`、`SensitiveLexiconRealWordsTest`、
//! `BuiltinLexiconTest`）。
//!
//! 最重要的性质不是「能拦住多少」，而是**不误伤**：词库里有「法院」「大师」这类正常词，
//! `sm`、`anal` 是大量英文单词的子串。所以边界全按「宁漏不错」来钉。
//! 规则用例用 `SensitiveLexicon::build` 注入小词表；钉真实词条的用例用内置词库。

use torrent_spider::TorrentInfo;
use torrent_spider::search::relevance::is_cjk;
use torrent_spider_server::sensitive::lexicon::SensitiveLexicon;

/// 与 `src/sensitive/lexicon.rs` 编进去的是同一个文件。
const BUILTIN_JSON: &str = include_str!("../../data/lexicon.json");

fn lexicon_of(words: &[&str]) -> SensitiveLexicon {
    SensitiveLexicon::build(words)
}

fn item(title: &str, detail_url: Option<&str>) -> TorrentInfo {
    TorrentInfo {
        title: Some(title.to_owned()).filter(|t| !t.is_empty()),
        detail_url: detail_url.map(str::to_owned),
        src: 6,
        ..TorrentInfo::default()
    }
}

// ---------- 汉字词 ----------

#[test]
fn 两个汉字的词照常命中() {
    let lexicon = lexicon_of(&["奶子"]);
    assert!(lexicon.matches("奶子"));
    assert!(lexicon.matches("【无删减】奶子大合集"));
}

#[test]
fn 单个汉字的词整条不参与匹配() {
    // 一个汉字做子串等于「凡是含这个字的标题全拦」，误杀面积太大
    let lexicon = lexicon_of(&["奶"]);
    assert!(lexicon.is_empty());
    assert!(!lexicon.matches("奶油小方"));
}

#[test]
fn 汉字词按子串匹配不要求整词() {
    // 词库对变体字的覆盖靠子串；代价是同样会命中正常语境里的用法，这是词库的性质
    let lexicon = lexicon_of(&["写真"]);
    assert!(lexicon.matches("裸体写真集"));
    assert!(lexicon.matches("写真"));
    assert!(!lexicon.matches("写生课"), "没出现这个词就不该命中");
}

#[test]
fn 词库里的正常词仍然会命中这是已知代价() {
    // 口径是「汉字词至少 2 个字就子串匹配」，如实钉住现状：不是 bug
    assert!(lexicon_of(&["法院"]).matches("宪法法院"));
    assert!(lexicon_of(&["大师"]).matches("星海大师兄的评测"));
}

#[test]
fn 汉字词大小写不敏感地匹配混合词() {
    // 混合词只要有 ≥2 个汉字就进子串表，字母部分按小写比
    let lexicon = lexicon_of(&["法X功"]);
    assert!(lexicon.matches("关于法x功的讨论"));
    assert!(lexicon.matches("关于法X功的讨论"));
}

// ---------- 拉丁词 ----------

#[test]
fn 短拉丁词整条丢掉() {
    // `anal` 命中 analysis / analytics，`sm`、`flg`、`a4u`、`gcd` 同理
    for short in ["sm", "flg", "a4u", "gcd"] {
        let lexicon = lexicon_of(&[short]);
        assert!(lexicon.is_empty(), "应当被丢掉：{short}");
        assert!(!lexicon.matches("A totally unrelated title about analytics"), "不该命中：{short}");
    }
    // 混了汉字的超短词同样丢掉：一个汉字不够判，拉丁部分又只有两位
    assert!(!lexicon_of(&["k粉"]).matches("k 粉色的天空"));
}

#[test]
fn 拉丁词必须整词命中不能命中单词内部() {
    let lexicon = lexicon_of(&["anal"]);
    assert!(!lexicon.matches("[medical] The Matrix"), "`anal` 不该命中 medical 里的字串");
    assert!(!lexicon.matches("Analysis of Algorithms"), "不该命中 Analysis");
    assert!(lexicon.matches("anal amateur"), "整词出现时应当命中");
    assert!(lexicon.matches("AMATEUR ANAL"), "大小写不敏感");
}

#[test]
fn 拉丁词与标点相邻也算整词() {
    let lexicon = lexicon_of(&["anal"]);
    assert!(lexicon.matches("xxx (anal) xxx"));
    assert!(lexicon.matches("anal-2024.mkv"));
}

#[test]
fn 拉丁词与汉字相邻也算整词() {
    // 汉字是分隔符
    let lexicon = lexicon_of(&["anal"]);
    assert!(lexicon.matches("合集anal合集"));
}

#[test]
fn 拉丁词不命中更长单词的内部() {
    let lexicon = lexicon_of(&["youtube"]);
    assert!(!lexicon.matches("Some 2160p YouTubeMovies rip"));
    assert!(lexicon.matches("Youtube 1080p"));
}

#[test]
fn 字母段与数字段分开切纯数字词能命中() {
    let lexicon = lexicon_of(&["198964"]);
    assert!(lexicon.matches("xxx.abc198964.xxx"));
    assert!(!lexicon.matches("abc1989645"), "更长的一串数字不是同一个词");
}

// ---------- filter ----------

#[test]
fn 关键词命中时整批结果为空() {
    let lexicon = lexicon_of(&["奶子"]);
    let items = vec![item("奶子大合集", None), item("无关内容", None)];
    assert_eq!(lexicon.filter(items.clone(), "奶子"), Vec::<TorrentInfo>::new());
    assert_eq!(lexicon.filter(items, "【无删减】奶子"), Vec::<TorrentInfo>::new());
}

#[test]
fn 命中的条目整条丢掉其余原样保留() {
    let lexicon = lexicon_of(&["奶子"]);
    let keep_a = item("ubuntu-24.04.iso", None);
    let keep_b = item("无关内容", None);
    let drop = item("奶子大合集", None);

    let filtered = lexicon.filter(vec![keep_a.clone(), drop, keep_b.clone()], "linux");

    assert_eq!(filtered, vec![keep_a, keep_b]);
}

#[test]
fn 标题为空时按详情页地址兜底() {
    let lexicon = lexicon_of(&["奶子"]);
    let bad = item("", Some("https://example.test/torrent/奶子合集.html"));
    let good = item("", Some("https://example.test/torrent/12345.html"));
    let blank = TorrentInfo { title: Some("   ".to_owned()), ..item("", Some("https://example.test/奶子/1")) };
    let no_url = item("", None);

    assert_eq!(lexicon.filter(vec![bad, good.clone(), blank, no_url.clone()], "x"), vec![good, no_url]);
}

#[test]
fn 空词表不丢任何东西() {
    // 词库拉不到时是空表，此时必须完全不拦——宁可不过滤也不能拦光
    let empty = SensitiveLexicon::empty();
    let items = vec![item("任意内容", None), item("奶子大合集", None)];
    assert_eq!(empty.filter(items.clone(), "奶子"), items);
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert!(!empty.matches("奶子"));
}

#[test]
fn 空列表原样返回() {
    let lexicon = lexicon_of(&["奶子"]);
    assert_eq!(lexicon.filter(Vec::new(), "x"), Vec::<TorrentInfo>::new());
}

// ---------- build / parse ----------

#[test]
fn build_去重并报告生效词条数() {
    let lexicon = lexicon_of(&["奶子", "奶子", "写真", "anal", "ANAL", "sm"]);
    // sm 被丢，奶子/写真/anal 各算一条（去重不分大小写）
    assert_eq!(lexicon.len(), 3);
}

#[test]
fn build_忽略空白词与首尾空格() {
    let lexicon = lexicon_of(&["", "   ", "  奶子  "]);
    assert_eq!(lexicon.len(), 1);
    assert!(lexicon.matches("奶子大合集"));
}

#[test]
fn parse_接受词库的真实形状() {
    // 多出来的字段要忽略
    let raw = r#"{"lastUpdateDate":"2025/07/25","words":["奶子","写真","sm"]}"#;
    let lexicon = SensitiveLexicon::parse(raw).expect("应当解析成功");
    assert_eq!(lexicon.len(), 2, "sm 应当被丢掉");
    assert!(lexicon.matches("奶子大合集"));
}

#[test]
fn parse_对坏数据返回_none_而不是_panic() {
    // 格式变了或下了一半，应当由调用方决定沿用旧表
    assert!(SensitiveLexicon::parse("not json").is_none());
    assert!(SensitiveLexicon::parse(r#"{"words": 12}"#).is_none());
    assert!(SensitiveLexicon::parse("").is_none());
    assert!(SensitiveLexicon::parse(r#"{"words": ["奶子""#).is_none(), "截断的 JSON");
}

#[test]
fn parse_对缺_words_字段当空词表处理() {
    let lexicon = SensitiveLexicon::parse(r#"{"lastUpdateDate":"x"}"#).expect("缺字段不算解析失败");
    assert!(lexicon.is_empty());
}

#[test]
fn 汉字判定区间与核心库一致() {
    // 词表直接复用核心库的 is_cjk；这里钉住区间本身
    assert!(is_cjk('奶'));
    assert!(is_cjk('\u{4E00}'), "CJK 基本区起点");
    assert!(is_cjk('\u{9FFF}'), "CJK 基本区终点");
    assert!(is_cjk('\u{3400}'), "扩展 A 区起点");
    assert!(!is_cjk('a'));
    assert!(!is_cjk('1'));
    assert!(!is_cjk('\u{3000}'), "全角空格不是汉字");
}

// ---------- 真实词库里的原词（SensitiveLexiconRealWordsTest）----------

/// 真实词库里的原词，按误杀风险挑出来的几类。
fn real_words() -> SensitiveLexicon {
    SensitiveLexicon::build([
        // 误杀主因：超短拉丁词
        "sm",
        "anal",
        "flg",
        "a4u",
        "gcd", //
        // 混了汉字的超短词
        "k粉",
        "9评", //
        // 正常汉字词——词库里确实有，口径选了保留
        "法院",
        "大师",
        "刺激",
        "集合",
        "写真",
        "自拍",
        "激情",
        "无码", //
        // 长度够的拉丁词：保留，且必须整词命中
        "youtube",
        "cocain",
        "falungong",
        "198964", //
        // 长度够但会命中单词内部，只有整词匹配才拦得住
        "anal", //
        // 长度不够，丢掉
        "sex",
    ])
}

#[test]
fn 真实词_超短拉丁词被丢掉() {
    let lexicon = real_words();
    assert!(!lexicon.matches("A medical analysis of flag work"));
    assert!(!lexicon.matches("gcd of 12 and 18"));
}

#[test]
fn 真实词_混了汉字的超短词被丢掉() {
    let lexicon = real_words();
    assert!(!lexicon.matches("共 9 评论"));
    assert!(!lexicon.matches("k 粉色的房间"));
}

#[test]
fn 真实词_正常汉字词仍会命中() {
    // 不是 bug：如实钉住，免得以后有人以为这是回归而顺手「修」掉
    let lexicon = real_words();
    assert!(lexicon.matches("法院的判决书"));
    assert!(lexicon.matches("集合啦动物森友会"));
}

#[test]
fn 真实词_长度够的拉丁词必须整词命中() {
    let lexicon = real_words();
    assert!(!lexicon.matches("Some 2160p YouTubeMovies rip"));
    assert!(lexicon.matches("Youtube 1080p"));
    // `cocain` 是 `cocaine` 的前缀，但两者不是一个词
    assert!(!lexicon.matches("cocaine blues"));
    assert!(lexicon.matches("cocain powder"));
}

#[test]
fn 真实词_纯数字词在字母粘连时也能命中() {
    assert!(real_words().matches("xxx.abc198964.xxx"));
}

#[test]
fn 真实词_生效词条数等于去重加口径筛过的结果() {
    // 21 个原词里：anal 重复一次，sm/flg/a4u/gcd/sex 太短，k粉/9评 汉字不够 → 留下 13 条
    assert_eq!(real_words().len(), 13);
}

// ---------- 内置词库（BuiltinLexiconTest）----------

#[test]
fn 内置词库是一份合法的词库_json() {
    let value: serde_json::Value = serde_json::from_str(BUILTIN_JSON).expect("内置词库应当是合法 JSON");
    let words = value["words"].as_array().expect("应当含 words 数组");
    // konsheng 2408 条 + gogoswift 929 条 + 补充 30 条光杆词，合并去重后 2014 条；
    // 2026-09-30 手工补充 15 条（其中「肏」重复一次），共 2029 条
    assert_eq!(words.len(), 2029);
    assert!(words.iter().all(serde_json::Value::is_string));
}

#[test]
fn 内置词库生效词条数与打包时一致() {
    // 按匹配口径收窄后生效 1929 条（汉字词 1849 + 拉丁词 80）；其余是「一个汉字 + 字母」
    // 和「<4 字符的拉丁词」，被有意丢弃。数字变了说明词库被重新生成过——确认是有意的再改这里。
    let lexicon = SensitiveLexicon::builtin();
    assert!(!lexicon.is_empty(), "内置词库解析失败会退回空表");
    assert_eq!(lexicon.len(), 1929);
}

#[test]
fn 补充的光杆词确实生效了() {
    // 两份原始词库只有组合词（「色情网站」），没有通用指代词（「色情」本身）
    let lexicon = SensitiveLexicon::builtin();
    for word in ["色情", "成人", "约炮", "萝莉"] {
        assert!(lexicon.matches(&format!("前缀{word}后缀")), "补充词应当生效：{word}");
    }
}

#[test]
fn 手工补充的词确实生效了() {
    let lexicon = SensitiveLexicon::builtin();
    for word in ["破处", "一炮", "鲍鱼", "处女", "干哭", "床上", "探花", "白虎", "大神", "美鲍", "啪啪", "约操", "强上"]
    {
        assert!(lexicon.matches(&format!("前缀{word}后缀")), "补充词应当生效：{word}");
    }
    // 单字「肏」也加进了词库，但单个汉字不参与匹配（误杀面太大），这是匹配口径的有意取舍
    assert!(!lexicon.matches("前缀肏后缀"));
}

#[test]
fn 内置词库里的词确实能拦下() {
    // 不写死具体的词：从词库里取一个生效的纯汉字词，验证「加载 + 匹配」链路通了
    let value: serde_json::Value = serde_json::from_str(BUILTIN_JSON).unwrap();
    let sample = value["words"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(serde_json::Value::as_str)
        .find(|word| word.chars().count() >= 2 && word.chars().all(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c)))
        .expect("词库里应当有汉字词");
    assert!(SensitiveLexicon::builtin().matches(&format!("前缀{sample}后缀")), "词库里的词应当能命中：{sample}");
}

#[test]
fn 误杀主因的超短拉丁词仍然不在词表里() {
    let lexicon = SensitiveLexicon::builtin();
    assert!(!lexicon.matches("A medical analysis of flag work"), "超短拉丁词不该生效");
    assert!(!lexicon.matches("the gcd of two numbers"), "超短拉丁词不该生效");
}

#[test]
fn 正常内容不会被误伤() {
    // 只挑「一眼不可能命中」的测不出风险；这几条是内置词库下的真实边界
    let lexicon = SensitiveLexicon::builtin();
    for title in [
        "ubuntu-22.04-desktop-amd64.iso",
        "The Matrix 1999 1080p BluRay",
        "复仇者联盟4：终局之战",
        "Windows 11 24H2 简体中文版",
    ] {
        assert!(!lexicon.matches(title), "正常标题不该被拦：{title}");
    }
}

#[test]
fn 已知的误杀代价如实钉住() {
    // 补光杆词是用户明确接受的取舍，但代价要写下来。删掉这条不会让误杀消失，
    // 只会让下一个改词库的人不知道有这回事。
    let lexicon = SensitiveLexicon::builtin();
    // 「素人」也是《素人特工》(The Rookies, 2019) 片名的一部分
    assert!(lexicon.matches("素人特工[国语音轨+中文字幕] The Rookies 2019"), "已知误杀：素人");
    // 「成人」会命中「成人教育」「成人高考」
    assert!(lexicon.matches("成人高考数学真题解析"), "已知误杀：成人");
    // 「骑兵」「步兵」在番号语境里指有码/无码，但也是军事词
    assert!(lexicon.matches("骑兵与步兵的战术配合"), "已知误杀：骑兵/步兵");
}
