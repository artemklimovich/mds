mod bees;
mod engine;
mod http;
mod lite;
mod model_sync;
mod updates;
mod wire_feed;

pub use engine::Engine;
pub use http::{serve, ServeConfig};
pub use lite::{run_lite, LiteConfig};
pub use model_sync::sync_model;

use std::net::SocketAddr;
use std::path::Path;

use mds_crawl::CrawlPolicy;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error(transparent)]
    Index(#[from] mds_index::IndexError),
    #[error(transparent)]
    Embed(#[from] mds_embed::EmbedError),
    #[error(transparent)]
    Crawl(#[from] mds_crawl::CrawlError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Update(#[from] mds_update::UpdateError),
}

pub async fn index_fixture_dir(
    engine: &mut Engine,
    dir: &Path,
    policy: &CrawlPolicy,
) -> Result<usize, NodeError> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr: SocketAddr = listener.local_addr()?;
    let router = axum::Router::new().fallback_service(tower_http::services::ServeDir::new(dir));
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    wait_until_ready(addr).await;
    let mut seeds = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".html") {
            seeds.push(format!("http://{addr}/{name}"));
        }
    }
    seeds.sort();
    let indexed = engine.index_urls(&seeds, policy).await?;
    server.abort();
    Ok(indexed)
}

async fn wait_until_ready(addr: SocketAddr) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let request = b"GET /robots.txt HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    for _ in 0..40 {
        if let Ok(mut stream) = tokio::net::TcpStream::connect(addr).await {
            if stream.write_all(request).await.is_ok() {
                let mut buf = [0_u8; 32];
                if stream.read(&mut buf).await.ok().filter(|n| *n > 12).is_some()
                    && buf.windows(3).any(|w| w == b"200")
                {
                    return;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
