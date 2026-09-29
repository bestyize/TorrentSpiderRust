//! 命令行搜索示例：
//!
//! ```sh
//! cargo run -p torrent-spider --features reqwest --example search -- ubuntu        # 聚合全部来源
//! cargo run -p torrent-spider --features reqwest --example search -- ubuntu 6      # 只搜 BtSow
//! TORRENT_PROXY=http://127.0.0.1:7897 cargo run -p torrent-spider --features reqwest --example search -- ubuntu
//! ```

use std::sync::Arc;

use torrent_spider::{ReqwestHttpClient, TorrentSpiderClient, TorrentSrc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let keyword = args.next().ok_or("用法：search <关键字> [站点编号]")?;
    let src = args.next().map(|s| s.parse::<u32>()).transpose()?;

    let mut http = ReqwestHttpClient::builder();
    if let Ok(proxy) = std::env::var("TORRENT_PROXY") {
        http = http.proxy(proxy);
    }
    let client = TorrentSpiderClient::new(Arc::new(http.build()?));

    let items = match src {
        Some(code) => {
            let src = TorrentSrc::from_code(code).ok_or("未知的站点编号")?;
            client.search(src, &keyword, 1).await?
        }
        None => {
            let result = client.search_all(&keyword, 1).await;
            for status in result.statuses.iter().filter(|s| !s.ok) {
                eprintln!("[{}] 失败：{}", status.title, status.error.as_deref().unwrap_or(""));
            }
            result.items
        }
    };

    for item in items.iter().take(20) {
        println!(
            "{:<8} {:>10}  {}\n         {}",
            TorrentSrc::from_code(item.src).map_or("?", |s| s.title()),
            item.size.as_deref().unwrap_or("-"),
            item.title.as_deref().unwrap_or("-"),
            item.magnet_url.as_deref().or(item.detail_url.as_deref()).unwrap_or("-"),
        );
    }
    println!("共 {} 条", items.len());
    Ok(())
}
