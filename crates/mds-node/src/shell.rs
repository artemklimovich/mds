use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

const HOLD: Duration = Duration::from_secs(30 * 60);
const WINDOW: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug)]
pub struct ShellLimits {
    pub batch: usize,
    pub gap: Duration,
    pub window_max: usize,
    pub global_max: usize,
}

pub fn shell_id_ok(id: &str) -> bool {
    let id = id.trim();
    id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Память улья на время открытого приложения. Адрес человека сюда не пишется.
#[derive(Default)]
pub struct ShellBook {
    last: HashMap<String, Instant>,
    assigned: HashMap<String, (Instant, Vec<String>)>,
    windows: HashMap<String, VecDeque<Instant>>,
    global: VecDeque<Instant>,
}

impl ShellBook {
    pub fn ready(&mut self, id: &str, now: Instant, limits: ShellLimits) -> bool {
        self.gate(id, now, limits)
    }

    fn gate(&mut self, id: &str, now: Instant, limits: ShellLimits) -> bool {
        let id = id.trim();
        if !shell_id_ok(id) {
            return false;
        }
        let window_max = limits.window_max.clamp(4, 60);
        let global_max = limits.global_max.clamp(4, 600);
        self.global.retain(|at| now.saturating_duration_since(*at) < WINDOW);
        if self.global.len() >= global_max {
            return false;
        }
        let too_many = {
            let mine = self.windows.entry(id.to_string()).or_default();
            mine.retain(|at| now.saturating_duration_since(*at) < WINDOW);
            mine.len() >= window_max
        };
        if too_many {
            return false;
        }
        !self.last.get(id).is_some_and(|at| now.saturating_duration_since(*at) < limits.gap)
    }

    pub fn take(&mut self, id: &str, urls: Vec<String>, now: Instant, limits: ShellLimits) -> Vec<String> {
        let id = id.trim();
        if !self.gate(id, now, limits) {
            return Vec::new();
        }
        let batch = limits.batch.clamp(2, 30);
        let urls: Vec<String> = urls.into_iter().take(batch).filter(|url| !url.trim().is_empty()).collect();
        if urls.is_empty() {
            return Vec::new();
        }
        self.last.insert(id.to_string(), now);
        self.assigned.insert(id.to_string(), (now, urls.clone()));
        self.windows.entry(id.to_string()).or_default().push_back(now);
        self.global.push_back(now);
        urls
    }

    pub fn accepts(&mut self, id: &str, url: &str, now: Instant) -> bool {
        let id = id.trim();
        let Some((at, urls)) = self.assigned.get_mut(id) else {
            return false;
        };
        if now.saturating_duration_since(*at) > HOLD {
            return false;
        }
        let url = url.trim();
        if let Some(pos) = urls.iter().position(|item| item == url) {
            urls.remove(pos);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_only_returns_urls_the_hive_just_gave() {
        let mut book = ShellBook::default();
        let now = Instant::now();
        let id = "0123456789abcdef0123456789abcdef";
        let limits = ShellLimits {
            batch: 2,
            gap: Duration::from_secs(600),
            window_max: 4,
            global_max: 30,
        };
        let got = book.take(id, vec!["https://a.example/".into(), "https://b.example/".into()], now, limits);
        assert_eq!(got.len(), 2);
        assert!(book.accepts(id, "https://a.example/", now));
        assert!(!book.accepts(id, "https://evil.example/", now));
        assert!(book.take(id, vec!["https://c.example/".into()], now, limits).is_empty());
    }
}
