//! `GET /api/search`：多源搜索、搜索优化与违禁过滤的编排。

use std::collections::HashSet;
use std::net::IpAddr;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use futures::future::join_all;
use serde::Serialize;
use torrent_spider::search::{aggregate, relevance};
use torrent_spider::{SourceStatus, TorrentInfo, TorrentSource};

use super::extract::{Params, params};
use super::params::{parse_flag, parse_page, parse_search_src, require_keyword};
use super::{ApiError, ClientIp, codes};
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub keyword: String,
    pub page: u32,
    /// 单源搜索时是站点编号；聚合搜索时为 null。
    pub src: Option<u32>,
    pub items: Vec<TorrentInfo>,
    /// 每个源这一页的成败，让界面能显示「某个源挂了」而不是静默少几条。
    ///
    /// `count` 是违禁过滤之后、`filter` 之前的条数：`count` 与 `items` 的差是「已隐藏的无关结果」，
    /// 被违禁过滤拦下的条目则连数量都不透露。
    pub sources: Vec<SourceStatus>,
    /// 为这次搜索额外打给上游的写法（不含 `keyword` 本身）；只按字面搜了一次时为空。
    pub queries: Vec<String>,
}

/// `GET /api/search?q=&page=1&src=all&filter=1&optimize=1`
///
/// - `optimize`（默认开）：查询里有紧挨汉字的数字时，把另一种写法也搜一遍再按资源去重合并；
/// - `filter`（默认开）：丢掉与关键词完全沾不上边的条目。
///
/// 两者默认值必须一致：网页端是同一个「搜索优化」开关，不一致会让什么都不传的调用方落进
/// 「补搜了却没滤噪音」这种没有开关位置对应的半开状态。结果一律按相关性排序。
///
/// 单源失败直接 502；聚合时单源失败只标记该源，全部源都失败才 502。
/// 全部源都成功但没结果（或全被过滤掉）是正常的 200 + 空列表。
pub async fn search(
    State(state): State<AppState>,
    ClientIp(client): ClientIp,
    query: Params,
) -> Result<Json<SearchResponse>, ApiError> {
    let params = params(query)?;
    let param = |name: &str| params.get(name).map(String::as_str);
    let keyword = require_keyword(param("q"))?;
    let page = parse_page(param("page"))?;
    let src = parse_search_src(param("src"), &state.sources)?;
    let filter = parse_flag(param("filter"), true, "filter", codes::INVALID_FILTER)?;
    let optimize = parse_flag(param("optimize"), true, "optimize", codes::INVALID_OPTIMIZE)?;

    let extra_queries = if optimize { relevance::search_variants(&keyword) } else { Vec::new() };
    let queries: Vec<&str> =
        std::iter::once(keyword.as_str()).chain(extra_queries.iter().map(String::as_str)).collect();

    let targets: Vec<&TorrentSource> = match src {
        Some(code) => state.sources.iter().filter(|s| s.src == code).collect(),
        None => state.sources.iter().collect(),
    };
    let outcomes = join_all(targets.iter().map(|source| fetch_source(&state, source, &queries, page, client))).await;

    let mut statuses = Vec::with_capacity(outcomes.len());
    let mut items = Vec::new();
    for (status, found) in outcomes {
        statuses.push(status);
        items.extend(found);
    }

    if !statuses.is_empty() && statuses.iter().all(|s| !s.ok) {
        let message = match src {
            Some(_) => statuses[0].error.clone().unwrap_or_else(|| "来源不可用".to_owned()),
            None => {
                let detail = statuses
                    .iter()
                    .map(|s| format!("{}={}", s.title, s.error.as_deref().unwrap_or("")))
                    .collect::<Vec<_>>()
                    .join("；");
                format!("全部 {} 个来源都失败了：{detail}", statuses.len())
            }
        };
        return Err(ApiError::new(StatusCode::BAD_GATEWAY, codes::ALL_SOURCES_FAILED, message));
    }

    let items = if filter { relevance::filter_related(items, &keyword) } else { items };
    let items = relevance::rank(items, &keyword);
    Ok(Json(SearchResponse { keyword, page, src, items, sources: statuses, queries: extra_queries }))
}

/// 打一个源：每种写法各搜一次（串行，同一站点并发打两次容易被限流），结果按资源去重合并。
///
/// 只要有一种写法成功这个源就算可用；全失败才标失败，错误取第一次的。
/// 违禁过滤在这里、在去重与计数之前逐个写法地做：变体捞回来的违禁条目同样要拦，
/// 而且 `count` 必须是过滤后的数，否则 `count - items.len()` 会泄露被拦了几条。
async fn fetch_source(
    state: &AppState,
    source: &TorrentSource,
    queries: &[&str],
    page: u32,
    client: Option<IpAddr>,
) -> (SourceStatus, Vec<TorrentInfo>) {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();
    let mut first_error = None;
    let mut succeeded = false;

    for &query in queries {
        let future = state.search.search(source.src, query, page);
        let (status, found) = aggregate::search_one(source.src, &source.title, state.search_timeout, future).await;
        if !status.ok {
            first_error = first_error.or(status.error);
            continue;
        }
        succeeded = true;
        let found = match &state.sensitive {
            Some(filter) => filter.filter(found, query, client),
            None => found,
        };
        for item in found {
            if seen.insert(dedupe_key(&item)) {
                merged.push(item);
            }
        }
    }

    let status = SourceStatus {
        src: source.src,
        title: source.title.clone(),
        ok: succeeded,
        count: merged.len(),
        error: if succeeded { None } else { first_error },
    };
    (status, merged)
}

/// 去重键：hash > 详情页地址 > 小写标题，先到的留下。TorLock、1337 的搜索页不给 hash，详情页地址才是唯一键。
fn dedupe_key(item: &TorrentInfo) -> String {
    item.hash
        .as_deref()
        .map(str::to_ascii_uppercase)
        .or_else(|| item.detail_url.clone())
        .unwrap_or_else(|| item.title.as_deref().unwrap_or("").to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_prefers_hash_then_detail_url_then_title() {
        let item = |hash: Option<&str>, detail: Option<&str>, title: &str| TorrentInfo {
            hash: hash.map(str::to_owned),
            detail_url: detail.map(str::to_owned),
            title: Some(title.to_owned()),
            ..Default::default()
        };
        assert_eq!(dedupe_key(&item(Some("ab"), Some("https://d"), "T")), "AB");
        assert_eq!(dedupe_key(&item(None, Some("https://d"), "T")), "https://d");
        assert_eq!(dedupe_key(&item(None, None, "Ubuntu")), "ubuntu");
    }
}
