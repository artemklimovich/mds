mod admit;
pub mod discover;
mod hygiene;
mod kind;
mod markdown;
mod signals;
pub mod spa;

pub use admit::{check_site, CheckLine, SiteCheck};
pub use hygiene::{is_broken_url, is_not_found, resolve_href, strip_banner_nodes, strip_boilerplate};

pub use discover::{discover_ct_domains, parse_crt_json};

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use thiserror::Error;
use url::Url;

use markdown::{is_llms_card, is_markdown_document, llms_card_blurb, page_from_markdown};
use signals::{
    anchor_nofollow, is_anchor_href, is_sitemap_document, read_page, robots_crawl_delay, robots_sitemaps,
    same_host_canonical, sitemap_locs, x_robots_tag,
};
use spa::{parse_js_bundle, spa_script_urls};

pub const USER_AGENT: &str = "MDS-Bot/0.1 (polite crawler; local index)";

#[derive(Debug, Error)]
pub enum CrawlError {
    #[error("адрес не разобран: {0}")]
    BadUrl(String),
    #[error("сеть: {0}")]
    Http(#[from] reqwest::Error),
}

#[derive(Clone, Debug)]
pub struct FetchedPage {
    pub url: String,
    pub host: String,
    pub title: String,
    pub text: String,
    pub simhash: u64,
    pub links: Vec<String>,
    pub indexable: bool,
    pub kind: u32,
    pub media_url: Option<String>,
    pub feed_url: Option<String>,
    pub price: Option<String>,
    pub docs: Vec<FetchedPage>,
    /// До 100 адресов своего сайта из llms.txt или llms.md. Пусто, если карточка их не назвала.
    pub priority: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CrawlPolicy {
    pub delay: Duration,
    pub max_bytes: usize,
    pub max_pages: usize,
    pub follow_external: bool,
}

impl Default for CrawlPolicy {
    fn default() -> Self {
        Self {
            delay: Duration::from_secs(1),
            max_bytes: 1_000_000,
            max_pages: 30,
            follow_external: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Crawler {
    client: reqwest::Client,
    policy: CrawlPolicy,
    robots: HashMap<String, String>,
    last_hit: HashMap<String, Instant>,
    crawl_delay: HashMap<String, Duration>,
    sitemaps_seen: HashSet<String>,
    site_cards: HashMap<String, String>,
    spa_bundles: HashMap<String, spa::SpaBundleExtract>,
    gone: Vec<String>,
}

impl Crawler {
    pub fn new(policy: CrawlPolicy) -> Result<Self, CrawlError> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;
        Ok(Self {
            client,
            policy,
            robots: HashMap::new(),
            last_hit: HashMap::new(),
            crawl_delay: HashMap::new(),
            sitemaps_seen: HashSet::new(),
            site_cards: HashMap::new(),
            spa_bundles: HashMap::new(),
            gone: Vec::new(),
        })
    }

    pub fn take_gone(&mut self) -> Vec<String> {
        std::mem::take(&mut self.gone)
    }

    fn note_gone(&mut self, url: &str) {
        if !url.is_empty() && !self.gone.iter().any(|known| known == url) {
            self.gone.push(url.to_string());
        }
    }

    pub async fn crawl(&mut self, seeds: &[String]) -> Result<Vec<FetchedPage>, CrawlError> {
        self.gone.clear();
        let mut queue: VecDeque<String> = seeds.iter().cloned().collect();
        let mut seen = HashSet::new();
        let mut pages = Vec::new();
        let mut fetched = 0_usize;
        while let Some(url) = queue.pop_front() {
            let card_request = is_llms_request(&url);
            if (fetched >= self.policy.max_pages && !card_request) || !seen.insert(url.clone()) {
                continue;
            }
            let Some(parsed) = Url::parse(&url).ok() else {
                self.note_gone(&url);
                continue;
            };
            if is_broken_url(&url) {
                self.note_gone(&url);
                continue;
            }
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                self.note_gone(&url);
                continue;
            }
            let origin = origin_of(&parsed);
            let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
            self.ensure_robots(&origin, &mut queue).await;
            let path = format!("{}{}", parsed.path(), parsed.query().map(|q| format!("?{q}")).unwrap_or_default());
            if !path_allowed(self.robots.get(&origin).map(String::as_str).unwrap_or(""), USER_AGENT, &path)
            {
                continue;
            }
            self.wait_turn(&origin, &host).await;
            let response = match self.client.get(parsed.clone()).send().await {
                Ok(response) => response,
                Err(_) => continue,
            };
            if !response.status().is_success() {
                if response.status().as_u16() == 404 || response.status().as_u16() == 410 {
                    self.note_gone(&url);
                    self.note_gone(response.url().as_str());
                }
                if parsed.path().eq_ignore_ascii_case("/llms.txt") {
                    self.enqueue_locs(&mut queue, &[format!("{origin}/llms.md")]);
                }
                continue;
            }
            let final_url = response.url().clone();
            let (header_noindex, header_nofollow) = response
                .headers()
                .get("x-robots-tag")
                .and_then(|value| value.to_str().ok())
                .map(x_robots_tag)
                .unwrap_or((false, false));
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            let bytes = response.bytes().await?;
            if bytes.len() > self.policy.max_bytes {
                continue;
            }
            let body = String::from_utf8_lossy(&bytes).into_owned();
            if is_sitemap_document(&content_type, final_url.as_str(), &body) {
                self.enqueue_locs(&mut queue, &sitemap_locs(&body));
                continue;
            }
            if is_markdown_document(&final_url, &content_type, &body) {
                fetched += 1;
                let host = final_url.host_str().unwrap_or(&host).to_ascii_lowercase();
                if is_llms_card(&final_url) {
                    let blurb = llms_card_blurb(&body);
                    if !blurb.is_empty() {
                        self.site_cards.insert(host.clone(), blurb);
                    }
                }
                let page = page_from_markdown(final_url.as_str(), &host, &body);
                if !header_nofollow {
                    self.enqueue_links(&mut queue, &host, &page.links);
                }
                if page.indexable && !header_noindex && !is_not_found(&page.title, &page.text) && !is_broken_url(&page.url)
                {
                    push_page(&mut pages, page);
                } else {
                    self.note_gone(&url);
                    self.note_gone(&page.url);
                }
                continue;
            }
            if is_llms_card(&final_url) {
                continue;
            }
            if !content_type.is_empty()
                && !content_type.contains("html")
                && !content_type.contains("text/plain")
            {
                continue;
            }
            if !content_type.contains("html") && !looks_like_html(&body) && !content_type.contains("text/plain")
            {
                continue;
            }
            fetched += 1;
            let host = final_url.host_str().unwrap_or(&host).to_ascii_lowercase();
            let mut page = page_from_html(&response_url(&final_url, &body), &host, &body);
            if page.text.chars().count() < 160 {
                self.enrich_from_spa_bundles(final_url.as_str(), &body, &mut page).await;
                page.text = strip_boilerplate(&page.text);
            }
            if !header_nofollow {
                self.enqueue_links(&mut queue, &host, &page.links);
            }
            if page.indexable && !header_noindex && !is_not_found(&page.title, &page.text) && !is_broken_url(&page.url)
            {
                push_page(&mut pages, page);
            } else {
                self.note_gone(&url);
                self.note_gone(&page.url);
            }
        }
        self.attach_site_cards(&mut pages);
        Ok(pages)
    }

    async fn enrich_from_spa_bundles(&mut self, page_url: &str, html: &str, page: &mut FetchedPage) {
        let scripts = spa_script_urls(page_url, html);
        for script_url in scripts {
            let extracted = if let Some(cached) = self.spa_bundles.get(&script_url) {
                cached.clone()
            } else {
                let Ok(response) = self.client.get(&script_url).send().await else {
                    continue;
                };
                if !response.status().is_success() {
                    continue;
                }
                let Ok(bytes) = response.bytes().await else {
                    continue;
                };
                if bytes.len() > 2_000_000 {
                    continue;
                }
                let js = String::from_utf8_lossy(&bytes);
                let parsed = parse_js_bundle(page_url, &js);
                self.spa_bundles.insert(script_url.clone(), parsed.clone());
                parsed
            };
            if !extracted.text.is_empty() {
                if !page.text.is_empty() {
                    page.text.push(' ');
                }
                page.text.push_str(&extracted.text);
                if page.title.is_empty() {
                    page.title = page.text.chars().take(80).collect();
                }
                page.simhash = simhash(&format!("{} {}", page.url, page.text));
            }
            for route in extracted.routes {
                if !page.links.contains(&route) {
                    page.links.push(route);
                }
            }
        }
    }

    fn attach_site_cards(&self, pages: &mut [FetchedPage]) {
        for page in pages {
            if is_card_url(&page.url) {
                continue;
            }
            let Some(blurb) = self.site_cards.get(&page.host) else {
                continue;
            };
            if blurb.is_empty() || page.text.contains(blurb.as_str()) {
                continue;
            }
            page.text.push(' ');
            page.text.push_str(blurb);
        }
    }

    fn enqueue_links(&self, queue: &mut VecDeque<String>, host: &str, links: &[String]) {
        for link in links {
            if queue.len() >= self.policy.max_pages.saturating_mul(4) {
                break;
            }
            if is_broken_url(link) {
                continue;
            }
            if let Ok(linked) = Url::parse(link) {
                if linked.path().to_ascii_lowercase().ends_with(".mdx") {
                    continue;
                }
                let same = linked.host_str().unwrap_or("").eq_ignore_ascii_case(host);
                if same || self.policy.follow_external {
                    queue.push_back(linked.to_string());
                }
            }
        }
    }

    fn enqueue_locs(&self, queue: &mut VecDeque<String>, locs: &[String]) {
        for loc in locs {
            if queue.len() >= self.policy.max_pages.saturating_mul(4) {
                break;
            }
            queue.push_back(loc.clone());
        }
    }

    async fn ensure_robots(&mut self, origin: &str, queue: &mut VecDeque<String>) {
        if self.robots.contains_key(origin) {
            return;
        }
        let url = format!("{origin}/robots.txt");
        let text = match self.client.get(&url).send().await {
            Ok(response) if response.status().is_success() => response.text().await.unwrap_or_default(),
            _ => String::new(),
        };
        if let Some(seconds) = robots_crawl_delay(&text, USER_AGENT) {
            self.crawl_delay.insert(origin.to_string(), Duration::from_secs(seconds));
        }
        if self.sitemaps_seen.insert(origin.to_string()) {
            self.enqueue_locs(queue, &robots_sitemaps(&text));
            queue.push_back(format!("{origin}/llms.txt"));
        }
        self.robots.insert(origin.to_string(), text);
    }

    async fn wait_turn(&mut self, origin: &str, host: &str) {
        let mut delay = if is_loopback(host) {
            Duration::ZERO
        } else {
            self.policy.delay
        };
        if let Some(extra) = self.crawl_delay.get(origin) {
            if *extra > delay {
                delay = *extra;
            }
        }
        if delay.is_zero() {
            self.last_hit.insert(host.to_string(), Instant::now());
            return;
        }
        if let Some(last) = self.last_hit.get(host) {
            let elapsed = last.elapsed();
            if elapsed < delay {
                tokio::time::sleep(delay - elapsed).await;
            }
        }
        self.last_hit.insert(host.to_string(), Instant::now());
    }
}

fn response_url(requested: &Url, _body: &str) -> String {
    requested.to_string()
}

fn origin_of(url: &Url) -> String {
    format!("{}://{}", url.scheme(), url.host_str().unwrap_or(""))
        + &url
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default()
}

fn is_card_url(url: &str) -> bool {
    Url::parse(url).map(|url| is_llms_card(&url)).unwrap_or(false)
}

fn is_llms_request(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("/llms.txt") || lower.contains("/llms.md")
}

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host == "127.0.0.1" || host == "::1"
}

pub fn path_allowed(robots: &str, user_agent: &str, path: &str) -> bool {
    let rules = active_rules(robots, user_agent);
    let path = if path.is_empty() { "/" } else { path };
    let mut allowed = true;
    let mut best = 0_usize;
    for (disallow, prefix) in rules {
        if prefix.len() >= best && path.starts_with(prefix.as_str()) {
            best = prefix.len();
            allowed = !disallow;
        }
    }
    allowed
}

fn active_rules(robots: &str, user_agent: &str) -> Vec<(bool, String)> {
    let mut current_matches = false;
    let mut saw_specific = false;
    let mut specific = Vec::new();
    let mut wildcard = Vec::new();
    let mut any_group = false;
    for line in robots.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key.eq_ignore_ascii_case("user-agent") {
            any_group = true;
            current_matches = value == "*" || user_agent.to_ascii_lowercase().starts_with(&value.to_ascii_lowercase());
            if current_matches && value != "*" {
                saw_specific = true;
            }
            continue;
        }
        if !current_matches {
            continue;
        }
        if key.eq_ignore_ascii_case("disallow") {
            let bucket = if saw_specific && !value.is_empty() {
                &mut specific
            } else if !saw_specific {
                &mut wildcard
            } else {
                continue;
            };
            if !value.is_empty() {
                bucket.push((true, value.to_string()));
            }
        } else if key.eq_ignore_ascii_case("allow") && !value.is_empty() {
            let bucket = if saw_specific { &mut specific } else { &mut wildcard };
            bucket.push((false, value.to_string()));
        }
    }
    if !any_group {
        return Vec::new();
    }
    if saw_specific {
        specific
    } else {
        wildcard
    }
}

pub fn page_from_html(url: &str, host: &str, html: &str) -> FetchedPage {
    let read = read_page(html);
    let title = {
        let from_tag = extract_title(html);
        if !from_tag.is_empty() {
            from_tag
        } else {
            read.title_hint.clone()
        }
    };
    let mut text = extract_text(&strip_banner_nodes(html));
    if !read.extra_text.is_empty() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&read.extra_text);
    }
    text = strip_boilerplate(&text);
    let mut links = if read.follow {
        extract_links(url, html)
    } else {
        Vec::new()
    };
    if read.follow {
        if let Ok(base) = Url::parse(url) {
            for route in &read.spa_routes {
                if let Some(joined) = resolve_href(&base, route) {
                    let full = joined.to_string();
                    if !links.contains(&full) {
                        links.push(full);
                    }
                }
            }
        }
    }
    let stored_url = same_host_canonical(url, host, read.canonical.as_deref());
    let indexable = read.indexable && !is_not_found(&title, &text) && !is_broken_url(&stored_url);
    let title = if title.is_empty() {
        text.chars().take(80).collect()
    } else {
        title
    };
    let classified = kind::classify_html(&stored_url, host, &title, &text, html);
    kind::drop_shop_products(&mut links, mds_types::is_webshop(&title, &text));
    links.retain(|url| !mds_types::is_document_url(url));
    FetchedPage {
        url: stored_url,
        host: host.to_string(),
        title,
        simhash: simhash(&text),
        text,
        links,
        indexable,
        kind: classified.kind,
        media_url: classified.media_url,
        feed_url: classified.feed_url,
        price: classified.price,
        docs: classified.docs,
        priority: Vec::new(),
    }
}

