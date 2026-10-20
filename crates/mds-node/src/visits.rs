use std::collections::HashMap;
use std::time::{Duration, Instant};

use mds_index::VisitDelta;

const IDLE: Duration = Duration::from_secs(30 * 60);
const PACE: Duration = Duration::from_secs(60);
const PACE_MAX: usize = 20;

struct OpenVisit {
    started: Instant,
    last: Instant,
    views: u32,
    bounce: bool,
    searches_at: Vec<Instant>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchMark {
    pub empty: bool,
    pub failed: bool,
    pub refused: bool,
}

pub fn fresh_token() -> String {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).expect("случайные байты");
    buf.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Default)]
pub struct VisitBook {
    open: HashMap<String, OpenVisit>,
}

impl VisitBook {
    pub fn page(&mut self, token: &str, now: Instant) -> VisitDelta {
        let mut delta = self.sweep(now);
        if let Some(visit) = self.open.get_mut(token) {
            visit.views = visit.views.saturating_add(1);
            visit.last = now;
            if visit.bounce && visit.views > 1 {
                visit.bounce = false;
                delta.bounces -= 1;
            }
            delta.views += 1;
            return delta;
        }
        self.open.insert(
            token.to_string(),
            OpenVisit {
                started: now,
                last: now,
                views: 1,
                bounce: true,
                searches_at: Vec::new(),
            },
        );
        delta.visits += 1;
        delta.views += 1;
        delta.bounces += 1;
        delta
    }

    pub fn search(&mut self, token: &str, now: Instant, mark: SearchMark) -> VisitDelta {
        let mut delta = VisitDelta::default();
        if !self.open.contains_key(token) {
            delta = self.page(token, now);
        }
        let Some(visit) = self.open.get_mut(token) else {
            return delta;
        };
        visit.last = now;
        note_pace(visit, now);
        if visit.bounce {
            visit.bounce = false;
            delta.bounces -= 1;
        }
        delta.searches += 1;
        if mark.failed {
            delta.search_errors += 1;
        } else if mark.refused {
            delta.refused += 1;
        } else if mark.empty {
            delta.empty_searches += 1;
        }
        delta
    }

    /// Сессия без cookie или после перезапуска пропускается: первый поиск человека проходит.
    /// Пачка с тем же номером останавливается здесь. Номер на диск не пишется.
    pub fn pace_open(&mut self, token: &str, now: Instant) -> bool {
        let Some(visit) = self.open.get_mut(token) else {
            return true;
        };
        visit.searches_at.retain(|at| now.saturating_duration_since(*at) < PACE);
        if visit.searches_at.len() >= PACE_MAX {
            visit.last = now;
            return false;
        }
        true
    }

    fn sweep(&mut self, now: Instant) -> VisitDelta {
        let mut delta = VisitDelta::default();
        let idle = self
            .open
            .iter()
            .filter(|(_, visit)| now.saturating_duration_since(visit.last) >= IDLE)
            .map(|(token, visit)| {
                let dwell = visit.last.saturating_duration_since(visit.started).as_secs() as i64;
                (token.clone(), dwell)
            })
            .collect::<Vec<_>>();
        for (token, dwell) in idle {
            self.open.remove(&token);
            if dwell > 0 {
                delta.dwell_seconds += dwell;
                delta.timed_visits += 1;
            }
        }
        delta
    }
}

fn note_pace(visit: &mut OpenVisit, now: Instant) {
    visit.searches_at.retain(|at| now.saturating_duration_since(*at) < PACE);
    if visit.searches_at.len() < PACE_MAX {
        visit.searches_at.push(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_page_is_a_bounce_and_a_search_cancels_it() {
        let mut book = VisitBook::default();
        let now = Instant::now();
        let first = book.page("a", now);
        assert_eq!(first.visits, 1);
        assert_eq!(first.bounces, 1);
        let second = book.page("a", now + Duration::from_secs(20));
        assert_eq!(second.visits, 0);
        assert_eq!(second.bounces, -1);
        assert_eq!(second.views, 1);
        let looked = book.search(
            "b",
            now,
            SearchMark {
                empty: true,
                failed: false,
                refused: false,
            },
        );
        assert_eq!(looked.visits, 1);
        assert_eq!(looked.bounces, 0);
        assert_eq!(looked.searches, 1);
        assert_eq!(looked.empty_searches, 1);
    }

    #[test]
    fn idle_visit_adds_time_between_requests_only() {
        let mut book = VisitBook::default();
        let now = Instant::now();
        book.page("a", now);
        book.page("a", now + Duration::from_secs(40));
        let closed = book.page("b", now + IDLE + Duration::from_secs(41));
        assert_eq!(closed.timed_visits, 1);
        assert_eq!(closed.dwell_seconds, 40);
        assert_eq!(closed.visits, 1);
    }

    #[test]
    fn a_burst_from_one_session_waits_and_a_new_session_does_not() {
        let mut book = VisitBook::default();
        let now = Instant::now();
        let mark = SearchMark {
            empty: false,
            failed: false,
            refused: false,
        };
        book.page("a", now);
        for step in 0..20 {
            let at = now + Duration::from_millis(step);
            assert!(book.pace_open("a", at));
            book.search("a", at, mark);
        }
        assert!(!book.pace_open("a", now + Duration::from_secs(30)));
        assert!(book.pace_open("a", now + Duration::from_secs(61)));
        assert!(book.pace_open("fresh", now));
    }
}
