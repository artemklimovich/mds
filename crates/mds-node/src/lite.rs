use std::time::Duration;

use mds_crawl::{CrawlPolicy, Crawler};
use mds_dht::host_of;
use mds_wire::{PeerHello, PushedPage};

use crate::NodeError;

pub struct LiteConfig {
    pub site: String,
    pub hive: String,
    /// Старый общий пароль. Лёгкая пчела его больше не посылает.
    #[allow(dead_code)]
    pub swarm_key: String,
    pub label: String,
    pub delay_ms: u64,
}

pub async fn run_lite(config: LiteConfig) -> Result<(), NodeError> {
    let host = site_host(&config.site).ok_or_else(|| {
        NodeError::Message("укажите адрес сайта, например https://tehnoveles.ru".into())
    })?;
    let hive = config.hive.trim().trim_end_matches('/').to_string();
    if !hive.starts_with("http://") && !hive.starts_with("https://") {
        return Err(NodeError::Message("улью нужен адрес http".into()));
    }
    eprintln!("MDS: лёгкая пчела сайта {host}, улей {hive}. Модель и общий индекс не загружаются.");
    eprintln!("MDS: положите на сайт файл https://{host}/.well-known/pchela.txt");
    eprintln!("MDS: в файле одна строка: {}", config.label);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(mds_crawl::USER_AGENT)
        .build()
        .map_err(|err| NodeError::Message(err.to_string()))?;
    loop {
        if let Err(err) = push_once(&client, &config, &hive, &host).await {
            eprintln!("MDS: лёгкая пчела не отдала страницы: {err}");
        }
        tokio::time::sleep(Duration::from_secs(10 * 60)).await;
    }
}

async fn push_once(
    client: &reqwest::Client,
    config: &LiteConfig,
    hive: &str,
    host: &str,
) -> Result<(), NodeError> {
    if !join(client, config, hive).await? {
        eprintln!(
            "MDS: улей не подтвердил карточку. Строка {} должна лежать в https://{host}/.well-known/pchela.txt",
            config.label
        );
        return Ok(());
    }
    let policy = CrawlPolicy {
        delay: Duration::from_millis(config.delay_ms.max(2000)),
        max_bytes: 500_000,
        max_pages: 6,
        follow_external: false,
    };
    let mut crawler = Crawler::new(policy)?;
    let pages = crawler.crawl(&[config.site.clone()]).await?;
    let mut pushed: Vec<PushedPage> = pages
        .into_iter()
        .filter(|page| {
            page.indexable
                && host_of(&page.url).as_deref() == Some(host)
                && !page.text.trim().is_empty()
        })
        .take(4)
        .map(|page| PushedPage {
            url: page.url,
            title: page.title,
            host: page.host,
            text: page.text.chars().take(80_000).collect(),
        })
        .collect();
    if let Some(foreign) = fetch_work(client, config, hive).await {
        pushed.extend(foreign);
    }
    if pushed.is_empty() {
        eprintln!("MDS: страниц для улья нет");
        return Ok(());
    }
    let body = serde_json::to_vec(&pushed).map_err(|err| NodeError::Message(err.to_string()))?;
    client
        .post(format!("{hive}/peer/pages"))
        .header("x-mds-bee", &config.label)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?;
    eprintln!("MDS: улью отдано страниц: {}", pushed.len());
    Ok(())
}

async fn join(client: &reqwest::Client, config: &LiteConfig, hive: &str) -> Result<bool, NodeError> {
    let hello = PeerHello {
        base: String::new(),
        label: config.label.clone(),
        buckets: Vec::new(),
        holds_all: false,
        pages: 0,
        queue: 0,
        site: config.site.clone(),
    };
    let hello_body = serde_json::to_vec(&hello).map_err(|err| NodeError::Message(err.to_string()))?;
    let response = client
        .post(format!("{hive}/peer/join"))
        .header("x-mds-bee", &config.label)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(hello_body)
        .send()
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?;
    Ok(response.status().is_success())
}

async fn fetch_work(client: &reqwest::Client, config: &LiteConfig, hive: &str) -> Option<Vec<PushedPage>> {
    let response = client
        .get(format!("{hive}/peer/work"))
        .header("x-mds-bee", &config.label)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    let work: mds_wire::CrawlWork = serde_json::from_slice(&bytes).ok()?;
    if work.urls.is_empty() {
        return None;
    }
    let policy = CrawlPolicy {
        delay: Duration::from_millis(config.delay_ms.max(2000)),
        max_bytes: 500_000,
        max_pages: work.urls.len().clamp(1, 4),
        follow_external: false,
    };
    let mut crawler = Crawler::new(policy).ok()?;
    let pages = crawler.crawl(&work.urls).await.ok()?;
    let pushed = pages
        .into_iter()
        .filter(|page| page.indexable && !page.text.trim().is_empty())
        .take(4)
        .map(|page| PushedPage {
            url: page.url,
            title: page.title,
            host: page.host,
            text: page.text.chars().take(80_000).collect(),
        })
        .collect::<Vec<_>>();
    if pushed.is_empty() {
        None
    } else {
        Some(pushed)
    }
}

pub fn site_host(site: &str) -> Option<String> {
    let site = site.trim();
    if site.is_empty() {
        return None;
    }
    if site.contains("://") {
        host_of(site)
    } else {
        host_of(&format!("https://{site}"))
    }
}
