//! 搜索结果的编排：多源并发扇出，以及按相关性排序、过滤。

pub mod aggregate;
pub mod relevance;

pub use aggregate::{AggregateResult, SourceStatus};
