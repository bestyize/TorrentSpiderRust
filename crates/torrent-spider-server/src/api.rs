//! JSON 接口，每个端点一个模块。字段名与错误码见 `docs/api.md`，只增不改。

mod error;
mod extract;
mod magnet;
mod meta;
mod params;
mod search;

pub use error::{ApiError, codes};
pub use extract::ClientIp;
pub use magnet::{MagnetResponse, magnet_url};
pub use meta::{HealthResponse, SourcesResponse, health, sources};
pub use params::{ALL_SOURCES, MAX_PAGE};
pub use search::{SearchResponse, search};
