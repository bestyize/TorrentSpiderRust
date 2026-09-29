//! 基于 `scraper` 的 HTML 取值工具。
//!
//! 这些站点改版时类名经常变，但 DOM 结构和内容特征往往还在。
//! 这里的 `*_any` 系列依次尝试多个候选选择器，改版时只损失其中一个分支；
//! [`text_matching`] 则按内容特征兜底。

use std::sync::LazyLock;

use regex::Regex;
use scraper::{ElementRef, Html, Selector};

/// 形如 `700 MB` / `1.5 GiB` / `336741734` 的尺寸文本。
pub static SIZE_TEXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\d+(?:\.\d+)?\s*[KMGT]?i?B$").unwrap());

/// 形如 `2026-03-15` 的日期文本。
pub static DATE_TEXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap());

/// 零宽字符：站点拿它当反爬手段插进标题，屏幕上看不见，复制去搜索时却对不上。
static ZERO_WIDTH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("[\u{200B}\u{200C}\u{200D}\u{2060}\u{FEFF}]").unwrap());

/// 解析整页 HTML。
pub fn parse(html: &str) -> Html {
    Html::parse_document(html)
}

/// 解析选择器；语法非法时返回 `None`，由调用方跳过该候选。
pub fn selector(css: &str) -> Option<Selector> {
    if css.trim().is_empty() {
        return None;
    }
    Selector::parse(css).ok()
}

/// 全部命中 `css` 的后代元素（不含自身）；选择器非法时为空。
pub fn select<'a>(scope: ElementRef<'a>, css: &str) -> Vec<ElementRef<'a>> {
    match selector(css) {
        Some(sel) => scope.select(&sel).collect(),
        None => Vec::new(),
    }
}

/// 第一个命中 `css` 的后代元素。
pub fn select_first<'a>(scope: ElementRef<'a>, css: &str) -> Option<ElementRef<'a>> {
    selector(css).and_then(|sel| scope.select(&sel).next())
}

/// 依次尝试多个选择器，返回第一个命中的元素。
pub fn select_first_any<'a>(scope: ElementRef<'a>, selectors: &[&str]) -> Option<ElementRef<'a>> {
    selectors.iter().find_map(|css| select_first(scope, css))
}

/// 依次尝试多个选择器，返回第一个非空的结果集。
pub fn select_all_any<'a>(scope: ElementRef<'a>, selectors: &[&str]) -> Vec<ElementRef<'a>> {
    selectors.iter().map(|css| select(scope, css)).find(|hits| !hits.is_empty()).unwrap_or_default()
}

/// 依次尝试多个选择器，返回第一个命中元素的文本；都没命中返回空串。
pub fn text_any(scope: ElementRef<'_>, selectors: &[&str]) -> String {
    select_first_any(scope, selectors).map(text).unwrap_or_default()
}

/// 依次尝试多个 class 名，返回第一个命中元素的文本。
pub fn class_text_any(scope: ElementRef<'_>, class_names: &[&str]) -> String {
    class_names.iter().find_map(|name| by_class(scope, name)).map(text).unwrap_or_default()
}

/// 第一个带有 class `name` 的后代元素。
pub fn by_class<'a>(scope: ElementRef<'a>, name: &str) -> Option<ElementRef<'a>> {
    scope.descendent_elements().skip(1).find(|el| el.value().classes().any(|c| c == name))
}

/// 属性值（去掉首尾空白）；不存在时返回空串。
pub fn attr<'a>(element: ElementRef<'a>, name: &str) -> &'a str {
    element.value().attr(name).map(str::trim).unwrap_or("")
}

/// 元素文本：拼接全部文本节点，空白压成单个空格并去掉首尾空白。
///
/// 相邻内联元素之间不补空格：`<em>微微一笑</em><em>很</em>` 得到「微微一笑很」，
/// 与浏览器里看到的一致。
pub fn text(element: ElementRef<'_>) -> String {
    collapse_whitespace(&element.text().collect::<String>())
}

/// 元素的原始文本（不压缩内部空白），只去掉首尾空白。
pub fn whole_text(element: ElementRef<'_>) -> String {
    element.text().collect::<String>().trim().to_owned()
}

/// 去掉零宽字符。
pub fn without_zero_width(value: &str) -> String {
    ZERO_WIDTH.replace_all(value, "").into_owned()
}

/// 标题用：原样文本、去零宽字符、压缩空白。
pub fn title_text(element: ElementRef<'_>) -> String {
    collapse_whitespace(&without_zero_width(&element.text().collect::<String>()))
}

