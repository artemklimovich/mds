use std::time::Duration;

use mds_crawl::{CrawlPolicy, Crawler};
use mds_dht::host_of;
use mds_wire::{PeerHello, PushedAnchor, PushedPage};

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
    let hive = config.hive.trim().trim_end_matches('/').to_string();
    if !hive.starts_with("http://") && !hive.starts_with("https://") {
        return Err(NodeError::Message("улью нужен адрес http".into()));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(mds_crawl::USER_AGENT)
        .build()
        .map_err(|err| NodeError::Message(err.to_string()))?;
    let Some(host) = site_host(&config.site) else {
        eprintln!("MDS: пчела без своего сайта, улей {hive}. Модель и база не загружаются.");
        eprintln!("MDS: она разбирает чужие адреса из очереди. Темп задаётся в кабинете, режим сервера.");
        let mut pause = 60_u64;
        loop {
            flush_outbox(&client, &hive, "x-mds-shell", &shell_id(&config.label), "/shell/pages").await;
            match work_foreign(&client, &config, &hive).await {
                Ok(next) => pause = next.max(60),
                Err(err) => eprintln!("MDS: чужие страницы не отданы: {err}"),
            }
            tokio::time::sleep(Duration::from_secs(pause)).await;
        }
    };
    eprintln!("MDS: лёгкая пчела сайта {host}, улей {hive}. Модель и общий индекс не загружаются.");
    eprintln!("MDS: положите на сайт файл https://{host}/.well-known/pchela.txt");
    eprintln!("MDS: в файле одна строка: {}", config.label);
    eprintln!("MDS: свои страницы уходят, только если они новые или изменились. Иначе пчела берёт чужие адреса.");
    let mut pause = 120_u64;
    loop {
        flush_outbox(&client, &hive, "x-mds-bee", &config.label, "/peer/pages").await;
        match push_once(&client, &config, &hive, &host).await {
            Ok(next) => pause = next.max(60),
            Err(err) => eprintln!("MDS: лёгкая пчела не отдала страницы: {err}"),
        }
        tokio::time::sleep(Duration::from_secs(pause)).await;
    }
}

async fn push_once(
    client: &reqwest::Client,
    config: &LiteConfig,
    hive: &str,
    host: &str,
) -> Result<u64, NodeError> {
    if !join(client, config, hive).await? {
        eprintln!(
            "MDS: улей не подтвердил карточку. Строка {} должна лежать в https://{host}/.well-known/pchela.txt",
            config.label
        );
        return Ok(120);
    }
    let policy = CrawlPolicy {
        delay: Duration::from_millis(config.delay_ms.max(2000)),
        max_bytes: 500_000,
        max_pages: 40,
        follow_external: false,
    };
    let mut crawler = Crawler::new(policy)?;
    let pages = crawler.crawl(&[config.site.clone()]).await?;
    let own: Vec<PushedPage> = pages
        .into_iter()
        .filter(|page| {
            page.indexable
                && host_of(&page.url).as_deref() == Some(host)
                && !page.text.trim().is_empty()
        })
        .take(40)
        .map(|page| pushed_from(page, 80_000))
        .collect();
    let own = keep_changed(client, config, hive, own).await;
    if !own.is_empty() {
        if let Err(err) = send_pages(client, config, hive, &own).await {
            stash_outbox(&own);
            eprintln!("MDS: свои страницы лежат в ящике, улей не принял: {err}");
            return Ok(120);
        }
        eprintln!("MDS: улью отданы свои страницы: {}", own.len());
        return Ok(120);
    }
    if let Some((foreign, pause)) = fetch_work(client, config, hive).await {
        if let Err(err) = send_pages(client, config, hive, &foreign).await {
            stash_outbox(&foreign);
            eprintln!("MDS: чужие страницы лежат в ящике, улей не принял: {err}");
            return Ok(pause.max(60));
        }
        eprintln!("MDS: улью отданы чужие страницы: {}", foreign.len());
        return Ok(pause.max(60));
    }
    eprintln!("MDS: своих изменений нет, чужих адресов сейчас нет");
    return Ok(120);
}