fn push_page(pages: &mut Vec<FetchedPage>, mut page: FetchedPage) {
    let docs = std::mem::take(&mut page.docs);
    pages.push(page);
    pages.extend(docs);
}

fn extract_title(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<title") else {
        return String::new();
    };
    let Some(open) = lower[start..].find('>') else {
        return String::new();
    };
    let from = start + open + 1;
    let Some(end) = lower[from..].find("</title>") else {
        return String::new();
    };
    decode_basic(html[from..from + end].trim())
}

fn extract_text(html: &str) -> String {
    let without = strip_blocks(html, "script");
    let without = strip_blocks(&without, "style");
    let without = strip_blocks(&without, "noscript");
    let without = strip_blocks(&without, "svg");
    let trimmed = strip_blocks(&without, "nav");
    let trimmed = strip_blocks(&trimmed, "header");
    let trimmed = strip_blocks(&trimmed, "footer");
    let trimmed = strip_blocks(&trimmed, "aside");
    let cleaned = plain_from_html(&trimmed);
    let cleaned = if cleaned.chars().count() >= 40 {
        cleaned
    } else {
        plain_from_html(&without)
    };
    decode_basic(&collapse_ws(&cleaned))
}

fn plain_from_html(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
            out.push(' ');
            continue;
        }
        if ch == '>' {
            in_tag = false;
            continue;
        }
        if !in_tag {
            out.push(ch);
        }
    }
    out
}

