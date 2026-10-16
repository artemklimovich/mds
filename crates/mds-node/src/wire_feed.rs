use std::sync::Mutex;
use std::time::{Duration, Instant};

use mds_crawl::USER_AGENT;
use mds_types::{press_feeds, rss_headlines};
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct WireLine {
    pub source: String,
    pub title: String,
    pub url: String,
}

struct Cache {
    at: Instant,
    lines: Vec<WireLine>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
const REMEMBER: Duration = Duration::from_secs(600);

/// Свежие заголовки перечисленных изданий. В индекс не пишутся.
pub async fn headlines() -> Vec<WireLine> {
    if let Some(lines) = fresh() {
        return lines;
    }
    let client = client();
    let mut jobs = Vec::new();
    for feed in press_feeds() {
        let client = client.clone();
        let name = feed.name.to_string();
        let url = feed.rss.to_string();
        jobs.push(tokio::spawn(async move { read_feed(&client, &name, &url).await }));
    }
    let mut lines = Vec::new();
    for job in jobs {
        if let Ok(mut batch) = job.await {
            lines.append(&mut batch);
        }
    }
    if lines.is_empty() {
        return stale();
    }
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some(Cache {
            at: Instant::now(),
            lines: lines.clone(),
        });
    }
    lines
}

fn fresh() -> Option<Vec<WireLine>> {
    let guard = CACHE.lock().ok()?;
    let cache = guard.as_ref()?;
    if cache.at.elapsed() < REMEMBER {
        Some(cache.lines.clone())
    } else {
        None
    }
}

fn stale() -> Vec<WireLine> {
    CACHE
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|cache| cache.lines.clone()))
        .unwrap_or_default()
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(USER_AGENT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

async fn read_feed(client: &reqwest::Client, name: &str, url: &str) -> Vec<WireLine> {
    let Ok(response) = client.get(url).send().await else {
        return Vec::new();
    };
    if !response.status().is_success() {
        return Vec::new();
    }
    let Ok(xml) = response.text().await else {
        return Vec::new();
    };
    rss_headlines(&xml, 8)
        .into_iter()
        .map(|(title, url)| WireLine {
            source: name.to_string(),
            title,
            url,
        })
        .collect()
}