async fn send_pages(
    client: &reqwest::Client,
    config: &LiteConfig,
    hive: &str,
    pushed: &[PushedPage],
) -> Result<(), NodeError> {
    if pushed.is_empty() {
        return Ok(());
    }
    let body = serde_json::to_vec(&pushed).map_err(|err| NodeError::Message(err.to_string()))?;
    let response = client
        .post(format!("{hive}/peer/pages"))
        .header("x-mds-bee", &config.label)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?;
    if !response.status().is_success() {
        return Err(NodeError::Message("улей не принял страницы".into()));
    }
    Ok(())
}

fn outbox_path() -> std::path::PathBuf {
    std::path::PathBuf::from("data/pchela-outbox.json")
}

fn stash_outbox(pages: &[PushedPage]) {
    if pages.is_empty() {
        return;
    }
    let mut have = load_outbox();
    have.extend(pages.iter().cloned());
    if have.len() > 40 {
        let skip = have.len() - 40;
        have.drain(0..skip);
    }
    if let Some(parent) = outbox_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_vec(&have) {
        let _ = std::fs::write(outbox_path(), body);
    }
}

fn load_outbox() -> Vec<PushedPage> {
    let Ok(body) = std::fs::read(outbox_path()) else {
        return Vec::new();
    };
    serde_json::from_slice(&body).unwrap_or_default()
}

async fn flush_outbox(
    client: &reqwest::Client,
    hive: &str,
    header: &str,
    value: &str,
    path: &str,
) {
    let pages = load_outbox();
    if pages.is_empty() {
        return;
    }
    let Ok(body) = serde_json::to_vec(&pages) else {
        return;
    };
    let response = client
        .post(format!("{hive}{path}"))
        .header(header, value)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await;
    if response.is_ok_and(|item| item.status().is_success()) {
        let _ = std::fs::remove_file(outbox_path());
        eprintln!("MDS: ящик отдан улью, страниц {}", pages.len());
    }
}

#[derive(serde::Serialize)]
struct FreshItem {
    url: String,
    simhash: u64,
}

#[derive(serde::Serialize)]
struct FreshAsk {
    pages: Vec<FreshItem>,
}

#[derive(serde::Deserialize)]
struct FreshAnswer {
    want: Vec<String>,
}

async fn keep_changed(
    client: &reqwest::Client,
    config: &LiteConfig,
    hive: &str,
    pages: Vec<PushedPage>,
) -> Vec<PushedPage> {
    if pages.is_empty() {
        return pages;
    }
    let ask = FreshAsk {
        pages: pages
            .iter()
            .map(|page| FreshItem {
                url: page.url.clone(),
                simhash: page.simhash,
            })
            .collect(),
    };
    let Ok(body) = serde_json::to_vec(&ask) else {
        return pages.into_iter().take(8).collect();
    };
    let response = client
        .post(format!("{hive}/peer/fresh"))
        .header("x-mds-bee", &config.label)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await;
    let Ok(response) = response else {
        return pages.into_iter().take(8).collect();
    };
    if !response.status().is_success() {
        return pages.into_iter().take(8).collect();
    }
    let Ok(bytes) = response.bytes().await else {
        return pages.into_iter().take(8).collect();
    };
    let Ok(answer) = serde_json::from_slice::<FreshAnswer>(&bytes) else {
        return pages.into_iter().take(8).collect();
    };
    pages
        .into_iter()
        .filter(|page| answer.want.iter().any(|url| url == &page.url))
        .collect()
}