fn strip_blocks(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::new();
    let mut cursor = 0;
    while let Some(rel) = lower[cursor..].find(&open) {
        let start = cursor + rel;
        out.push_str(&html[cursor..start]);
        let Some(end_rel) = lower[start..].find(&close) else {
            cursor = html.len();
            break;
        };
        cursor = start + end_rel + close.len();
    }
    if cursor < html.len() {
        out.push_str(&html[cursor..]);
    }
    out
}

fn extract_links(base: &str, html: &str) -> Vec<String> {
    let Ok(base) = Url::parse(base) else {
        return Vec::new();
    };
    let lower = html.to_ascii_lowercase();
    let mut links = Vec::new();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("href=") {
        let start = from + rel + 5;
        let bytes = html[start..].as_bytes();
        if bytes.is_empty() {
            break;
        }
        let quote = bytes[0] as char;
        if quote != '"' && quote != '\'' {
            from = start;
            continue;
        }
        let rest = &html[start + 1..];
        let Some(end) = rest.find(quote) else {
            break;
        };
        let raw = &rest[..end];
        let href_at = start;
        from = start + 1 + end;
        if !is_anchor_href(html, href_at) || anchor_nofollow(html, href_at) {
            continue;
        }
        if let Some(joined) = resolve_href(&base, raw) {
            links.push(joined.to_string());
        }
    }
    links
}

