use std::collections::HashMap;

mod safety;

use mds_index::{PageRow, VectorMatch, WordMatch};
use mds_types::{snippet, Hit};

pub use safety::{is_doorway, page_lane, query_lane, Lane};

pub const RRF_K: f32 = 60.0;
const MEANING_WITH_WORD_FLOOR: f32 = 0.80;
const MEANING_COMMERCIAL_FLOOR: f32 = 0.88;
const MEANING_ONLY_FLOOR: f32 = 0.835;
const MEANING_SPREAD_MAX: f32 = 0.025;

const MARKETPLACES: &[&str] = &[
    "ozon.ru",
    "wildberries.ru",
    "wb.ru",
    "market.yandex.ru",
    "avito.ru",
    "megamarket.ru",
];

const COMMERCIAL: &[&str] = &[
    "купить",
    "цена",
    "цены",
    "заказать",
    "магазин",
    "доставка",
    "стоимость",
];

const GENERIC_TOKENS: &[&str] = &[
    "как",
    "где",
    "что",
    "для",
    "или",
    "это",
    "все",
    "при",
    "без",
    "под",
    "над",
    "про",
    "купить",
    "цена",
    "цены",
    "заказать",
    "магазин",
    "доставка",
    "стоимость",
    "онлайн",
    "сайт",
    "официальный",
    "москва",
    "москве",
    "россия",
    "россии",
];

#[derive(Clone, Debug)]
struct Fused {
    id: i64,
    score: f32,
    via_words: bool,
    via_meaning: bool,
}

pub fn fuse(query: &str, words: &[WordMatch], vectors: &[VectorMatch], pages: &[PageRow]) -> Vec<Hit> {
    let mut scores: HashMap<i64, Fused> = HashMap::new();
    for hit in words {
        let entry = scores.entry(hit.id).or_insert(Fused {
            id: hit.id,
            score: 0.0,
            via_words: false,
            via_meaning: false,
        });
        entry.score += 1.0 / (RRF_K + hit.rank as f32);
        entry.via_words = true;
    }

    let subject_stems = query_subject_stems(query);
    let commercial = is_commercial(query);
    let top_cosine = vectors.first().map(|v| v.cosine).unwrap_or(0.0);
    let base_meaning_floor = if commercial || subject_stems.len() <= 1 {
        MEANING_COMMERCIAL_FLOOR
    } else {
        MEANING_ONLY_FLOOR
    };
    let adaptive_floor = (top_cosine - MEANING_SPREAD_MAX).max(base_meaning_floor);

    for hit in vectors {
        let has_word = scores.get(&hit.id).map(|f| f.via_words).unwrap_or(false);
        if has_word {
            if hit.cosine >= MEANING_WITH_WORD_FLOOR {
                if let Some(entry) = scores.get_mut(&hit.id) {
                    entry.score += 1.0 / (RRF_K + hit.rank as f32);
                    entry.via_meaning = true;
                }
            }
            continue;
        }
        let stem_match = pages
            .iter()
            .find(|p| p.id == hit.id)
            .map(|p| page_matches_any_stem(p, &subject_stems))
            .unwrap_or(false);
        let floor = if stem_match {
            MEANING_WITH_WORD_FLOOR
        } else {
            adaptive_floor
        };
        if hit.cosine < floor {
            continue;
        }
        let entry = scores.entry(hit.id).or_insert(Fused {
            id: hit.id,
            score: 0.0,
            via_words: false,
            via_meaning: false,
        });
        entry.score += 1.0 / (RRF_K + hit.rank as f32);
        entry.via_meaning = true;
    }

    let mut ranked: Vec<Fused> = scores.into_values().collect();
    for item in &mut ranked {
        if let Some(page) = pages.iter().find(|page| page.id == item.id) {
            if commercial && is_marketplace(&page.host) {
                item.score *= 0.5;
            }
            if is_doorway(&page.text) {
                item.score *= 0.55;
            }
        }
    }
    ranked.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    ranked
        .into_iter()
        .filter_map(|item| {
            let page = pages.iter().find(|page| page.id == item.id)?;
            let lane = page_lane(&page.url, &page.title, &page.text);
            if lane == Lane::Hidden {
                return None;
            }
            Some(Hit {
                url: page.url.clone(),
                title: page.title.clone(),
                snippet: snippet(&page.text, query),
                score: item.score,
                via_words: item.via_words,
                via_meaning: item.via_meaning,
                adult: lane == Lane::Adult,
                volunteer: false,
            })
        })
        .collect()
}

