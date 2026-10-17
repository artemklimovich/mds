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

/// Оставляет одну ссылку из почти одинакового текста. Сначала страница сайта с пчелой, иначе та, что выше по счёту.
/// Нулевой отпечаток не склеивает: так помечены старые страницы без него. Похожий адрес сам по себе не склеивает.
pub fn drop_near_copies(hits: Vec<Hit>, pages: &[PageRow], bees: &[String]) -> Vec<Hit> {
    let mut out = Vec::new();
    let mut kept: Vec<KeptCopy> = Vec::new();
    for hit in hits {
        let page = pages.iter().find(|page| page.url == hit.url);
        let hash = page.map(|page| page.simhash).unwrap_or(0);
        if hash == 0 {
            out.push(hit);
            continue;
        }
        let bee = page.map(|page| holds_bee(&page.host, bees)).unwrap_or(false);
        if let Some(slot) = kept.iter_mut().find(|slot| (slot.hash ^ hash).count_ones() <= 3) {
            if bee && !slot.bee {
                out[slot.at] = hit;
                slot.hash = hash;
                slot.bee = true;
            }
            continue;
        }
        kept.push(KeptCopy {
            hash,
            at: out.len(),
            bee,
        });
        out.push(hit);
    }
    out
}

struct KeptCopy {
    hash: u64,
    at: usize,
    bee: bool,
}

fn holds_bee(host: &str, bees: &[String]) -> bool {
    let host = mds_types::bare_host(host);
    bees.iter().any(|bee| mds_types::bare_host(bee) == host)
}

pub fn prefer_region(hits: Vec<Hit>, region: Option<&str>) -> Vec<Hit> {
    prefer_geo(hits, region, None)
}

pub fn prefer_geo(hits: Vec<Hit>, region: Option<&str>, place: Option<&str>) -> Vec<Hit> {
    prefer_geo_radius(hits, region, place, None)
}

pub fn prefer_geo_radius(
    mut hits: Vec<Hit>,
    region: Option<&str>,
    place: Option<&str>,
    around: Option<(f64, f64, f64)>,
) -> Vec<Hit> {
    if let Some((ulat, ulon, radius_km)) = around {
        if mds_types::valid_coords(ulat, ulon) && radius_km > 0.0 {
            let mut inside = Vec::new();
            let mut outside = Vec::new();
            for mut hit in hits {
                if let (Some(lat), Some(lon)) = (hit.lat, hit.lon) {
                    if mds_types::valid_coords(lat, lon) {
                        let km = mds_types::haversine_km(ulat, ulon, lat, lon);
                        hit.distance_km = Some(km as f32);
                        if km <= radius_km {
                            inside.push(hit);
                            continue;
                        }
                    }
                }
                outside.push(hit);
            }
            inside.sort_by(|a, b| {
                a.distance_km
                    .unwrap_or(f32::MAX)
                    .partial_cmp(&b.distance_km.unwrap_or(f32::MAX))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let rest = order_by_region_and_place(outside, region, place);
            return inside.into_iter().chain(rest).collect();
        }
    }
    for hit in &mut hits {
        hit.distance_km = None;
    }
    order_by_region_and_place(hits, region, place)
}

fn order_by_region_and_place(hits: Vec<Hit>, region: Option<&str>, place: Option<&str>) -> Vec<Hit> {
    let Some(region) = region else {
        return hits;
    };
    let mut exact_place = Vec::new();
    let mut same_region = Vec::new();
    let mut other = Vec::new();
    for hit in hits {
        if hit.region.as_deref() == Some(region) {
            if let Some(place) = place {
                if hit.place.as_deref() == Some(place) {
                    exact_place.push(hit);
                    continue;
                }
            }
            same_region.push(hit);
        } else {
            other.push(hit);
        }
    }
    exact_place.into_iter().chain(same_region).chain(other).collect()
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
                region: page.region.clone(),
                place: page.place.clone(),
                distance_km: None,
                lat: page.lat,
                lon: page.lon,
                kind: page.kind,
                media_url: page.media_url.clone(),
                feed_url: page.feed_url.clone(),
                news_feed: page.news_feed.clone(),
                price: page.price.clone(),
            })
        })
        .collect()
}

