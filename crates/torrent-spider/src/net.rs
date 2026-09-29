//! 网络与页面解析的基础设施：HTTP 抽象、HTML 取值、百分号编解码。

pub mod encoding;
pub mod html;
mod http;
#[cfg(feature = "reqwest")]
mod reqwest_client;

pub use http::{HeadResponse, Header, HttpClient, HttpError};
#[cfg(feature = "reqwest")]
pub use reqwest_client::{ReqwestHttpClient, ReqwestHttpClientBuilder};