fn query_subject_stems(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|tok| tok.chars().count() >= 3 && !GENERIC_TOKENS.contains(tok))
        .map(stem_ru)
        .collect()
}

fn stem_ru(word: &str) -> String {
    let chars: Vec<char> = word.chars().collect();
    let keep = match chars.len() {
        0..=4 => chars.len(),
        5..=6 => chars.len() - 1,
        _ => chars.len() - 2,
    };
    chars.into_iter().take(keep).collect()
}

fn page_matches_any_stem(page: &PageRow, stems: &[String]) -> bool {
    if stems.is_empty() {
        return false;
    }
    let hay = format!("{} {}", page.title, page.text).to_lowercase();
    hay.split(|c: char| !c.is_alphanumeric()).any(|word| {
        stems.iter().any(|stem| {
            if stem.chars().count() <= 3 {
                word == stem
            } else {
                word.starts_with(stem.as_str())
            }
        })
    })
}

pub fn is_commercial(query: &str) -> bool {
    let query = query.to_lowercase();
    COMMERCIAL.iter().any(|word| query.contains(word))
}

/// Небольшой подъём сайта, который держит лёгкую пчелу. Один шаг вверху выдачи может смениться, далёкий результат на первое место не выходит.
pub const VOLUNTEER_NUDGE: f32 = 1.05;

pub fn nudge_volunteer(score: f32) -> f32 {
    score * VOLUNTEER_NUDGE
}

pub fn is_marketplace(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    MARKETPLACES.iter().any(|market| host == *market || host.ends_with(&format!(".{market}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(id: i64, host: &str) -> PageRow {
        PageRow {
            id,
            url: format!("https://{host}/item"),
            title: host.into(),
            host: host.into(),
            text: "Карточка товара".into(),
        }
    }

    #[test]
    fn marketplace_loses_to_direct_site_on_price_query() {
        let words = vec![
            WordMatch { id: 1, rank: 1 },
            WordMatch { id: 2, rank: 1 },
        ];
        let pages = vec![page(1, "ozon.ru"), page(2, "farm.example")];
        let hits = fuse("купить молоко цена", &words, &[], &pages);
        assert_eq!(hits[0].url, "https://farm.example/item");
    }

    #[test]
    fn meaning_below_floor_is_dropped() {
        let vectors = vec![VectorMatch {
            id: 1,
            rank: 1,
            cosine: 0.83,
        }];
        let pages = vec![page(1, "farm.example")];
        let hits = fuse("купить мёд", &[], &vectors, &pages);
        assert!(hits.is_empty());
    }

    #[test]
    fn hidden_page_is_omitted_and_doorway_ranks_lower() {
        let words = vec![
            WordMatch { id: 1, rank: 1 },
            WordMatch { id: 2, rank: 1 },
            WordMatch { id: 3, rank: 2 },
        ];
        let mut hidden = page(1, "shop.example");
        hidden.title = "Купить мефедрон".into();
        hidden.text = "доставка сегодня".into();
        let mut doorway = page(2, "news.example");
        doorway.text = "Рекламная статья. Перейти на сайт партнера и читайте дальше.".into();
        let direct = page(3, "farm.example");
        let hits = fuse("мёд", &words, &[], &[hidden, doorway, direct]);
        assert!(hits.iter().all(|hit| !hit.url.contains("shop.example")));
        let doorway_score = hits.iter().find(|hit| hit.url.contains("news.example")).unwrap().score;
        let direct_score = hits.iter().find(|hit| hit.url.contains("farm.example")).unwrap().score;
        assert!(direct_score > doorway_score);
    }

    #[test]
    fn volunteer_nudge_does_not_lift_a_distant_hit_over_the_first() {
        let first = 1.0 / (RRF_K + 1.0);
        let tenth = nudge_volunteer(1.0 / (RRF_K + 10.0));
        assert!(tenth < first);
    }
}