/// 第一个文本完全匹配 `pattern` 的 `tag` 后代元素的文本。
///
/// 类名失效时的兜底：类名会改，但「一个只写着 `700 MB` 的单元格」这个特征不会。
pub fn text_matching(scope: ElementRef<'_>, pattern: &Regex, tag: &str) -> String {
    select(scope, tag).into_iter().map(text).find(|t| pattern.is_match(t)).unwrap_or_default()
}

/// 第一个 `href` 以 `prefix` 开头的后代元素的 `href`。
///
/// 不用 `[href^=magnet:]` 这类选择器：未加引号的 `:` 让它不是合法选择器，
/// 非法选择器会被静默跳过，表现为「一直取不到」。
pub fn href_with_prefix(scope: ElementRef<'_>, prefix: &str) -> Option<String> {
    scope
        .descendent_elements()
        .filter_map(|el| el.value().attr("href"))
        .map(str::trim)
        .find(|href| href.starts_with(prefix))
        .map(str::to_owned)
}

/// 父元素。
pub fn parent(element: ElementRef<'_>) -> Option<ElementRef<'_>> {
    element.parent().and_then(ElementRef::wrap)
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(html: &str) -> Html {
        Html::parse_document(html)
    }

    #[test]
    fn select_first_any_skips_invalid_and_missing_selectors() {
        let html = doc(r#"<div><span class="b">B</span><i id="c">C</i></div>"#);
        let root = html.root_element();
        assert_eq!(text_any(root, &["", "a[href^=magnet:]", ".missing", ".b"]), "B");
        assert_eq!(text_any(root, &["#c", ".b"]), "C");
        assert_eq!(text_any(root, &[".none"]), "");
    }

    #[test]
    fn select_all_any_returns_first_non_empty_set() {
        let html = doc("<ul><li>1</li><li>2</li></ul>");
        let hits = select_all_any(html.root_element(), &[".row", "li"]);
        assert_eq!(hits.len(), 2);
        assert!(select_all_any(html.root_element(), &[".row"]).is_empty());
    }

    #[test]
    fn text_does_not_insert_spaces_between_inline_elements() {
        let html = doc("<h3><em>微微一笑</em><em>很</em><em>倾城</em>  \n 2024</h3>");
        let h3 = select_first(html.root_element(), "h3").unwrap();
        assert_eq!(text(h3), "微微一笑很倾城 2024");
        assert_eq!(whole_text(h3), "微微一笑很倾城  \n 2024");
    }

    #[test]
    fn zero_width_characters_are_removed_from_titles() {
        let html = doc("<b>Ub\u{200B}untu\u{FEFF} 24</b>");
        let b = select_first(html.root_element(), "b").unwrap();
        assert_eq!(title_text(b), "Ubuntu 24");
        assert_eq!(without_zero_width("a\u{200C}b\u{2060}c"), "abc");
    }

    #[test]
    fn text_matching_finds_cells_by_content() {
        let html = doc("<table><tr><td>name</td><td> 1.5 GiB </td><td>2026-03-15</td></tr></table>");
        let root = html.root_element();
        assert_eq!(text_matching(root, &SIZE_TEXT, "td"), "1.5 GiB");
        assert_eq!(text_matching(root, &DATE_TEXT, "td"), "2026-03-15");
        assert_eq!(text_matching(root, &SIZE_TEXT, "span"), "");
        for ok in ["700 MB", "336741734B", "2kb", "1.2 TiB"] {
            assert!(SIZE_TEXT.is_match(ok), "{ok}");
        }
        assert!(!SIZE_TEXT.is_match("12 files"));
    }

    #[test]
    fn class_and_href_helpers() {
        let html = doc(r#"<div><p class="x size">9 MB</p><a href=" magnet:?xt=1 ">m</a><a href="/d">d</a></div>"#);
        let root = html.root_element();
        assert_eq!(class_text_any(root, &["nope", "size"]), "9 MB");
        assert_eq!(href_with_prefix(root, "magnet:").as_deref(), Some("magnet:?xt=1"));
        assert_eq!(href_with_prefix(root, "ftp:"), None);
        let a = select_first(root, "a").unwrap();
        assert_eq!(attr(a, "href"), "magnet:?xt=1");
        assert_eq!(attr(a, "title"), "");
        assert_eq!(parent(a).unwrap().value().name(), "div");
    }
}
