//! 多源并发扇出。

use std::future::Future;
use std::time::Duration;

use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::error::SpiderError;
use crate::model::TorrentInfo;

/// 单个源在一次搜索里的成败。
///
/// 解析器把「站点改版」和「真的没结果」都收敛成空列表，外界分不出来；
/// 至少能把「请求直接失败」的源标出来，而不是静默少几条。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStatus {
    pub src: u32,
    pub title: String,
    pub ok: bool,
    pub count: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AggregateResult {
    /// 全部成功源的结果，按源的传入顺序拼接。
    pub items: Vec<TorrentInfo>,
    /// 每个源的状态，顺序与传入顺序一致。
    pub statuses: Vec<SourceStatus>,
}

impl AggregateResult {
    /// 判定条件是「全员失败」，不是「没结果」：全部源都正常返回空列表是正常的空结果。
    pub fn all_failed(&self) -> bool {
        !self.statuses.is_empty() && self.statuses.iter().all(|s| !s.ok)
    }
}

/// 执行一个源的搜索，超时或失败都折算成 [`SourceStatus`]，不向外抛。
pub async fn search_one<Fut>(src: u32, title: &str, timeout: Duration, search: Fut) -> (SourceStatus, Vec<TorrentInfo>)
where
    Fut: Future<Output = Result<Vec<TorrentInfo>, SpiderError>>,
{
    let outcome = match tokio::time::timeout(timeout, search).await {
        Ok(result) => result,
        Err(_) => Err(SpiderError::Timeout(timeout)),
    };
    match outcome {
        Ok(items) => {
            let items: Vec<TorrentInfo> = items.into_iter().map(TorrentInfo::normalized).collect();
            let status = SourceStatus { src, title: title.to_owned(), ok: true, count: items.len(), error: None };
            (status, items)
        }
        Err(error) => {
            let status =
                SourceStatus { src, title: title.to_owned(), ok: false, count: 0, error: Some(error.describe()) };
            (status, Vec::new())
        }
    }
}

/// 并发搜索多个源。
///
/// 每个源单独计时：一个源挂住只会让它自己超时，不拖垮其他源。
/// 返回的 future 被丢弃（例如客户端断开）时，所有在途请求随之取消。
pub async fn fan_out<I, F, Fut>(targets: I, per_source_timeout: Duration, search: F) -> AggregateResult
where
    I: IntoIterator<Item = (u32, String)>,
    F: Fn(u32) -> Fut,
    Fut: Future<Output = Result<Vec<TorrentInfo>, SpiderError>>,
{
    let tasks = targets.into_iter().map(|(src, title)| {
        let future = search(src);
        async move { search_one(src, &title, per_source_timeout, future).await }
    });
    let outcomes = join_all(tasks).await;

    let mut result = AggregateResult::default();
    for (status, items) in outcomes {
        result.statuses.push(status);
        result.items.extend(items);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::HttpError;

    fn item(src: u32, title: &str) -> TorrentInfo {
        TorrentInfo { src, title: Some(title.into()), hash: Some(String::new()), ..Default::default() }
    }

    fn targets(codes: &[u32]) -> Vec<(u32, String)> {
        codes.iter().map(|c| (*c, format!("S{c}"))).collect()
    }

    #[tokio::test]
    async fn one_failing_source_does_not_break_the_rest() {
        let result = fan_out(targets(&[0, 1, 2]), Duration::from_secs(1), |src| async move {
            match src {
                1 => Err(HttpError::status("GET", "https://x", 403, "blocked").into()),
                _ => Ok(vec![item(src, &format!("t{src}"))]),
            }
        })
        .await;
        assert_eq!(result.items.len(), 2);
        assert!(!result.all_failed());
        let failed = &result.statuses[1];
        assert!(!failed.ok);
        assert_eq!(failed.count, 0);
        assert!(failed.error.as_deref().unwrap().contains("403"));
        assert_eq!(result.items[0].hash, None, "blank fields are normalized");
    }

    #[tokio::test]
    async fn empty_results_are_not_failures() {
        let result = fan_out(targets(&[0, 1]), Duration::from_secs(1), |_| async { Ok(vec![]) }).await;
        assert!(result.items.is_empty());
        assert!(!result.all_failed());
        assert!(result.statuses.iter().all(|s| s.ok));
    }

    #[tokio::test]
    async fn all_failed_is_detected() {
        let result = fan_out(targets(&[0, 1]), Duration::from_secs(1), |_| async {
            Err(HttpError::status("GET", "u", 500, "").into())
        })
        .await;
        assert!(result.all_failed());
    }

    #[tokio::test]
    async fn slow_source_times_out_on_its_own() {
        let result = fan_out(targets(&[0, 1]), Duration::from_millis(50), |src| async move {
            if src == 1 {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Ok(vec![item(src, "x")])
        })
        .await;
        assert!(result.statuses[0].ok);
        assert!(!result.statuses[1].ok);
        assert_eq!(result.statuses[1].error.as_deref(), Some("请求超时"));
    }
}