fn query_subject_stems(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|tok| tok.chars().count() >= 3 && !GENERIC_TOKENS.contains(tok) && !mds_types::is_geo_word(tok))
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
            region: None,
            place: None,
            lat: None,
            lon: None,
            kind: 0,
            media_url: None,
            feed_url: None,
            news_feed: None,
            price: None,
            simhash: 0,
        }
    }

    #[test]
    fn home_region_stays_above_another_oblast() {
        let hits = vec![
            Hit {
                url: "https://orel.example/".into(),
                title: "Орёл".into(),
                snippet: "мёд".into(),
                score: 2.0,
                via_words: true,
                via_meaning: false,
                adult: false,
                volunteer: false,
                region: Some("Орловская область".into()),
                place: Some("Орёл".into()),
                distance_km: None,
                lat: None,
                lon: None,
                kind: 0,
                media_url: None,
                feed_url: None,
                news_feed: None,
                price: None,
            },
            Hit {
                url: "https://kamyshin.example/".into(),
                title: "Камышин".into(),
                snippet: "банки".into(),
                score: 1.0,
                via_words: true,
                via_meaning: false,
                adult: false,
                volunteer: false,
                region: Some("Волгоградская область".into()),
                place: Some("Камышин".into()),
                distance_km: None,
                lat: None,
                lon: None,
                kind: 0,
                media_url: None,
                feed_url: None,
                news_feed: None,
                price: None,
            },
        ];
        let ordered = prefer_region(hits, Some("Волгоградская область"));
        assert_eq!(ordered[0].region.as_deref(), Some("Волгоградская область"));
        assert_eq!(ordered[1].region.as_deref(), Some("Орловская область"));
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

    #[test]
    fn near_copies_leave_the_stronger_link() {
        let mut first = page(1, "shop.example");
        first.url = "https://shop.example/jar".into();
        first.simhash = 0b1111;
        let mut copy = page(2, "shop.example");
        copy.url = "https://shop.example/jar?print=1".into();
        copy.simhash = 0b1110;
        let mut other = page(3, "farm.example");
        other.simhash = 0b1111_0000;
        let hits = vec![
            hit_of(&first, 3.0),
            hit_of(&copy, 2.0),
            hit_of(&other, 1.0),
        ];
        let left = drop_near_copies(hits, &[first, copy, other], &[]);
        assert_eq!(left.len(), 2);
        assert_eq!(left[0].url, "https://shop.example/jar");
        assert_eq!(left[1].url, "https://farm.example/item");
    }

    #[test]
    fn a_bee_page_stays_ahead_of_a_stronger_copy() {
        let mut bee = page(1, "shop.example");
        bee.url = "https://shop.example/jar".into();
        bee.simhash = 0b1111;
        let mut copy = page(2, "mirror.example");
        copy.url = "https://mirror.example/jar".into();
        copy.simhash = 0b0111;
        let hits = vec![hit_of(&copy, 5.0), hit_of(&bee, 1.0)];
        let bees = vec!["www.shop.example".into()];
        let left = drop_near_copies(hits, &[bee, copy], &bees);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].url, "https://shop.example/jar");
    }

    #[test]
    fn different_pages_of_one_site_stay_apart() {
        let mut honey = page(1, "shop.example");
        honey.url = "https://shop.example/honey".into();
        honey.simhash = 0b1111;
        let mut about = page(2, "shop.example");
        about.url = "https://shop.example/about".into();
        about.simhash = 0x00ff_0000_0000_0000;
        let hits = vec![hit_of(&honey, 3.0), hit_of(&about, 2.0)];
        let left = drop_near_copies(hits, &[honey, about], &[]);
        assert_eq!(left.len(), 2);
    }

    #[test]
    fn radius_filter_sorts_points_by_distance() {
        let mut far = hit_of(&page(1, "far.example"), 5.0);
        far.lat = Some(55.7000);
        far.lon = Some(37.5000);
        let mut near = hit_of(&page(2, "near.example"), 2.0);
        near.lat = Some(55.7540);
        near.lon = Some(37.6200);
        let ordered = prefer_geo_radius(vec![far, near], Some("Москва"), None, Some((55.7558, 37.6173, 1.0)));
        assert_eq!(ordered[0].url, "https://near.example/item");
        assert!(ordered[0].distance_km.unwrap_or(99.0) < 0.5);
    }

    fn hit_of(page: &PageRow, score: f32) -> Hit {
        Hit {
            url: page.url.clone(),
            title: page.title.clone(),
            snippet: page.text.clone(),
            score,
            via_words: true,
            via_meaning: false,
            adult: false,
            volunteer: false,
            region: None,
            place: None,
            distance_km: None,
            lat: None,
            lon: None,
            kind: 0,
            media_url: None,
            feed_url: None,
            news_feed: None,
            price: None,
        }
    }
}
