//! 接口行为契约。全部走假网关，不联网：站点会改版、会 403、会被墙，
//! 依赖真站的测试在 CI 上只能是噪音。

mod magnet;
mod meta;
mod optimize;
mod search;
mod sensitive;
mod support;
mod web;