async fn work_foreign(client: &reqwest::Client, config: &LiteConfig, hive: &str) -> Result<u64, NodeError> {
    let id = shell_id(&config.label);
    let response = client
        .get(format!("{hive}/shell/work"))
        .header("x-mds-shell", &id)
        .header("x-mds-wish", "server")
        .send()
        .await
        .map_err(|err| NodeError::Message(err.to_string()))?;
    if !response.status().is_success() {
        return Err(NodeError::Message("улей не дал работу".into()));
    }
    let bytes = response.bytes().await.map_err(|err| NodeError::Message(err.to_string()))?;
    let work: mds_wire::CrawlWork = serde_json::from_slice(&bytes).unwrap_or(mds_wire::CrawlWork {
        urls: Vec::new(),
        pause_secs: 60,
    });
    let pause = work.pause_secs.max(60);
    if work.urls.is_empty() {
        eprintln!("MDS: очередь для пчелы без сайта сейчас пуста");
        return Ok(pause);
    }
    let policy = CrawlPolicy {
        delay: Duration::from_millis(config.delay_ms.max(2000)),
        max_bytes: 500_000,
        max_pages: work.urls.len().clamp(1, 30),
        follow_external: false,
    };
    let mut crawler = Crawler::new(policy)?;
    let pages = crawler.crawl(&work.urls).await?;
    let pushed = pages
        .into_iter()
        .filter(|page| page.indexable && page.text.trim().chars().count() >= 40)
        .map(|page| pushed_from(page, 20_000))
        .collect::<Vec<_>>();
    if pushed.is_empty() {
        eprintln!("MDS: выданные адреса не открылись");
        return Ok(pause);
    }
    let body = serde_json::to_vec(&pushed).map_err(|err| NodeError::Message(err.to_string()))?;
    let sent = client
        .post(format!("{hive}/shell/pages"))
        .header("x-mds-shell", &id)
        .header("x-mds-wish", "server")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await;
    match sent {
        Ok(response) if response.status().is_success() => {
            eprintln!("MDS: улью отданы чужие страницы: {}", pushed.len());
            Ok(pause)
        }
        Ok(_) => {
            stash_outbox(&pushed);
            eprintln!("MDS: чужие страницы лежат в ящике, улей не принял");
            Ok(pause)
        }
        Err(err) => {
            stash_outbox(&pushed);
            eprintln!("MDS: чужие страницы лежат в ящике: {err}");
            Ok(pause)
        }
    }
}

fn shell_id(label: &str) -> String {
    let label = label.trim();
    if label.len() == 32 && label.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return label.to_string();
    }
    let mut salt = label.to_string();
    let mut hex = String::new();
    while hex.len() < 32 {
        salt.push('x');
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        salt.hash(&mut hasher);
        hex.push_str(&format!("{:016x}", hasher.finish()));
    }
    hex.chars().take(32).collect()
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

async fn fetch_work(client: &reqwest::Client, config: &LiteConfig, hive: &str) -> Option<(Vec<PushedPage>, u64)> {
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
    let pause = work.pause_secs.max(60);
    let policy = CrawlPolicy {
        delay: Duration::from_millis(config.delay_ms.max(2000)),
        max_bytes: 500_000,
        max_pages: work.urls.len().clamp(1, 24),
        follow_external: false,
    };
    let mut crawler = Crawler::new(policy).ok()?;
    let pages = crawler.crawl(&work.urls).await.ok()?;
    let pushed = pages
        .into_iter()
        .filter(|page| page.indexable && !page.text.trim().is_empty())
        .map(|page| pushed_from(page, 80_000))
        .collect::<Vec<_>>();
    if pushed.is_empty() {
        None
    } else {
        Some((pushed, pause))
    }
}

fn pushed_from(page: mds_crawl::FetchedPage, text_limit: usize) -> PushedPage {
    let links = page
        .links
        .iter()
        .filter(|url| same_site(&page.host, url))
        .take(20)
        .cloned()
        .collect();
    let anchors = page
        .anchors
        .into_iter()
        .filter(|anchor| same_site(&page.host, &anchor.url) && anchor.text.trim().chars().count() >= 2)
        .take(20)
        .map(|anchor| PushedAnchor {
            url: anchor.url,
            text: anchor.text,
        })
        .collect();
    PushedPage {
        url: page.url,
        title: page.title,
        host: page.host.clone(),
        text: page.text.chars().take(text_limit).collect(),
        links,
        simhash: page.simhash,
        anchors,
    }
}

fn same_site(page_host: &str, url: &str) -> bool {
    let Some(host) = host_of(url) else {
        return false;
    };
    host.trim_start_matches("www.") == page_host.trim_start_matches("www.")
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