fn looks_like_html(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("<html") || lower.contains("<body") || lower.contains("<p") || lower.contains("<title")
}

fn collapse_ws(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(ch);
    }
    out
}

fn decode_basic(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
}

pub fn simhash(text: &str) -> u64 {
    let mut weights = [0_i32; 64];
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        if token.chars().count() < 2 {
            continue;
        }
        let hash = blake3::hash(token.to_lowercase().as_bytes());
        let mut bits = 0_u64;
        for byte in hash.as_bytes().iter().take(8) {
            bits = (bits << 8) | *byte as u64;
        }
        for i in 0..64 {
            if (bits >> i) & 1 == 1 {
                weights[i] += 1;
            } else {
                weights[i] -= 1;
            }
        }
    }
    let mut out = 0_u64;
    for (i, weight) in weights.iter().enumerate() {
        if *weight >= 0 {
            out |= 1 << i;
        }
    }
    out
}

pub fn hamming(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}

pub fn is_near_duplicate(left: u64, right: u64) -> bool {
    hamming(left, right) <= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_prefix_blocks_secret() {
        let robots = "User-agent: *\nDisallow: /secret\nAllow: /secret-public\n";
        assert!(!path_allowed(robots, USER_AGENT, "/secret.html"));
        assert!(path_allowed(robots, USER_AGENT, "/secret-public"));
        assert!(path_allowed(robots, USER_AGENT, "/vet.html"));
    }

    #[test]
    fn banner_missing_page_and_glued_link_do_not_enter() {
        let html = r#"<html><title>Мёд</title><div class="cookie-notice">Мы используем cookie. Нажмите принять.</div><p>Настоящий мёд с пасеки в Камышине, стеклянная банка три литра.</p><p>Наша форма -- наш webhook CRM.</p><a href="www.thelivecoffee.ru">сайт</a></html>"#;
        let page = page_from_html("https://rusprodsoyuz.ru/participants/", "rusprodsoyuz.ru", html);
        assert!(page.indexable, "{}", page.text);
        assert!(page.text.contains("мёд"), "{}", page.text);
        assert!(!page.text.to_lowercase().contains("cookie"), "{}", page.text);
        assert!(!page.text.to_lowercase().contains("webhook"), "{}", page.text);
        assert!(
            page.links.iter().any(|link| link.contains("://www.thelivecoffee.ru")),
            "{:?}",
            page.links
        );
        assert!(page.links.iter().all(|link| !link.contains("rusprodsoyuz.ru/www")));
        let missing = page_from_html(
            "https://tehnoveles.ru/project/x",
            "tehnoveles.ru",
            "<html><title>Проект не найден</title><p>Проект не найден</p></html>",
        );
        assert!(!missing.indexable);
    }

    #[test]
    fn extract_skips_script() {
        let html = "<html><title>Крепёж</title><script>SECRETTOKEN99</script><p>Каркас MDSBOLT42</p></html>";
        let page = page_from_html("https://shop.example/bolt", "shop.example", html);
        assert_eq!(page.title, "Крепёж");
        assert!(page.text.contains("MDSBOLT42"));
        assert!(!page.text.contains("SECRETTOKEN99"));
    }

    #[test]
    fn duplicate_text_is_close() {
        let left = simhash("Ветеринарная помощь питомцам в городской лечебнице");
        let right = simhash("Ветеринарная помощь питомцам в городской лечебнице");
        let other = simhash("Рецепт яблочного пирога с корицей и тестом");
        assert!(is_near_duplicate(left, right));
        assert!(!is_near_duplicate(left, other));
    }

    #[tokio::test]
    async fn crawler_skips_disallowed_page() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0_u8; 1024];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                let request = String::from_utf8_lossy(&buf);
                let (status, body, kind) = if request.contains("GET /robots.txt") {
                    ("200 OK", "User-agent: *\nDisallow: /secret.html\n", "text/plain")
                } else if request.contains("GET /secret.html") {
                    ("200 OK", "<html><title>Скрыто</title><p>SECRETTOKEN99</p></html>", "text/html")
                } else {
                    ("200 OK", "<html><title>Открыто</title><p>MDSBOLT42</p></html>", "text/html")
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
            }
        });
        let mut crawler = Crawler::new(CrawlPolicy {
            delay: Duration::from_millis(0),
            max_bytes: 100_000,
            max_pages: 5,
            follow_external: false,
        })
        .unwrap();
        let pages = crawler
            .crawl(&[
                format!("http://{addr}/bolt.html"),
                format!("http://{addr}/secret.html"),
            ])
            .await
            .unwrap();
        server.abort();
        assert!(pages.iter().any(|page| page.text.contains("MDSBOLT42")));
        assert!(pages.iter().all(|page| !page.text.contains("SECRETTOKEN99")));
    }

    #[test]
    fn seo_markup_enters_the_page_text() {
        let html = r#"<html><head>
            <title>Магазин</title>
            <meta name="description" content="Коротко про лавку">
            <meta property="og:description" content="OGAPPLE9 яблоки">
            <meta name="robots" content="index, nofollow">
            <link rel="canonical" href="https://shop.example/store">
            <script type="application/ld+json">{"@type":"Product","name":"Болт","description":"BOLTJSON7"}</script>
            </head><body><span itemprop="name">МИКРОИМЯ</span><a href="/secret">секрет</a><p>текст</p></body></html>"#;
        let page = page_from_html("https://shop.example/old", "shop.example", html);
        assert!(page.text.contains("BOLTJSON7"), "{}", page.text);
        assert!(page.text.contains("OGAPPLE9"), "{}", page.text);
        assert!(page.text.contains("МИКРОИМЯ"), "{}", page.text);
        assert_eq!(page.url, "https://shop.example/store");
        assert!(page.links.is_empty());
        assert!(page.indexable);
    }

    #[test]
    fn noindex_is_not_stored_and_nofollow_link_is_dropped() {
        let hidden = r#"<html><meta name="robots" content="noindex"><p>HIDDENROBOTS</p></html>"#;
        let hidden = page_from_html("https://shop.example/h", "shop.example", hidden);
        assert!(!hidden.indexable);
        let links = r#"<link rel="profile" href="https://gmpg.org/xfn/11"><a href="https://shop.example/skip" rel="nofollow">x</a><a href="/ok">y</a>"#;
        let page = page_from_html("https://shop.example/", "shop.example", links);
        assert!(page.links.iter().any(|link| link.contains("/ok")));
        assert!(page.links.iter().all(|link| !link.contains("skip") && !link.contains("gmpg")));
    }

    #[test]
    fn robots_lists_sitemap_and_crawl_delay() {
        let robots = "User-agent: *\nCrawl-delay: 2\nSitemap: https://shop.example/sitemap.xml\n";
        assert_eq!(signals::x_robots_tag("noindex, nofollow"), (true, true));
        assert_eq!(signals::robots_crawl_delay(robots, USER_AGENT), Some(2));
        assert_eq!(
            signals::robots_sitemaps(robots),
            vec!["https://shop.example/sitemap.xml".to_string()]
        );
        let xml = "<urlset><url><loc>https://shop.example/open</loc></url></urlset>";
        assert_eq!(signals::sitemap_locs(xml), vec!["https://shop.example/open".to_string()]);
    }

    #[tokio::test]
    async fn sitemap_page_is_fetched_and_noindex_is_skipped() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0_u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                let request = String::from_utf8_lossy(&buf);
                let (kind, body) = if request.contains("GET /robots.txt") {
                    (
                        "text/plain",
                        format!("User-agent: *\nSitemap: http://{addr}/sitemap.xml\n"),
                    )
                } else if request.contains("GET /sitemap.xml") {
                    (
                        "application/xml",
                        format!("<urlset><url><loc>http://{addr}/open.html</loc></url><url><loc>http://{addr}/hidden.html</loc></url></urlset>"),
                    )
                } else if request.contains("GET /hidden.html") {
                    (
                        "text/html",
                        "<html><meta name=\"robots\" content=\"noindex\"><p>HIDDENROBOTS</p></html>".to_string(),
                    )
                } else if request.contains("GET /open.html") {
                    ("text/html", "<html><title>Открыто</title><p>SITEMARKER</p></html>".to_string())
                } else {
                    ("text/html", "<html><title>Старт</title><p>корень</p></html>".to_string())
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
            }
        });
        let mut crawler = Crawler::new(CrawlPolicy {
            delay: Duration::from_millis(0),
            max_bytes: 100_000,
            max_pages: 6,
            follow_external: false,
        })
        .unwrap();
        let pages = crawler.crawl(&[format!("http://{addr}/")]).await.unwrap();
        server.abort();
        assert!(pages.iter().any(|page| page.text.contains("SITEMARKER")), "{pages:?}");
        assert!(pages.iter().all(|page| !page.text.contains("HIDDENROBOTS")));
    }

    #[test]
    fn llms_card_keeps_markdown_links_and_drops_mdx() {
        let markdown = "# Ферма\n> Молоко FARMWORD\n\n- [Молоко](/milk.md)\n- [Код](secret.mdx)\n";
        let page = markdown::page_from_markdown("https://shop.example/llms.txt", "shop.example", markdown);
        assert!(page.text.contains("FARMWORD"), "{}", page.text);
        assert!(page.links.iter().any(|link| link.ends_with("/milk.md")), "{:?}", page.links);
        assert!(page.links.iter().all(|link| !link.contains("mdx")));
        assert!(markdown::llms_card_blurb(markdown).contains("FARMWORD"));
        let url = Url::parse("https://shop.example/llms.md").unwrap();
        assert!(!markdown::is_markdown_document(&url, "text/html", "<!doctype html><html><body>SPA</body></html>"));
    }

    #[tokio::test]
    async fn llms_card_is_indexed_and_added_to_the_site_page() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0_u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                let request = String::from_utf8_lossy(&buf);
                let (status, kind, body) = if request.contains("GET /robots.txt") {
                    ("200 OK", "text/plain", "User-agent: *\nAllow: /\n".to_string())
                } else if request.contains("GET /llms.txt") {
                    (
                        "200 OK",
                        "text/plain",
                        "# Ферма\n> Молочная ферма FARMWORD\n\n- [Молоко](/milk.md)\n- [Код](/secret.mdx)\n".to_string(),
                    )
                } else if request.contains("GET /milk.md") {
                    ("200 OK", "text/markdown", "# Молоко\n\nСвежее MILKPAGE\n".to_string())
                } else if request.contains("GET /llms.md") || request.contains("GET /secret.mdx") {
                    ("404 Not Found", "text/plain", String::new())
                } else {
                    ("200 OK", "text/html", "<html><title>Каталог</title><p>список товаров</p></html>".to_string())
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
            }
        });
        let mut crawler = Crawler::new(CrawlPolicy {
            delay: Duration::from_millis(0),
            max_bytes: 100_000,
            max_pages: 6,
            follow_external: false,
        })
        .unwrap();
        let pages = crawler.crawl(&[format!("http://{addr}/")]).await.unwrap();
        server.abort();
        assert!(pages.iter().any(|page| page.url.contains("llms.txt") && page.text.contains("FARMWORD")), "{pages:?}");
        assert!(pages.iter().any(|page| page.url.contains("milk.md") && page.text.contains("MILKPAGE")), "{pages:?}");
        assert!(pages.iter().any(|page| page.url.ends_with('/') && page.text.contains("FARMWORD")), "{pages:?}");
        assert!(pages.iter().all(|page| !page.url.contains("mdx")));
    }

    #[tokio::test]
    async fn csr_spa_extracts_text_and_routes_from_js_bundle_and_next_data() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0_u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                let request = String::from_utf8_lossy(&buf);
                let (status, kind, body) = if request.contains("GET /robots.txt") || request.contains("GET /llms") {
                    ("404 Not Found", "text/plain", String::new())
                } else if request.contains("GET /assets/app.js") {
                    (
                        "200 OK",
                        "application/javascript",
                        r#"const routes = ["/pricing", "/assets/logo.png"]; const hero = "Разработка современных SPA приложений и децентрализованный поиск SPABUNDLETOKEN";"#
                            .to_string(),
                    )
                } else if request.contains("GET /pricing") {
                    (
                        "200 OK",
                        "text/html",
                        r#"<html><head><title>Тарифы</title><script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"summary":"Тарифный план для студий и агентств NEXTDATATOKEN"}}}</script></head><body><div id="root"></div></body></html>"#
                            .to_string(),
                    )
                } else {
                    (
                        "200 OK",
                        "text/html",
                        r#"<html><head><title>SPA</title><script type="module" src="/assets/app.js"></script></head><body><div id="root"></div></body></html>"#
                            .to_string(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
            }
        });
        let mut crawler = Crawler::new(CrawlPolicy {
            delay: Duration::from_millis(0),
            max_bytes: 100_000,
            max_pages: 6,
            follow_external: false,
        })
        .unwrap();
        let pages = crawler.crawl(&[format!("http://{addr}/")]).await.unwrap();
        server.abort();
        assert!(pages.iter().any(|p| p.text.contains("SPABUNDLETOKEN")), "{pages:?}");
        assert!(pages.iter().any(|p| p.url.ends_with("/pricing") && p.text.contains("NEXTDATATOKEN")), "{pages:?}");
    }

    #[test]
    fn crt_json_parses_clean_domains_and_skips_wildcards_and_mail() {
        let raw = br#"[{"name_value":"*.dartalab.ru\ndartalab.ru\nmail.dartalab.ru\nshop.example.ru"}]"#;
        let hosts = parse_crt_json(raw);
        assert_eq!(hosts, vec!["dartalab.ru".to_string(), "shop.example.ru".to_string()]);
    }
}
