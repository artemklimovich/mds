use std::collections::{HashMap, HashSet};
use std::time::Duration;

use mds_types::{bare_host, host_of_url};

/// Имя пчелы, которое она печатает и которое лежит в карточке сайта.
pub fn bee_id_ok(id: &str) -> bool {
    let id = id.trim();
    (8..=64).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// В файле карточки имя пчелы стоит отдельной строкой.
pub fn card_names_bee(body: &str, id: &str) -> bool {
    let id = id.trim();
    if !bee_id_ok(id) {
        return false;
    }
    body.lines().any(|line| line.trim() == id)
}

pub fn card_url(host: &str) -> String {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    format!("https://{host}/.well-known/pchela.txt")
}

/// Кто доказал свой сайт и какие чужие адреса улей ему выдал.
#[derive(Default)]
pub struct BeeBook {
    verified: HashMap<String, String>,
    assigned: HashMap<String, Vec<String>>,
    refused: HashSet<String>,
}

impl BeeBook {
    pub fn load_refused(&mut self, hosts: impl IntoIterator<Item = String>) {
        self.refused = hosts.into_iter().map(|host| bare_host(&host)).collect();
    }

    pub fn refuse(&mut self, host: &str) {
        let host = bare_host(host);
        if host.is_empty() {
            return;
        }
        self.refused.insert(host.clone());
        self.verified.retain(|_, known| known != &host);
        self.assigned.clear();
    }

    pub fn restore(&mut self, host: &str) {
        self.refused.remove(&bare_host(host));
    }

    pub fn is_refused(&self, host: &str) -> bool {
        self.refused.contains(&bare_host(host))
    }

    pub fn refused(&self) -> Vec<String> {
        let mut hosts: Vec<String> = self.refused.iter().cloned().collect();
        hosts.sort();
        hosts
    }

    pub fn note_verified(&mut self, id: &str, host: &str) {
        let host = bare_host(host);
        if !bee_id_ok(id) || host.is_empty() || self.refused.contains(&host) {
            return;
        }
        self.verified.insert(id.trim().to_string(), host);
    }

    pub fn host_of(&self, id: &str) -> Option<String> {
        let host = self.verified.get(id.trim())?.clone();
        if self.refused.contains(&host) {
            None
        } else {
            Some(host)
        }
    }

    pub fn assign(&mut self, id: &str, urls: Vec<String>) {
        if self.host_of(id).is_none() {
            return;
        }
        self.assigned.insert(id.trim().to_string(), urls);
    }

    /// Свой хост принимается всегда. Чужой адрес — только если улей сам его выдал.
    pub fn accepts(&self, id: &str, url: &str) -> bool {
        let Some(site) = self.host_of(id) else {
            return false;
        };
        let url_host = host_of_url(url).map(|host| bare_host(&host)).unwrap_or_default();
        if url_host == site {
            return true;
        }
        self.assigned
            .get(id.trim())
            .is_some_and(|urls| urls.iter().any(|item| item == url.trim()))
    }
}

pub async fn read_card(host: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(mds_crawl::USER_AGENT)
        .build()
        .ok()?;
    let response = client.get(card_url(host)).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    Some(text.chars().take(2_000).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEE: &str = "abcdef0123456789";

    #[test]
    fn card_must_name_the_bee_on_its_own_line() {
        assert!(card_names_bee("abcdef0123456789\n", BEE));
        assert!(!card_names_bee("пчела abcdef0123456789", BEE));
        assert!(!bee_id_ok("короткий"));
        assert!(!bee_id_ok("zzzzzzzzzzzzzzzz"));
    }

    #[test]
    fn own_pages_pass_and_foreign_pages_need_an_assignment() {
        let mut book = BeeBook::default();
        book.note_verified(BEE, "www.magtask.ru");
        assert!(book.accepts(BEE, "https://magtask.ru/about"));
        assert!(!book.accepts(BEE, "https://example.ru/a"));
        book.assign(BEE, vec!["https://example.ru/a".into()]);
        assert!(book.accepts(BEE, "https://example.ru/a"));
        assert!(!book.accepts(BEE, "https://example.ru/b"));
        book.refuse("magtask.ru");
        assert!(!book.accepts(BEE, "https://magtask.ru/about"));
        assert!(book.is_refused("www.magtask.ru"));
        book.restore("magtask.ru");
        book.note_verified(BEE, "magtask.ru");
        assert!(book.accepts(BEE, "https://magtask.ru/"));
    }
}
