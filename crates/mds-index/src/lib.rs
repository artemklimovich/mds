use std::collections::HashMap;
use std::path::Path;

use mds_crawl::{is_broken_url, is_near_duplicate, is_not_found, strip_boilerplate, tidy_url, FetchedPage};
use mds_types::{
    all_kinds, bare_host, host_of_url, is_channel_host, is_marketplace_host, is_press_front, is_press_host, is_product_card, kind_from_stored, cosine,
    QuantVector, EMBED_DIMS, HOST_PAGE_CAP, KIND_CHANNEL, KIND_NEWS, KIND_VIDEO,
};
use rusqlite::{params, Connection};
use thiserror::Error;

mod spool;
pub use spool::{spool_path, ParkedPage, TextSpool};

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("sqlite ({context}): {source}")]
    Sqlite {
        context: &'static str,
        #[source]
        source: rusqlite::Error,
    },
    #[error("вектор: {0}")]
    Vector(String),
}

impl From<rusqlite::Error> for IndexError {
    fn from(source: rusqlite::Error) -> Self {
        Self::Sqlite {
            context: "query",
            source,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellPace {
    pub batch: i64,
    pub gap_min: i64,
    pub window_max: i64,
}

/// Потолок, выше которого телефон в быстром режиме не поднимается.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellCeiling {
    pub max_batch: i64,
    pub min_gap: i64,
}

/// Сколько адресов улей сам берёт за один заход и сколько секунд ждёт.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrawlPace {
    pub batch: i64,
    pub gap_secs: i64,
}

/// Сколько чужих адресов пчела получает за заход.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeePace {
    pub batch: i64,
    pub gap_secs: i64,
}

/// Сколько уже лежащих страниц перепроверять за заход и какая пауза между заходами.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefreshPace {
    pub batch: i64,
    pub gap_secs: i64,
}

/// Страница, которую пора спросить у сайта ещё раз.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshJob {
    pub url: String,
    pub etag: Option<String>,
    pub modified: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntakeDay {
    pub day: String,
    pub new_pages: i64,
    pub changed_pages: i64,
    pub same_pages: i64,
    pub new_hosts: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadHour {
    pub hour: String,
    pub cpu_pct: i64,
    pub ram_pct: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeeTotals {
    pub day: i64,
    pub week: i64,
    pub month: i64,
    pub all: i64,
    pub quiet: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownNode {
    pub base: String,
    pub alive: bool,
    pub latency_ms: u64,
    pub last_ok: Option<i64>,
}

fn span_filter(span: &str) -> String {
    match span {
        "day" => "day = date('now','localtime')".into(),
        "yesterday" => "day = date('now','localtime','-1 day')".into(),
        "week" => "day >= date('now','localtime','-' || ((cast(strftime('%w','now','localtime') as integer) + 6) % 7) || ' days')".into(),
        "month" => "day >= date('now','localtime','start of month')".into(),
        _ => "1 = 1".into(),
    }
}

fn clamp_meta(raw: Option<String>, low: i64, high: i64, fallback: i64) -> i64 {
    raw.and_then(|text| text.parse().ok()).unwrap_or(fallback).clamp(low, high)
}

pub struct Index {
    conn: Connection,
}

#[derive(Clone, Debug)]
pub struct WordMatch {
    pub id: i64,
    pub rank: usize,
}

#[derive(Clone, Debug)]
pub struct VectorMatch {
    pub id: i64,
    pub rank: usize,
    pub cosine: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredLeaf {
    pub url: String,
    pub host: String,
    pub title: String,
    pub text: String,
    pub simhash: u64,
    pub revision: u64,
}

#[derive(Clone, Debug)]
pub struct PageRow {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub host: String,
    pub text: String,
    pub region: Option<String>,
    pub place: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub kind: u32,
    pub media_url: Option<String>,
    pub feed_url: Option<String>,
    pub news_feed: Option<String>,
    pub price: Option<String>,
    pub simhash: u64,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self, IndexError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        let conn = Connection::open(path).map_err(|source| IndexError::Sqlite {
            context: "open",
            source,
        })?;
        init_schema(&conn)?;
        ensure_region_column(&conn)?;
        ensure_kind_columns(&conn)?;
        ensure_queue_lease(&conn)?;
        ensure_queue_worth(&conn)?;
        ensure_refresh_columns(&conn)?;
        ensure_anchor_table(&conn)?;
        ensure_revision(&conn)?;
        let index = Self { conn };
        index.seed_revisions()?;
        index.seed_refresh()?;
        let _ = index.fill_kinds()?;
        let (labeled, total) = index.fill_regions()?;
        if total > 0 {
            eprintln!("MDS: регион подписан у {labeled} страниц из {total}");
        }
        Ok(index)
    }

    fn seed_revisions(&self) -> Result<(), IndexError> {
        let seeded: String = self
            .conn
            .query_row(
                "SELECT val FROM index_meta WHERE key = 'revision_seeded'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_default();
        if seeded == "1" {
            return Ok(());
        }
        self.conn
            .execute("UPDATE pages SET revision = id WHERE revision = 0", [])?;
        let max: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(revision), 0) FROM pages", [], |row| row.get(0))?;
        self.conn.execute(
            "INSERT INTO index_meta (key, val) VALUES ('index_revision', ?1)
             ON CONFLICT(key) DO UPDATE SET val = CASE
                WHEN CAST(index_meta.val AS INTEGER) < CAST(excluded.val AS INTEGER) THEN excluded.val
                ELSE index_meta.val END",
            [max.to_string()],
        )?;
        self.conn.execute(
            "INSERT OR REPLACE INTO index_meta (key, val) VALUES ('revision_seeded', '1')",
            [],
        )?;
        Ok(())
    }

    pub fn fill_regions(&self) -> Result<(usize, usize), IndexError> {
        let mut stmt = self.conn.prepare("SELECT id, url, host, title, text, lat, lon FROM pages")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<f64>>(5)?,
                row.get::<_, Option<f64>>(6)?,
            ))
        })?;
        let mut labeled = 0;
        let mut total = 0;
        let mut pending = Vec::new();
        let mut host_coords: Vec<(String, f64, f64)> = Vec::new();
        let mut host_contacts_map: HashMap<String, mds_types::Contacts> = HashMap::new();
        for row in rows {
            let (id, url, host, title, text, cur_lat, cur_lon) = row?;
            total += 1;
            let full = format!("{title}\n{text}");
            let geo = mds_types::geo_for_page(&url, &full);
            let region = geo.region.map(str::to_string);
            let place = geo.place.map(str::to_string);
            let bare = mds_types::bare_host(&host);
            let is_shared = is_marketplace_host(&bare) || is_channel_host(&bare);
            let coords = mds_types::coords_in_text(&full).or_else(|| match (cur_lat, cur_lon) {
                (Some(lat), Some(lon)) if mds_types::valid_coords(lat, lon) => Some((lat, lon)),
                _ => None,
            });
            if let Some((lat, lon)) = coords {
                if !bare.is_empty() && !is_shared {
                    host_coords.push((bare.clone(), lat, lon));
                }
            }
            if !bare.is_empty() && !is_shared {
                let c = mds_types::contacts_in_text(&full);
                if !c.is_empty() {
                    host_contacts_map.entry(bare).or_default().merge(&c);
                }
            }
            if region.is_some() {
                labeled += 1;
            }
            pending.push((region, place, coords.map(|c| c.0), coords.map(|c| c.1), host, id));
        }
        drop(stmt);
        for (bare, lat, lon) in &host_coords {
            let _ = self.conn.execute(
                "INSERT INTO host_geo (host, lat, lon) VALUES (?1, ?2, ?3)
                 ON CONFLICT(host) DO UPDATE SET lat=excluded.lat, lon=excluded.lon",
                params![bare, lat, lon],
            );
        }
        for (bare, c) in &host_contacts_map {
            let _ = upsert_host_contacts(&self.conn, bare, c);
        }
        for (region, place, mut lat, mut lon, host, id) in pending {
            if lat.is_none() || lon.is_none() {
                let bare = mds_types::bare_host(&host);
                if let Ok((hlat, hlon)) = self.conn.query_row(
                    "SELECT lat, lon FROM host_geo WHERE host = ?1",
                    [&bare],
                    |r| Ok((r.get::<_, f64>(0)?, r.get::<_, f64>(1)?)),
                ) {
                    lat = Some(hlat);
                    lon = Some(hlon);
                }
            }
            self.conn.execute(
                "UPDATE pages SET region = ?1, place = ?2, lat = COALESCE(?3, lat), lon = COALESCE(?4, lon) WHERE id = ?5",
                params![region, place, lat, lon, id],
            )?;
        }
        Ok((labeled, total))
    }

    pub fn len(&self) -> Result<usize, IndexError> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    pub fn census(&self) -> Result<(usize, usize), IndexError> {
        let pages: i64 = self.conn.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))?;
        let sites: i64 = self.conn.query_row("SELECT COUNT(DISTINCT host) FROM pages", [], |row| row.get(0))?;
        Ok((sites as usize, pages as usize))
    }

    pub fn is_duplicate(&self, hash: u64) -> Result<bool, IndexError> {
        let mut stmt = self.conn.prepare("SELECT simhash FROM pages")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        for stored in rows {
            let stored = stored? as u64;
            if is_near_duplicate(hash, stored) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn insert(
        &mut self,
        page: &FetchedPage,
        vector: &QuantVector,
        domain_hash: &[u8; 16],
        semantic_bucket: u16,
    ) -> Result<(), IndexError> {
        if vector.values.len() != EMBED_DIMS {
            return Err(IndexError::Vector(format!(
                "ожидалось {EMBED_DIMS}, получено {}",
                vector.values.len()
            )));
        }
        let blob = vector.values.iter().map(|v| *v as u8).collect::<Vec<_>>();
        let tx = self.conn.transaction()?;
        let full = format!("{}\n{}", page.title, page.text);
        let geo = mds_types::geo_for_page(&page.url, &full);
        let region = geo.region;
        let place = geo.place;
        let bare = mds_types::bare_host(&page.host);
        let is_shared = is_marketplace_host(&bare) || is_channel_host(&bare);
        if !bare.is_empty() && !is_shared {
            let c = mds_types::contacts_in_text(&full);
            if !c.is_empty() {
                let _ = upsert_host_contacts(&tx, &bare, &c);
            }
        }
        let direct_coords = mds_types::coords_in_text(&full);
        if let Some((lat, lon)) = direct_coords {
            if !bare.is_empty() && !is_shared {
                let _ = tx.execute(
                    "INSERT INTO host_geo (host, lat, lon) VALUES (?1, ?2, ?3)
                     ON CONFLICT(host) DO UPDATE SET lat=excluded.lat, lon=excluded.lon",
                    params![&bare, lat, lon],
                );
                let _ = tx.execute(
                    "UPDATE pages SET lat = ?1, lon = ?2 WHERE (host = ?3 OR host = ?4) AND lat IS NULL",
                    params![lat, lon, &bare, format!("www.{bare}")],
                );
            }
        }
        let (lat, lon) = match direct_coords {
            Some((lat, lon)) => (Some(lat), Some(lon)),
            None => tx
                .query_row(
                    "SELECT lat, lon FROM host_geo WHERE host = ?1",
                    [&bare],
                    |row| Ok((Some(row.get::<_, f64>(0)?), Some(row.get::<_, f64>(1)?))),
                )
                .unwrap_or((None, None)),
        };
        let revision = next_revision(&tx)?;
        let anchors = anchor_words_in(&tx, &page.url)?;
        let fts_text = join_anchors(&page.text, &anchors);
        tx.execute(
            "INSERT INTO pages (url, title, host, text, simhash, scale, vector, domain_hash, semantic_bucket, region, place, lat, lon, kind, media_url, feed_url, price, news_feed, next_check, revision)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, strftime('%s','now') + 86400, ?19)
             ON CONFLICT(url) DO UPDATE SET
                title=excluded.title,
                host=excluded.host,
                text=excluded.text,
                simhash=excluded.simhash,
                scale=excluded.scale,
                vector=excluded.vector,
                domain_hash=excluded.domain_hash,
                semantic_bucket=excluded.semantic_bucket,
                region=excluded.region,
                place=excluded.place,
                lat=COALESCE(excluded.lat, pages.lat),
                lon=COALESCE(excluded.lon, pages.lon),
                kind=excluded.kind,
                media_url=excluded.media_url,
                feed_url=excluded.feed_url,
                price=excluded.price,
                news_feed=excluded.news_feed,
                next_check=strftime('%s','now') + 86400,
                revision=excluded.revision",
            params![
                page.url,
                page.title,
                page.host,
                page.text,
                page.simhash as i64,
                vector.scale,
                blob,
                domain_hash.as_slice(),
                semantic_bucket as i64,
                region,
                place,
                lat,
                lon,
                page.kind,
                page.media_url,
                page.feed_url,
                page.price,
                page.news_feed,
                revision,
            ],
        )?;
        let id: i64 = tx.query_row("SELECT id FROM pages WHERE url = ?1", [&page.url], |row| row.get(0))?;
        tx.execute("DELETE FROM pages_fts WHERE rowid = ?1", [id])?;
        tx.execute(
            "INSERT INTO pages_fts(rowid, title, text) VALUES (?1, ?2, ?3)",
            params![id, page.title, fts_text],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn search_words(&self, query: &str, limit: usize, kind_mask: u32) -> Result<Vec<WordMatch>, IndexError> {
        let Some(match_query) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT pages.id
             FROM pages_fts
             JOIN pages ON pages.id = pages_fts.rowid
             WHERE pages_fts MATCH ?1
               AND ((?3 != 0 AND (pages.kind & ?3) != 0) OR (?3 = 0 AND (pages.kind & ?4) = 0))
             ORDER BY bm25(pages_fts)
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![match_query, limit as i64, kind_mask, KIND_NEWS | KIND_CHANNEL],
            |row| row.get::<_, i64>(0),
        )?;
        let mut out = Vec::new();
        for (offset, id) in rows.enumerate() {
            out.push(WordMatch {
                id: id?,
                rank: offset + 1,
            });
        }
        Ok(out)
    }

    pub fn search_vectors(
        &self,
        query: &[f32],
        bucket: Option<u16>,
        limit: usize,
        kind_mask: u32,
    ) -> Result<Vec<VectorMatch>, IndexError> {
        let mut scored = match bucket {
            Some(bucket) => self.score_bucket_neighbors(query, bucket, kind_mask)?,
            None => Vec::new(),
        };
        if scored.len() < limit {
            scored = self.score_all_vectors(query, kind_mask)?;
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(offset, (id, cosine))| VectorMatch {
                id,
                rank: offset + 1,
                cosine,
            })
            .collect())
    }

    fn score_bucket_neighbors(
        &self,
        query: &[f32],
        bucket: u16,
        kind_mask: u32,
    ) -> Result<Vec<(i64, f32)>, IndexError> {
        let mut b = [0_i64; 17];
        b[0] = bucket as i64;
        for i in 0..16 {
            b[i + 1] = (bucket ^ (1 << i)) as i64;
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, scale, vector, kind FROM pages
             WHERE semantic_bucket IN (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
               AND ((?18 != 0 AND (kind & ?18) != 0) OR (?18 = 0 AND (kind & ?19) = 0))",
        )?;
        let rows = stmt.query_map(
            params![
                b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8],
                b[9], b[10], b[11], b[12], b[13], b[14], b[15], b[16], kind_mask, KIND_NEWS | KIND_CHANNEL
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, f32>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )?;
        score_rows(query, rows)
    }

    fn score_all_vectors(&self, query: &[f32], kind_mask: u32) -> Result<Vec<(i64, f32)>, IndexError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, scale, vector FROM pages WHERE ((?1 != 0 AND (kind & ?1) != 0) OR (?1 = 0 AND (kind & ?2) = 0))",
            )?;
        let rows = stmt.query_map(params![kind_mask, KIND_NEWS | KIND_CHANNEL], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f32>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        score_rows(query, rows)
    }

    pub fn enqueue_urls(&self, urls: &[String]) -> Result<(), IndexError> {
        let mut pressure: HashMap<String, i64> = HashMap::new();
        for url in urls {
            let trimmed = tidy_url(url.trim());
            if trimmed.is_empty() || is_broken_url(&trimmed) {
                continue;
            }
            let Some(host) = host_of_url(&trimmed) else {
                continue;
            };
            let bare = bare_host(&host);
            if is_marketplace_host(&bare) && is_product_card(&trimmed) {
                continue;
            }
            if is_press_host(&bare) && !is_press_front(&trimmed) {
                continue;
            }
            let volunteer = self.is_volunteer_host(&bare)?;
            if self.has_picks(&bare)? && !volunteer && !self.is_pick(&bare, &trimmed)? {
                continue;
            }
            if !volunteer {
                let load = if let Some(load) = pressure.get(&bare).copied() {
                    load
                } else {
                    let load = if self.has_picks(&bare)? {
                        self.pick_pressure(&bare)?
                    } else {
                        self.host_pressure(&bare)?
                    };
                    pressure.insert(bare.clone(), load);
                    load
                };
                if load >= HOST_PAGE_CAP {
                    continue;
                }
                pressure.insert(bare.clone(), load + 1);
            }
            let worth = self.queue_worth(&bare)?;
            self.conn.execute(
                "INSERT INTO crawl_queue (url, visited, added_at, worth) VALUES (?1, 0, strftime('%s','now'), ?2)
                 ON CONFLICT(url) DO NOTHING",
                params![trimmed, worth],
            )?;
        }
        Ok(())
    }

    fn queue_worth(&self, bare: &str) -> Result<i64, IndexError> {
        if self.host_pages(bare)? > 0 {
            Ok(2)
        } else if self.host_is_quiet(bare)? {
            Ok(4)
        } else {
            Ok(3)
        }
    }

    pub fn note_host_quiet(&self, host: &str) -> Result<(), IndexError> {
        let host = bare_host(host);
        if host.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO host_quiet (host) VALUES (?1) ON CONFLICT(host) DO NOTHING",
            [host],
        )?;
        Ok(())
    }

    pub fn clear_host_quiet(&self, host: &str) -> Result<(), IndexError> {
        let host = bare_host(host);
        if host.is_empty() {
            return Ok(());
        }
        self.conn.execute("DELETE FROM host_quiet WHERE host = ?1", [host])?;
        Ok(())
    }

    fn host_is_quiet(&self, bare: &str) -> Result<bool, IndexError> {
        let found: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM host_quiet WHERE host = ?1",
            [bare],
            |row| row.get(0),
        )?;
        Ok(found > 0)
    }

    pub fn remember_anchor(&self, target: &str, words: &str) -> Result<(), IndexError> {
        let target = target.trim();
        let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
        let words: String = words.chars().take(80).collect();
        if target.is_empty() || words.chars().count() < 2 {
            return Ok(());
        }
        let existing: String = self
            .conn
            .query_row(
                "SELECT words FROM anchor_text WHERE url = ?1",
                [target],
                |row| row.get(0),
            )
            .unwrap_or_default();
        if existing.split_whitespace().any(|have| have == words) {
            return Ok(());
        }
        let mut joined = existing;
        if !joined.is_empty() {
            joined.push(' ');
        }
        joined.push_str(&words);
        let joined: String = joined.chars().take(500).collect();
        self.conn.execute(
            "INSERT INTO anchor_text (url, words) VALUES (?1, ?2)
             ON CONFLICT(url) DO UPDATE SET words = excluded.words",
            params![target, joined],
        )?;
        self.paint_anchor(target)?;
        Ok(())
    }

    fn paint_anchor(&self, url: &str) -> Result<(), IndexError> {
        let found: Result<(i64, String, String), rusqlite::Error> = self.conn.query_row(
            "SELECT id, title, text FROM pages WHERE url = ?1",
            [url],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        );
        let Ok((id, title, text)) = found else {
            return Ok(());
        };
        let anchors = anchor_words_in(&self.conn, url)?;
        let body = join_anchors(&text, &anchors);
        self.conn
            .execute("DELETE FROM pages_fts WHERE rowid = ?1", [id])?;
        self.conn.execute(
            "INSERT INTO pages_fts(rowid, title, text) VALUES (?1, ?2, ?3)",
            params![id, title, body],
        )?;
        Ok(())
    }

    pub fn defer_urls(&self, urls: &[String]) -> Result<(), IndexError> {
        for url in urls {
            self.conn.execute(
                "UPDATE crawl_queue SET worth = 5, lease_until = 0 WHERE url = ?1",
                [url.as_str()],
            )?;
        }
        Ok(())
    }

    pub fn release_deferred(&self) -> Result<(), IndexError> {
        self.conn
            .execute("UPDATE crawl_queue SET worth = 3 WHERE worth = 5", [])?;
        Ok(())
    }

    pub fn index_revision(&self) -> Result<u64, IndexError> {
        let value: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(CAST(val AS INTEGER)), 0) FROM index_meta WHERE key = 'index_revision'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(value.max(0) as u64)
    }

    pub fn pages_after(&self, revision: u64, limit: usize) -> Result<Vec<StoredLeaf>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT url, host, title, text, simhash, revision FROM pages
             WHERE revision > ?1 ORDER BY revision ASC, id ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![revision as i64, limit as i64], |row| {
            Ok(StoredLeaf {
                url: row.get(0)?,
                host: row.get(1)?,
                title: row.get(2)?,
                text: row.get(3)?,
                simhash: row.get::<_, i64>(4)? as u64,
                revision: row.get::<_, i64>(5)?.max(0) as u64,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn set_priority(&self, host: &str, urls: &[String]) -> Result<(), IndexError> {
        let bare = bare_host(host);
        if bare.is_empty() {
            return Ok(());
        }
        let picks: Vec<String> = urls
            .iter()
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty())
            .take(100)
            .collect();
        if picks.is_empty() {
            return Ok(());
        }
        self.conn.execute("DELETE FROM host_picks WHERE host = ?1", [&bare])?;
        for url in &picks {
            self.conn.execute(
                "INSERT INTO host_picks (host, url) VALUES (?1, ?2) ON CONFLICT(host, url) DO NOTHING",
                params![bare.as_str(), url.as_str()],
            )?;
        }
        let mut stmt = self.conn.prepare("SELECT url FROM crawl_queue WHERE visited = 0")?;
        let queued = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut drop = Vec::new();
        for url in queued {
            let url = url?;
            let Some(found) = host_of_url(&url) else {
                continue;
            };
            if bare_host(&found) == bare && !picks.iter().any(|pick| pick == &url) {
                drop.push(url);
            }
        }
        for url in drop {
            self.conn.execute("DELETE FROM crawl_queue WHERE url = ?1 AND visited = 0", [url])?;
        }
        for url in &picks {
            let stored: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM pages WHERE url = ?1",
                [url.as_str()],
                |row| row.get(0),
            )?;
            if stored > 0 {
                continue;
            }
            self.conn.execute(
                "INSERT INTO crawl_queue (url, visited, added_at, worth) VALUES (?1, 0, 20, 1)
                 ON CONFLICT(url) DO UPDATE SET visited = 0, added_at = 20, worth = 1",
                [url.as_str()],
            )?;
        }
        Ok(())
    }

    fn has_picks(&self, bare: &str) -> Result<bool, IndexError> {
        let found: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM host_picks WHERE host = ?1",
            [bare],
            |row| row.get(0),
        )?;
        Ok(found > 0)
    }

    fn is_pick(&self, bare: &str, url: &str) -> Result<bool, IndexError> {
        let found: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM host_picks WHERE host = ?1 AND url = ?2",
            params![bare, url],
            |row| row.get(0),
        )?;
        Ok(found > 0)
    }

    fn pick_pressure(&self, bare: &str) -> Result<i64, IndexError> {
        let stored: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pages WHERE url IN (SELECT url FROM host_picks WHERE host = ?1)",
            [bare],
            |row| row.get(0),
        )?;
        let queued: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM crawl_queue WHERE visited = 0 AND url IN (SELECT url FROM host_picks WHERE host = ?1)",
            [bare],
            |row| row.get(0),
        )?;
        Ok(stored + queued)
    }

    pub fn note_webshop(&self, host: &str) -> Result<(), IndexError> {
        let host = bare_host(host);
        if host.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO webshops (host) VALUES (?1) ON CONFLICT(host) DO NOTHING",
            [host],
        )?;
        Ok(())
    }

    fn is_volunteer_host(&self, bare: &str) -> Result<bool, IndexError> {
        let found: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM volunteers WHERE host = ?1 OR host = ?2",
            params![bare, format!("www.{bare}")],
            |row| row.get(0),
        )?;
        Ok(found > 0)
    }

    fn host_pressure(&self, bare: &str) -> Result<i64, IndexError> {
        let www = format!("www.{bare}");
        let pages: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pages WHERE host = ?1 OR host = ?2",
            params![bare, www],
            |row| row.get(0),
        )?;
        let queued: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM crawl_queue WHERE visited = 0 AND (
                url LIKE ?1 OR url LIKE ?2 OR url LIKE ?3 OR url LIKE ?4
                OR url LIKE ?5 OR url LIKE ?6 OR url LIKE ?7 OR url LIKE ?8
            )",
            params![
                format!("https://{bare}/%"),
                format!("http://{bare}/%"),
                format!("https://{www}/%"),
                format!("http://{www}/%"),
                format!("https://{bare}"),
                format!("http://{bare}"),
                format!("https://{www}"),
                format!("http://{www}"),
            ],
            |row| row.get(0),
        )?;
        Ok(pages + queued)
    }

    pub fn enqueue_next(&self, urls: &[String]) -> Result<(), IndexError> {
        for url in urls {
            let trimmed = url.trim();
            if trimmed.is_empty() || is_broken_url(trimmed) {
                continue;
            }
            if let Some(host) = host_of_url(trimmed) {
                if is_press_host(&host) && !is_press_front(trimmed) {
                    continue;
                }
            }
            self.conn
                .execute(
                    "INSERT INTO crawl_queue (url, visited, added_at, lease_until, worth) VALUES (?1, 0, 40, 0, 0)
                     ON CONFLICT(url) DO UPDATE SET visited = 0, added_at = 40, lease_until = 0, worth = 0",
                    params![trimmed],
                )
                .map_err(|source| IndexError::Sqlite {
                    context: "enqueue_next",
                    source,
                })?;
        }
        Ok(())
    }

    pub fn delete_urls(&self, urls: &[String]) -> Result<usize, IndexError> {
        let mut removed = 0_usize;
        for url in urls {
            let trimmed = url.trim();
            if trimmed.is_empty() {
                continue;
            }
            let id: Option<i64> = self
                .conn
                .query_row("SELECT id FROM pages WHERE url = ?1", [trimmed], |row| row.get(0))
                .ok();
            if let Some(id) = id {
                self.delete_id(id)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn scrub_stored_pages(&self) -> Result<usize, IndexError> {
        let rows = {
            let mut stmt = self
                .conn
                .prepare("SELECT id, url, title, text FROM pages")
                .map_err(|source| IndexError::Sqlite {
                    context: "scrub",
                    source,
                })?;
            let mapped = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|source| IndexError::Sqlite {
                    context: "scrub",
                    source,
                })?;
            mapped.collect::<Result<Vec<_>, _>>().map_err(|source| IndexError::Sqlite {
                context: "scrub",
                source,
            })?
        };
        let mut removed = 0_usize;
        for (id, url, title, text) in rows {
            if is_broken_url(&url) || is_not_found(&title, &text) {
                self.delete_id(id)?;
                removed += 1;
                continue;
            }
            let clean = strip_boilerplate(&text);
            if clean != text {
                self.rewrite_text(id, &clean)?;
            }
        }
        let queued = {
            let mut stmt = self.conn.prepare("SELECT url FROM crawl_queue").map_err(|source| {
                IndexError::Sqlite {
                    context: "scrub_queue",
                    source,
                }
            })?;
            let mapped = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|source| IndexError::Sqlite {
                    context: "scrub_queue",
                    source,
                })?;
            mapped.collect::<Result<Vec<_>, _>>().map_err(|source| IndexError::Sqlite {
                context: "scrub_queue",
                source,
            })?
        };
        for url in queued {
            if is_broken_url(&url) {
                self.conn
                    .execute("DELETE FROM crawl_queue WHERE url = ?1", [&url])
                    .map_err(|source| IndexError::Sqlite {
                        context: "scrub_queue",
                        source,
                    })?;
            }
        }
        Ok(removed)
    }

    fn delete_id(&self, id: i64) -> Result<(), IndexError> {
        self.conn
            .execute("DELETE FROM pages_fts WHERE rowid = ?1", [id])
            .map_err(|source| IndexError::Sqlite {
                context: "delete",
                source,
            })?;
        self.conn
            .execute("DELETE FROM pages WHERE id = ?1", [id])
            .map_err(|source| IndexError::Sqlite {
                context: "delete",
                source,
            })?;
        Ok(())
    }

    fn rewrite_text(&self, id: i64, text: &str) -> Result<(), IndexError> {
        self.conn
            .execute("UPDATE pages SET text = ?1 WHERE id = ?2", params![text, id])
            .map_err(|source| IndexError::Sqlite {
                context: "rewrite",
                source,
            })?;
        self.conn
            .execute("DELETE FROM pages_fts WHERE rowid = ?1", [id])
            .map_err(|source| IndexError::Sqlite {
                context: "rewrite",
                source,
            })?;
        let title: String = self
            .conn
            .query_row("SELECT title FROM pages WHERE id = ?1", [id], |row| row.get(0))
            .map_err(|source| IndexError::Sqlite {
                context: "rewrite",
                source,
            })?;
        self.conn
            .execute(
                "INSERT INTO pages_fts(rowid, title, text) VALUES (?1, ?2, ?3)",
                params![id, title, text],
            )
            .map_err(|source| IndexError::Sqlite {
                context: "rewrite",
                source,
            })?;
        Ok(())
    }

    pub fn mark_visited(&self, urls: &[String]) -> Result<(), IndexError> {
        for url in urls {
            let trimmed = url.trim();
            if trimmed.is_empty() {
                continue;
            }
            self.conn.execute(
                "INSERT INTO crawl_queue (url, visited, added_at) VALUES (?1, 1, strftime('%s','now'))
                 ON CONFLICT(url) DO UPDATE SET visited = 1",
                params![trimmed],
            )?;
        }
        Ok(())
    }

    pub fn pending_urls(&self, limit: usize) -> Result<Vec<String>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT url FROM crawl_queue WHERE visited = 0 AND worth < 5 AND lease_until < strftime('%s','now') ORDER BY worth ASC, added_at ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Забирает свободные адреса и держит их, пока получатель не вернёт страницу или срок не выйдет.
    pub fn claim_urls(&self, limit: usize, hold_secs: i64) -> Result<Vec<String>, IndexError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let hold = hold_secs.clamp(1, 14 * 24 * 3600);
        let tx = self.conn.unchecked_transaction().map_err(|source| IndexError::Sqlite {
            context: "lease",
            source,
        })?;
        let urls = {
            let mut stmt = tx.prepare(
                "SELECT url FROM crawl_queue WHERE visited = 0 AND worth < 5 AND lease_until < strftime('%s','now') ORDER BY worth ASC, added_at ASC LIMIT ?1",
            ).map_err(|source| IndexError::Sqlite {
                context: "lease",
                source,
            })?;
            let rows = stmt.query_map(params![limit as i64], |row| row.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };
        for url in &urls {
            tx.execute(
                "UPDATE crawl_queue SET lease_until = strftime('%s','now') + ?1 WHERE url = ?2",
                params![hold, url],
            )?;
        }
        tx.commit().map_err(|source| IndexError::Sqlite {
            context: "lease",
            source,
        })?;
        Ok(urls)
    }

    pub fn release_lease(&self, urls: &[String]) -> Result<(), IndexError> {
        for url in urls {
            let trimmed = url.trim();
            if trimmed.is_empty() {
                continue;
            }
            self.conn.execute(
                "UPDATE crawl_queue SET lease_until = 0 WHERE url = ?1 AND visited = 0",
                [trimmed],
            )?;
        }
        Ok(())
    }

    pub fn leased_count(&self) -> Result<i64, IndexError> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM crawl_queue WHERE visited = 0 AND lease_until >= strftime('%s','now')",
                [],
                |row| row.get(0),
            )
            .map_err(IndexError::from)
    }

    pub fn pages_by_ids(&self, ids: &[i64]) -> Result<Vec<PageRow>, IndexError> {
        let mut out = Vec::new();
        for id in ids {
            let row = self.conn.query_row(
                "SELECT id, url, title, host, text, region, place, lat, lon, kind, media_url, feed_url, price, news_feed, simhash FROM pages WHERE id = ?1",
                [id],
                |row| {
                    Ok(PageRow {
                        id: row.get(0)?,
                        url: row.get(1)?,
                        title: row.get(2)?,
                        host: row.get(3)?,
                        text: row.get(4)?,
                        region: row.get(5)?,
                        place: row.get(6)?,
                        lat: row.get(7)?,
                        lon: row.get(8)?,
                        kind: row.get(9)?,
                        media_url: row.get(10)?,
                        feed_url: row.get(11)?,
                        news_feed: row.get(13)?,
                        price: row.get(12)?,
                        simhash: row.get::<_, i64>(14)? as u64,
                    })
                },
            );
            match row {
                Ok(page) => out.push(page),
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(err) => return Err(err.into()),
            }
        }
        Ok(out)
    }

    pub fn pages_of_host(&self, host: &str, kind_mask: u32, limit: usize) -> Result<Vec<PageRow>, IndexError> {
        let bare = mds_types::bare_host(host);
        let www = format!("www.{bare}");
        let limit = limit.min(100) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT id, url, title, host, text, region, place, lat, lon, kind, media_url, feed_url, price, news_feed, simhash
             FROM pages
             WHERE (host = ?1 OR host = ?2)
               AND ((?3 != 0 AND (kind & ?3) != 0) OR (?3 = 0 AND (kind & ?4) = 0))
             ORDER BY id DESC
             LIMIT ?5",
        )?;
        let rows = stmt.query_map(
            params![bare, www, kind_mask, mds_types::KIND_NEWS | mds_types::KIND_CHANNEL, limit],
            |row| {
                Ok(PageRow {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    title: row.get(2)?,
                    host: row.get(3)?,
                    text: row.get(4)?,
                    region: row.get(5)?,
                    place: row.get(6)?,
                    lat: row.get(7)?,
                    lon: row.get(8)?,
                    kind: row.get(9)?,
                    media_url: row.get(10)?,
                    feed_url: row.get(11)?,
                    news_feed: row.get(13)?,
                    price: row.get(12)?,
                    simhash: row.get::<_, i64>(14)? as u64,
                })
            },
        )?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn has_shop_feed(&self, url: &str) -> Result<bool, IndexError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pages WHERE feed_url = ?1",
            [url],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn has_news_feed(&self, url: &str) -> Result<bool, IndexError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pages WHERE news_feed = ?1",
            [url],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn remember_query(
        &self,
        query: &str,
        results: i64,
        words: i64,
        meaning: i64,
        adult: i64,
    ) -> Result<(), IndexError> {
        self.remember_ranked(query, results, words, meaning, adult, &[])
    }

    pub fn remember_ranked(
        &self,
        query: &str,
        results: i64,
        words: i64,
        meaning: i64,
        adult: i64,
        places: &[RankPlace],
    ) -> Result<(), IndexError> {
        let trimmed = query.trim();
        if trimmed.is_empty() || trimmed.chars().count() > 300 {
            return Ok(());
        }
        let glued = glue_query(trimmed);
        self.conn.execute(
            "INSERT INTO query_log (query, asked_at, result_count, word_hits, meaning_hits, adult_count)
             VALUES (?1, strftime('%s','now'), ?2, ?3, ?4, ?5)",
            params![glued, results, words, meaning, adult],
        )?;
        self.bump_tally(&glued)?;
        let log_id = self.conn.last_insert_rowid();
        for place in places.iter().take(10) {
            self.conn.execute(
                "INSERT INTO query_rank (log_id, place, url, host) VALUES (?1, ?2, ?3, ?4)",
                params![log_id, place.place, place.url, place.host],
            )?;
        }
        self.conn.execute(
            "DELETE FROM query_log WHERE id NOT IN (
                SELECT id FROM query_log ORDER BY id DESC LIMIT 500
             )",
            [],
        )?;
        self.conn.execute(
            "DELETE FROM query_rank WHERE log_id NOT IN (SELECT id FROM query_log)",
            [],
        )?;
        Ok(())
    }

    pub fn key_queries(&self, limit: usize) -> Result<Vec<(String, i64)>, IndexError> {
        self.query_counts("month", limit)
    }

    pub fn query_counts(&self, span: &str, limit: usize) -> Result<Vec<(String, i64)>, IndexError> {
        let start_sql = match span {
            "day" => "date('now','localtime')",
            "week" => "date('now','localtime','-' || ((cast(strftime('%w','now','localtime') as integer) + 6) % 7) || ' days')",
            "month" => "date('now','localtime','start of month')",
            _ => return Ok(Vec::new()),
        };
        let limit = limit.min(50) as i64;
        let mut stmt = self.conn.prepare(&format!(
            "SELECT query, hits FROM query_tally WHERE span = ?1 AND start = {start_sql} ORDER BY hits DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![span, limit], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn add_visit(&self, delta: &VisitDelta) -> Result<(), IndexError> {
        if *delta == VisitDelta::default() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO visit_day (
                day, visits, views, searches, bounces, dwell_seconds, timed_visits,
                empty_searches, search_errors, refused
             ) VALUES (date('now','localtime'), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(day) DO UPDATE SET
                visits = visits + excluded.visits,
                views = views + excluded.views,
                searches = searches + excluded.searches,
                bounces = MAX(0, bounces + excluded.bounces),
                dwell_seconds = dwell_seconds + excluded.dwell_seconds,
                timed_visits = timed_visits + excluded.timed_visits,
                empty_searches = empty_searches + excluded.empty_searches,
                search_errors = search_errors + excluded.search_errors,
                refused = refused + excluded.refused",
            params![
                delta.visits,
                delta.views,
                delta.searches,
                delta.bounces,
                delta.dwell_seconds,
                delta.timed_visits,
                delta.empty_searches,
                delta.search_errors,
                delta.refused
            ],
        )?;
        self.conn.execute(
            "DELETE FROM visit_day WHERE day < date('now','localtime','-400 days')",
            [],
        )?;
        Ok(())
    }

    pub fn visit_span(&self, span: &str) -> Result<VisitReport, IndexError> {
        let start_sql = match span {
            "day" => "date('now','localtime')",
            "week" => "date('now','localtime','-' || ((cast(strftime('%w','now','localtime') as integer) + 6) % 7) || ' days')",
            "month" => "date('now','localtime','start of month')",
            _ => return Ok(VisitReport::default()),
        };
        let sql = format!(
            "SELECT
                COALESCE(SUM(visits), 0),
                COALESCE(SUM(views), 0),
                COALESCE(SUM(searches), 0),
                COALESCE(SUM(bounces), 0),
                COALESCE(SUM(dwell_seconds), 0),
                COALESCE(SUM(timed_visits), 0),
                COALESCE(SUM(empty_searches), 0),
                COALESCE(SUM(search_errors), 0),
                COALESCE(SUM(refused), 0)
             FROM visit_day WHERE day >= {start_sql}"
        );
        self.conn.query_row(&sql, [], |row| {
            Ok(VisitReport {
                visits: row.get(0)?,
                views: row.get(1)?,
                searches: row.get(2)?,
                bounces: row.get(3)?,
                dwell_seconds: row.get(4)?,
                timed_visits: row.get(5)?,
                empty_searches: row.get(6)?,
                search_errors: row.get(7)?,
                refused: row.get(8)?,
            })
        }).map_err(IndexError::from)
    }

    pub fn visit_days(&self, limit: usize) -> Result<Vec<VisitDay>, IndexError> {
        let limit = limit.clamp(1, 60) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT day, visits, views, searches, bounces FROM visit_day ORDER BY day DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(VisitDay {
                day: row.get(0)?,
                visits: row.get(1)?,
                views: row.get(2)?,
                searches: row.get(3)?,
                bounces: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out.reverse();
        Ok(out)
    }

    fn bump_tally(&self, query: &str) -> Result<(), IndexError> {
        for (span, start_sql) in [
            ("day", "date('now','localtime')"),
            (
                "week",
                "date('now','localtime','-' || ((cast(strftime('%w','now','localtime') as integer) + 6) % 7) || ' days')",
            ),
            ("month", "date('now','localtime','start of month')"),
        ] {
            self.conn.execute(
                &format!(
                    "INSERT INTO query_tally (query, span, start, hits) VALUES (?1, ?2, {start_sql}, 1)
                     ON CONFLICT(query, span, start) DO UPDATE SET hits = hits + 1"
                ),
                params![query, span],
            )?;
        }
        self.conn.execute(
            "DELETE FROM query_tally WHERE
                (span = 'day' AND start < date('now','localtime','-400 days'))
                OR (span = 'week' AND start < date('now','localtime','-3 years'))
                OR (span = 'month' AND start < date('now','localtime','-5 years'))",
            [],
        )?;
        Ok(())
    }

    pub fn site_places(&self, host: &str, limit: usize) -> Result<Vec<(String, i64, String)>, IndexError> {
        let limit = limit.min(50) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT l.query, r.place, r.url
             FROM query_rank r JOIN query_log l ON l.id = r.log_id
             WHERE r.host = ?1 ORDER BY r.id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![host, limit], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn list_index(
        &self,
        host: Option<&str>,
        kind: Option<u32>,
        after: i64,
        limit: usize,
    ) -> Result<Vec<IndexLine>, IndexError> {
        let limit = limit.clamp(1, 50) as i64;
        let host = host.unwrap_or("");
        let kind = kind.unwrap_or(0) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT id, url, title, host, kind FROM pages
             WHERE id > ?1
               AND (?2 = '' OR host = ?2)
               AND (?3 = 0 OR (kind & ?3) != 0)
             ORDER BY id LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![after, host, kind, limit], |row| {
            Ok(IndexLine {
                id: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
                host: row.get(3)?,
                kind: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn contacts_for_url(&self, url: &str) -> mds_types::Contacts {
        let mut out = mds_types::Contacts::default();
        if let Ok((title, text, host)) = self.conn.query_row(
            "SELECT title, text, host FROM pages WHERE url = ?1",
            [url],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
        ) {
            out.merge(&mds_types::contacts_in_text(&format!("{title}\n{text}")));
            let bare = mds_types::bare_host(&host);
            if !bare.is_empty() {
                if let Some(hc) = read_host_contacts(&self.conn, &bare) {
                    out.merge(&hc);
                }
            }
        } else if let Some(host) = mds_types::host_of_url(url) {
            let bare = mds_types::bare_host(&host);
            if let Some(hc) = read_host_contacts(&self.conn, &bare) {
                out.merge(&hc);
            }
        }
        out
    }

    pub fn geo_places(
        &self,
        q: &str,
        kind_mask: u32,
        region: Option<&str>,
        place: Option<&str>,
        around: Option<(f64, f64, f64)>,
        bbox: Option<(f64, f64, f64, f64)>,
        limit: usize,
    ) -> Result<(usize, usize, Vec<GeoPlaceRow>), IndexError> {
        let limit = limit.clamp(1, 200);
        let q_trim = q.trim();
        let word_ids: Option< std::collections::HashSet<i64> > = if !q_trim.is_empty() {
            let matches = self.search_words(q_trim, 2000, kind_mask)?;
            if !matches.is_empty() {
                Some(matches.into_iter().map(|m| m.id).collect())
            } else {
                None
            }
        } else {
            None
        };
        let needles: Vec<String> = q_trim
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .filter(|w| !w.is_empty() && !mds_types::is_geo_word(w))
            .collect();

        let mut stmt = self.conn.prepare(
            "SELECT id, url, title, host, text, region, place, lat, lon, kind, price
             FROM pages
             WHERE ((?1 != 0 AND (kind & ?1) != 0) OR (?1 = 0 AND (kind & ?2) = 0))",
        )?;
        let rows = stmt.query_map(
            params![kind_mask, KIND_NEWS | KIND_CHANNEL],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<f64>>(7)?,
                    row.get::<_, Option<f64>>(8)?,
                    row.get::<_, u32>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            },
        )?;

        let mut matched_pages = 0_usize;
        let mut by_host: HashMap<String, GeoPlaceRow> = HashMap::new();

        for row in rows {
            let (id, url, title, host, text, p_region, p_place, lat, lon, kind, price) = row?;
            if let Some(req_place) = place {
                if p_place.as_deref() != Some(req_place) && p_region.as_deref() != Some(req_place) {
                    continue;
                }
            } else if let Some(req_reg) = region {
                if p_region.as_deref() != Some(req_reg) {
                    continue;
                }
            }
            if let Some((min_lat, max_lat, min_lon, max_lon)) = bbox {
                let (Some(la), Some(lo)) = (lat, lon) else {
                    continue;
                };
                if la < min_lat || la > max_lat || lo < min_lon || lo > max_lon {
                    continue;
                }
            }
            let mut dist_km: Option<f32> = None;
            if let Some((ulat, ulon, rad_km)) = around {
                let (Some(la), Some(lo)) = (lat, lon) else {
                    continue;
                };
                let d = mds_types::haversine_km(ulat, ulon, la, lo);
                if d > rad_km {
                    continue;
                }
                dist_km = Some(d as f32);
            }
            if !q_trim.is_empty() {
                let in_fts = word_ids.as_ref().map(|set| set.contains(&id)).unwrap_or(false);
                if !in_fts && !needles.is_empty() {
                    let hay = format!("{title}\n{text}").to_lowercase();
                    if !needles.iter().all(|w| hay.contains(w)) {
                        continue;
                    }
                }
            }

            matched_pages += 1;
            let bare = mds_types::bare_host(&host);
            let key = if bare.is_empty() || is_marketplace_host(&bare) || is_channel_host(&bare) {
                url.clone()
            } else {
                bare.clone()
            };

            let mut contacts = mds_types::contacts_in_text(&format!("{title}\n{text}"));
            if !bare.is_empty() {
                if let Some(hc) = read_host_contacts(&self.conn, &bare) {
                    contacts.merge(&hc);
                }
            }
            let snip = mds_types::snippet(&text, if q_trim.is_empty() { &title } else { q_trim });

            by_host
                .entry(key)
                .and_modify(|existing| {
                    existing.contacts.merge(&contacts);
                    if existing.lat.is_none() && lat.is_some() {
                        existing.lat = lat;
                        existing.lon = lon;
                        existing.distance_km = dist_km;
                    } else if let (Some(d_new), Some(d_old)) = (dist_km, existing.distance_km) {
                        if d_new < d_old {
                            existing.distance_km = Some(d_new);
                            existing.lat = lat;
                            existing.lon = lon;
                        }
                    }
                    if existing.place.is_none() && p_place.is_some() {
                        existing.place = p_place.clone();
                    }
                    if existing.region.is_none() && p_region.is_some() {
                        existing.region = p_region.clone();
                    }
                    existing.kind |= kind;
                })
                .or_insert(GeoPlaceRow {
                    url,
                    host: bare,
                    title,
                    snippet: snip,
                    region: p_region,
                    place: p_place,
                    lat,
                    lon,
                    distance_km: dist_km,
                    kind,
                    price,
                    contacts,
                });
        }

        let total_sites = by_host.len();
        let mut list: Vec<GeoPlaceRow> = by_host.into_values().collect();
        list.sort_by(|a, b| match (a.distance_km, b.distance_km) {
            (Some(da), Some(db)) => da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.host.cmp(&b.host),
        });
        list.truncate(limit);
        Ok((total_sites, matched_pages, list))
    }

    pub fn kind_counts(&self) -> Result<Vec<(String, String, String, i64)>, IndexError> {
        let mut out = Vec::new();
        for spec in all_kinds() {
            let count: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM pages WHERE (kind & ?1) != 0",
                [spec.mask as i64],
                |row| row.get(0),
            )?;
            out.push((
                spec.code.to_string(),
                spec.label.to_string(),
                spec.place.to_string(),
                count,
            ));
        }
        Ok(out)
    }

    pub fn recent_queries(&self, limit: usize) -> Result<Vec<QueryStat>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT query, result_count, word_hits, meaning_hits, adult_count
             FROM query_log ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(QueryStat {
                text: row.get(0)?,
                results: row.get(1)?,
                words: row.get(2)?,
                meaning: row.get(3)?,
                adult: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn queue_len(&self) -> Result<usize, IndexError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM crawl_queue WHERE visited = 0",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn bucket_report(&self) -> Result<(Vec<u16>, bool), IndexError> {
        let distinct: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT semantic_bucket) FROM pages",
            [],
            |row| row.get(0),
        )?;
        if distinct > 512 {
            return Ok((Vec::new(), true));
        }
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT semantic_bucket FROM pages")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row? as u16);
        }
        Ok((out, false))
    }

    pub fn queries_last_minute(&self) -> Result<usize, IndexError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM query_log WHERE asked_at >= strftime('%s','now') - 60",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn recent_pages(&self, limit: usize) -> Result<Vec<(String, String, String, String)>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT url, title, host, text FROM pages ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn remember_volunteer(&self, host: &str) -> Result<(), IndexError> {
        let host = host.trim().to_ascii_lowercase();
        if host.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO volunteers (host, seen_at) VALUES (?1, strftime('%s','now'))
             ON CONFLICT(host) DO UPDATE SET seen_at = excluded.seen_at",
            [host],
        )?;
        Ok(())
    }

    pub fn volunteers(&self) -> Result<Vec<String>, IndexError> {
        let mut stmt = self.conn.prepare("SELECT host FROM volunteers ORDER BY host")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn forget_volunteer(&self, host: &str) -> Result<(), IndexError> {
        let host = bare_host(host);
        self.conn.execute("DELETE FROM volunteers WHERE host = ?1", [host])?;
        Ok(())
    }

    pub fn note_shell(&self, id: &str) -> Result<(), IndexError> {
        let id = id.trim();
        if id.len() != 32 {
            return Ok(());
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO shell_seen (day, id) VALUES (date('now','localtime'), ?1)",
            [id],
        )?;
        self.conn.execute(
            "DELETE FROM shell_seen WHERE day < date('now','localtime','-400 days')",
            [],
        )?;
        Ok(())
    }

    pub fn note_shell_pages(&self, pages: i64) -> Result<(), IndexError> {
        if pages <= 0 {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO shell_pages (day, pages) VALUES (date('now','localtime'), ?1)
             ON CONFLICT(day) DO UPDATE SET pages = pages + excluded.pages",
            [pages],
        )?;
        Ok(())
    }

    pub fn shell_span(&self, span: &str) -> Result<(i64, i64), IndexError> {
        let filter = span_filter(span);
        let apps: i64 = self.conn.query_row(
            &format!("SELECT COUNT(DISTINCT id) FROM shell_seen WHERE {filter}"),
            [],
            |row| row.get(0),
        )?;
        let pages: i64 = self.conn.query_row(
            &format!("SELECT COALESCE(SUM(pages), 0) FROM shell_pages WHERE {filter}"),
            [],
            |row| row.get(0),
        )?;
        Ok((apps, pages))
    }

    pub fn shell_pace(&self) -> Result<ShellPace, IndexError> {
        Ok(ShellPace {
            batch: clamp_meta(self.meta_get("shell_batch")?, 2, 8, 2),
            gap_min: clamp_meta(self.meta_get("shell_gap_min")?, 2, 10, 10),
            window_max: clamp_meta(self.meta_get("shell_window")?, 4, 16, 4),
        })
    }

    pub fn set_shell_pace(&self, batch: i64, gap_min: i64, window_max: i64) -> Result<(), IndexError> {
        self.meta_set("shell_batch", &clamp_meta(Some(batch.to_string()), 2, 8, 2).to_string())?;
        self.meta_set("shell_gap_min", &clamp_meta(Some(gap_min.to_string()), 2, 10, 10).to_string())?;
        self.meta_set("shell_window", &clamp_meta(Some(window_max.to_string()), 4, 16, 4).to_string())?;
        Ok(())
    }

    pub fn note_bee_pages(&self, host: &str, pages: i64) -> Result<(), IndexError> {
        let host = bare_host(host);
        if host.is_empty() || pages <= 0 {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO bee_day (host, day, pages) VALUES (?1, date('now','localtime'), ?2)
             ON CONFLICT(host, day) DO UPDATE SET pages = pages + excluded.pages",
            params![host, pages],
        )?;
        Ok(())
    }

    pub fn bee_totals(&self, host: &str) -> Result<BeeTotals, IndexError> {
        let host = bare_host(host);
        let sum = |filter: &str| -> Result<i64, IndexError> {
            self.conn.query_row(
                &format!("SELECT COALESCE(SUM(pages), 0) FROM bee_day WHERE host = ?1 AND {filter}"),
                [&host],
                |row| row.get(0),
            ).map_err(IndexError::from)
        };
        let last: Option<String> = self.conn.query_row(
            "SELECT MAX(day) FROM bee_day WHERE host = ?1 AND pages > 0",
            [&host],
            |row| row.get(0),
        )?;
        let yesterday: String = self.conn.query_row(
            "SELECT date('now','localtime','-1 day')",
            [],
            |row| row.get(0),
        )?;
        let quiet = last.as_ref().map(|day| day.as_str() < yesterday.as_str()).unwrap_or(true);
        Ok(BeeTotals {
            day: sum("day = date('now','localtime')")?,
            week: sum(&span_filter("week"))?,
            month: sum(&span_filter("month"))?,
            all: sum("1 = 1")?,
            quiet,
        })
    }

    pub fn stored_simhash(&self, url: &str) -> Result<Option<u64>, IndexError> {
        match self.conn.query_row(
            "SELECT simhash FROM pages WHERE url = ?1",
            [url],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(value) => Ok(Some(value as u64)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn host_pages(&self, host: &str) -> Result<i64, IndexError> {
        let host = bare_host(host);
        self.conn
            .query_row("SELECT COUNT(*) FROM pages WHERE host = ?1", [host], |row| row.get(0))
            .map_err(IndexError::from)
    }

    pub fn note_intake(&self, new_pages: i64, changed_pages: i64, same_pages: i64, new_hosts: i64) -> Result<(), IndexError> {
        if new_pages == 0 && changed_pages == 0 && same_pages == 0 && new_hosts == 0 {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO intake_day (day, new_pages, changed_pages, same_pages, new_hosts)
             VALUES (date('now','localtime'), ?1, ?2, ?3, ?4)
             ON CONFLICT(day) DO UPDATE SET
                new_pages = new_pages + excluded.new_pages,
                changed_pages = changed_pages + excluded.changed_pages,
                same_pages = same_pages + excluded.same_pages,
                new_hosts = new_hosts + excluded.new_hosts",
            params![new_pages, changed_pages, same_pages, new_hosts],
        )?;
        self.conn.execute(
            "DELETE FROM intake_day WHERE day < date('now','localtime','-400 days')",
            [],
        )?;
        Ok(())
    }

    pub fn intake_days(&self, limit: usize) -> Result<Vec<IntakeDay>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT day, new_pages, changed_pages, same_pages, new_hosts
             FROM intake_day ORDER BY day DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(IntakeDay {
                day: row.get(0)?,
                new_pages: row.get(1)?,
                changed_pages: row.get(2)?,
                same_pages: row.get(3)?,
                new_hosts: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out.reverse();
        Ok(out)
    }

    pub fn intake_sum(&self, column: &str, span: &str) -> Result<i64, IndexError> {
        let column = match column {
            "new_pages" | "changed_pages" | "same_pages" | "new_hosts" => column,
            _ => "new_pages",
        };
        self.conn
            .query_row(
                &format!(
                    "SELECT COALESCE(SUM({column}), 0) FROM intake_day WHERE {}",
                    span_filter(span)
                ),
                [],
                |row| row.get(0),
            )
            .map_err(IndexError::from)
    }

    pub fn note_load(&self, cpu_pct: i64, ram_pct: i64) -> Result<(), IndexError> {
        let cpu_pct = cpu_pct.clamp(0, 100);
        let ram_pct = ram_pct.clamp(0, 100);
        self.conn.execute(
            "INSERT INTO load_hour (hour, cpu_pct, ram_pct, samples)
             VALUES (strftime('%Y-%m-%d %H','now','localtime'), ?1, ?2, 1)
             ON CONFLICT(hour) DO UPDATE SET
                cpu_pct = (cpu_pct * samples + excluded.cpu_pct) / (samples + 1),
                ram_pct = (ram_pct * samples + excluded.ram_pct) / (samples + 1),
                samples = samples + 1",
            params![cpu_pct, ram_pct],
        )?;
        self.conn.execute(
            "DELETE FROM load_hour WHERE hour < strftime('%Y-%m-%d %H','now','localtime','-14 days')",
            [],
        )?;
        Ok(())
    }

    pub fn load_hours(&self, limit: usize) -> Result<Vec<LoadHour>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT hour, cpu_pct, ram_pct FROM load_hour ORDER BY hour DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(LoadHour {
                hour: row.get(0)?,
                cpu_pct: row.get(1)?,
                ram_pct: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out.reverse();
        Ok(out)
    }

    pub fn crawl_pace(&self) -> Result<CrawlPace, IndexError> {
        Ok(CrawlPace {
            batch: clamp_meta(self.meta_get("crawl_batch")?, 1, 24, 4),
            gap_secs: clamp_meta(self.meta_get("crawl_gap")?, 2, 300, 10),
        })
    }

    pub fn set_crawl_pace(&self, batch: i64, gap_secs: i64) -> Result<(), IndexError> {
        self.meta_set("crawl_batch", &clamp_meta(Some(batch.to_string()), 1, 24, 4).to_string())?;
        self.meta_set("crawl_gap", &clamp_meta(Some(gap_secs.to_string()), 2, 300, 10).to_string())?;
        Ok(())
    }

    pub fn bee_pace(&self) -> Result<BeePace, IndexError> {
        Ok(BeePace {
            batch: clamp_meta(self.meta_get("bee_batch")?, 1, 24, 4),
            gap_secs: clamp_meta(self.meta_get("bee_gap")?, 60, 3600, 600),
        })
    }

    pub fn set_bee_pace(&self, batch: i64, gap_secs: i64) -> Result<(), IndexError> {
        self.meta_set("bee_batch", &clamp_meta(Some(batch.to_string()), 1, 24, 4).to_string())?;
        self.meta_set("bee_gap", &clamp_meta(Some(gap_secs.to_string()), 60, 3600, 600).to_string())?;
        Ok(())
    }

    pub fn shell_ceiling(&self) -> Result<ShellCeiling, IndexError> {
        Ok(ShellCeiling {
            max_batch: clamp_meta(self.meta_get("shell_max_batch")?, 2, 30, 8),
            min_gap: clamp_meta(self.meta_get("shell_min_gap")?, 1, 60, 1),
        })
    }

    pub fn set_shell_ceiling(&self, max_batch: i64, min_gap: i64) -> Result<(), IndexError> {
        self.meta_set(
            "shell_max_batch",
            &clamp_meta(Some(max_batch.to_string()), 2, 30, 8).to_string(),
        )?;
        self.meta_set(
            "shell_min_gap",
            &clamp_meta(Some(min_gap.to_string()), 1, 60, 1).to_string(),
        )?;
        Ok(())
    }

    pub fn shell_grants(&self) -> Result<i64, IndexError> {
        Ok(clamp_meta(self.meta_get("shell_grants")?, 4, 600, 60))
    }

    pub fn set_shell_grants(&self, grants: i64) -> Result<(), IndexError> {
        self.meta_set(
            "shell_grants",
            &clamp_meta(Some(grants.to_string()), 4, 600, 60).to_string(),
        )?;
        Ok(())
    }

    pub fn refresh_pace(&self) -> Result<RefreshPace, IndexError> {
        Ok(RefreshPace {
            batch: clamp_meta(self.meta_get("refresh_batch")?, 1, 4, 1),
            gap_secs: clamp_meta(self.meta_get("refresh_gap")?, 60, 3600, 120),
        })
    }

    pub fn set_refresh_pace(&self, batch: i64, gap_secs: i64) -> Result<(), IndexError> {
        self.meta_set("refresh_batch", &clamp_meta(Some(batch.to_string()), 1, 4, 1).to_string())?;
        self.meta_set("refresh_gap", &clamp_meta(Some(gap_secs.to_string()), 60, 3600, 120).to_string())?;
        Ok(())
    }

    pub fn refresh_due(&self) -> Result<i64, IndexError> {
        self.conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM pages WHERE next_check <= strftime('%s','now') AND {NOT_VOLUNTEER}"
                ),
                [],
                |row| row.get(0),
            )
            .map_err(IndexError::from)
    }

    pub fn claim_refresh(&self, limit: usize) -> Result<Vec<RefreshJob>, IndexError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let tx = self.conn.unchecked_transaction().map_err(|source| IndexError::Sqlite {
            context: "refresh",
            source,
        })?;
        let jobs = {
            let mut stmt = tx
                .prepare(&format!(
                    "SELECT url, etag, modified FROM pages
                     WHERE next_check <= strftime('%s','now') AND {NOT_VOLUNTEER}
                     ORDER BY next_check ASC, id ASC LIMIT ?1"
                ))
                .map_err(|source| IndexError::Sqlite {
                    context: "refresh",
                    source,
                })?;
            let rows = stmt.query_map(params![limit as i64], |row| {
                Ok(RefreshJob {
                    url: row.get(0)?,
                    etag: row.get(1)?,
                    modified: row.get(2)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };
        for job in &jobs {
            tx.execute(
                "UPDATE pages SET next_check = strftime('%s','now') + 3600 WHERE url = ?1",
                [&job.url],
            )?;
        }
        tx.commit().map_err(|source| IndexError::Sqlite {
            context: "refresh",
            source,
        })?;
        Ok(jobs)
    }

    pub fn refresh_quiet(&self, url: &str, etag: Option<&str>, modified: Option<&str>) -> Result<(), IndexError> {
        let delay = self.bump_refresh_gap(url, false)?;
        self.write_refresh(url, etag, modified, delay)
    }

    pub fn refresh_changed(&self, url: &str, etag: Option<&str>, modified: Option<&str>) -> Result<(), IndexError> {
        let delay = self.bump_refresh_gap(url, true)?;
        self.write_refresh(url, etag, modified, delay)
    }

    pub fn refresh_retry(&self, url: &str) -> Result<(), IndexError> {
        self.write_refresh(url, None, None, 3600)
    }

    pub fn refresh_after(&self, url: &str, secs: i64) -> Result<(), IndexError> {
        self.write_refresh(url, None, None, secs.clamp(60, 14 * 86_400))
    }

    fn bump_refresh_gap(&self, url: &str, reset: bool) -> Result<i64, IndexError> {
        let delay: i64 = if reset {
            86_400
        } else {
            match self.conn.query_row("SELECT secs FROM refresh_gap WHERE url = ?1", [url], |row| row.get(0)) {
                Ok(secs) => secs,
                Err(rusqlite::Error::QueryReturnedNoRows) => 86_400,
                Err(err) => return Err(err.into()),
            }
        };
        let stored = if reset {
            86_400
        } else {
            delay.saturating_mul(2).min(14 * 86_400)
        };
        self.conn.execute(
            "INSERT INTO refresh_gap (url, secs) VALUES (?1, ?2)
             ON CONFLICT(url) DO UPDATE SET secs = excluded.secs",
            params![url, stored],
        )?;
        Ok(delay)
    }

    fn write_refresh(&self, url: &str, etag: Option<&str>, modified: Option<&str>, delay: i64) -> Result<(), IndexError> {
        self.conn.execute(
            "UPDATE pages SET
                etag = CASE WHEN ?2 IS NULL THEN etag ELSE ?2 END,
                modified = CASE WHEN ?3 IS NULL THEN modified ELSE ?3 END,
                next_check = strftime('%s','now') + ?4
             WHERE url = ?1",
            params![url, etag, modified, delay],
        )?;
        Ok(())
    }

    fn seed_refresh(&self) -> Result<(), IndexError> {
        if self.meta_get("refresh_seeded")?.is_some() {
            return Ok(());
        }
        self.conn
            .execute(
                "UPDATE pages SET next_check = strftime('%s','now') + (id % 604800) WHERE next_check = 0",
                [],
            )
            .map_err(|source| IndexError::Sqlite {
                context: "refresh",
                source,
            })?;
        self.meta_set("refresh_seeded", "1")
    }

    #[cfg(test)]
    fn next_check_of(&self, url: &str) -> Result<i64, IndexError> {
        self.conn
            .query_row("SELECT next_check FROM pages WHERE url = ?1", [url], |row| row.get(0))
            .map_err(IndexError::from)
    }

    #[cfg(test)]
    fn set_next_check(&self, url: &str, when: i64) -> Result<(), IndexError> {
        self.conn
            .execute("UPDATE pages SET next_check = ?1 WHERE url = ?2", params![when, url])?;
        Ok(())
    }

    pub fn remember_node(&self, base: &str) -> Result<(), IndexError> {
        let base = base.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO known_nodes (base, added_at) VALUES (?1, strftime('%s','now'))
             ON CONFLICT(base) DO NOTHING",
            [base],
        )?;
        Ok(())
    }

    pub fn forget_node(&self, base: &str) -> Result<(), IndexError> {
        self.conn.execute(
            "DELETE FROM known_nodes WHERE base = ?1",
            [base.trim().trim_end_matches('/')],
        )?;
        Ok(())
    }

    pub fn known_nodes(&self) -> Result<Vec<KnownNode>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT base, alive, latency_ms, last_ok FROM known_nodes ORDER BY base",
        )?;
        let rows = stmt.query_map([], |row| {
            let last: Option<i64> = row.get(3)?;
            Ok(KnownNode {
                base: row.get(0)?,
                alive: row.get::<_, i64>(1)? != 0,
                latency_ms: row.get::<_, i64>(2)? as u64,
                last_ok: last,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn touch_node(&self, base: &str, alive: bool, latency_ms: u64) -> Result<(), IndexError> {
        let base = base.trim().trim_end_matches('/');
        self.conn.execute(
            "UPDATE known_nodes SET alive = ?2, latency_ms = ?3, last_ok = CASE WHEN ?2 = 1 THEN strftime('%s','now') ELSE last_ok END WHERE base = ?1",
            params![base, if alive { 1 } else { 0 }, latency_ms as i64],
        )?;
        Ok(())
    }

    fn meta_get(&self, key: &str) -> Result<Option<String>, IndexError> {
        self.conn
            .query_row("SELECT val FROM index_meta WHERE key = ?1", [key], |row| row.get(0))
            .or_else(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other.into()),
            })
    }

    fn meta_set(&self, key: &str, val: &str) -> Result<(), IndexError> {
        self.conn.execute(
            "INSERT INTO index_meta (key, val) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET val = excluded.val",
            params![key, val],
        )?;
        Ok(())
    }

    pub fn refuse_host(&self, host: &str) -> Result<(), IndexError> {
        let host = bare_host(host);
        if host.is_empty() {
            return Ok(());
        }
        self.conn.execute("INSERT OR IGNORE INTO refused_hosts (host) VALUES (?1)", [host])?;
        Ok(())
    }

    pub fn restore_host(&self, host: &str) -> Result<(), IndexError> {
        let host = bare_host(host);
        self.conn.execute("DELETE FROM refused_hosts WHERE host = ?1", [host])?;
        Ok(())
    }

    pub fn refused_hosts(&self) -> Result<Vec<String>, IndexError> {
        let mut stmt = self.conn.prepare("SELECT host FROM refused_hosts ORDER BY host")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn mute_host(&self, host: &str) -> Result<(), IndexError> {
        let host = bare_host(host);
        if host.is_empty() {
            return Ok(());
        }
        self.conn.execute("INSERT OR IGNORE INTO muted_hosts (host) VALUES (?1)", [host])?;
        Ok(())
    }

    pub fn unmute_host(&self, host: &str) -> Result<(), IndexError> {
        let host = bare_host(host);
        self.conn.execute("DELETE FROM muted_hosts WHERE host = ?1", [host])?;
        Ok(())
    }

    pub fn muted_hosts(&self) -> Result<Vec<String>, IndexError> {
        let mut stmt = self.conn.prepare("SELECT host FROM muted_hosts ORDER BY host")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisitDelta {
    pub visits: i64,
    pub views: i64,
    pub searches: i64,
    pub bounces: i64,
    pub dwell_seconds: i64,
    pub timed_visits: i64,
    pub empty_searches: i64,
    pub search_errors: i64,
    pub refused: i64,
}

impl VisitDelta {
    pub fn merge(&mut self, extra: VisitDelta) {
        self.visits += extra.visits;
        self.views += extra.views;
        self.searches += extra.searches;
        self.bounces += extra.bounces;
        self.dwell_seconds += extra.dwell_seconds;
        self.timed_visits += extra.timed_visits;
        self.empty_searches += extra.empty_searches;
        self.search_errors += extra.search_errors;
        self.refused += extra.refused;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisitDay {
    pub day: String,
    pub visits: i64,
    pub views: i64,
    pub searches: i64,
    pub bounces: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisitReport {
    pub visits: i64,
    pub views: i64,
    pub searches: i64,
    pub bounces: i64,
    pub dwell_seconds: i64,
    pub timed_visits: i64,
    pub empty_searches: i64,
    pub search_errors: i64,
    pub refused: i64,
}

#[derive(Clone, Debug)]
pub struct RankPlace {
    pub place: i64,
    pub url: String,
    pub host: String,
}

#[derive(Clone, Debug)]
pub struct IndexLine {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub host: String,
    pub kind: i64,
}

#[derive(Clone, Debug)]
pub struct GeoPlaceRow {
    pub url: String,
    pub host: String,
    pub title: String,
    pub snippet: String,
    pub region: Option<String>,
    pub place: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub distance_km: Option<f32>,
    pub kind: u32,
    pub price: Option<String>,
    pub contacts: mds_types::Contacts,
}

fn upsert_host_contacts(conn: &Connection, bare: &str, fresh: &mds_types::Contacts) -> Result<(), rusqlite::Error> {
    let mut merged = read_host_contacts(conn, bare).unwrap_or_default();
    merged.merge(fresh);
    conn.execute(
        "INSERT INTO host_contacts (host, phones, emails, messengers, socials)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(host) DO UPDATE SET
            phones = excluded.phones,
            emails = excluded.emails,
            messengers = excluded.messengers,
            socials = excluded.socials",
        params![
            bare,
            merged.phones.join(","),
            merged.emails.join(","),
            merged.messengers.join(","),
            merged.socials.join(","),
        ],
    )?;
    Ok(())
}

fn read_host_contacts(conn: &Connection, bare: &str) -> Option<mds_types::Contacts> {
    let (phones, emails, messengers, socials): (String, String, String, String) = conn
        .query_row(
            "SELECT phones, emails, messengers, socials FROM host_contacts WHERE host = ?1",
            [bare],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok()?;
    let split = |s: String| -> Vec<String> {
        s.split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect()
    };
    Some(mds_types::Contacts {
        phones: split(phones),
        emails: split(emails),
        messengers: split(messengers),
        socials: split(socials),
    })
}

pub struct QueryStat {
    pub text: String,
    pub results: i64,
    pub words: i64,
    pub meaning: i64,
    pub adult: i64,
}

fn glue_query(query: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for ch in query.trim().chars() {
        if ch.is_whitespace() {
            gap = true;
            continue;
        }
        if gap && !out.is_empty() {
            out.push(' ');
        }
        gap = false;
        out.push(ch);
    }
    out
}

fn ensure_region_column(conn: &Connection) -> Result<(), IndexError> {
    for (name, ddl) in [
        ("region", "ALTER TABLE pages ADD COLUMN region TEXT"),
        ("place", "ALTER TABLE pages ADD COLUMN place TEXT"),
        ("lat", "ALTER TABLE pages ADD COLUMN lat REAL"),
        ("lon", "ALTER TABLE pages ADD COLUMN lon REAL"),
    ] {
        if column_exists(conn, name)? {
            continue;
        }
        conn.execute(ddl, []).map_err(|source| IndexError::Sqlite {
            context: "region",
            source,
        })?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS host_geo (
            host TEXT PRIMARY KEY,
            lat REAL NOT NULL,
            lon REAL NOT NULL
        );
        CREATE TABLE IF NOT EXISTS host_contacts (
            host TEXT PRIMARY KEY,
            phones TEXT NOT NULL DEFAULT '',
            emails TEXT NOT NULL DEFAULT '',
            messengers TEXT NOT NULL DEFAULT '',
            socials TEXT NOT NULL DEFAULT ''
        );",
    )
    .map_err(|source| IndexError::Sqlite {
        context: "host_geo",
        source,
    })?;
    Ok(())
}

fn ensure_kind_columns(conn: &Connection) -> Result<(), IndexError> {
    for (name, ddl) in [
        ("kind", "ALTER TABLE pages ADD COLUMN kind INTEGER NOT NULL DEFAULT 0"),
        ("media_url", "ALTER TABLE pages ADD COLUMN media_url TEXT"),
        ("feed_url", "ALTER TABLE pages ADD COLUMN feed_url TEXT"),
        ("price", "ALTER TABLE pages ADD COLUMN price TEXT"),
        ("news_feed", "ALTER TABLE pages ADD COLUMN news_feed TEXT"),
    ] {
        if column_exists(conn, name)? {
            continue;
        }
        conn.execute(ddl, []).map_err(|source| IndexError::Sqlite {
            context: "kind",
            source,
        })?;
    }
    conn.execute("CREATE INDEX IF NOT EXISTS idx_pages_host ON pages(host)", [])
        .map_err(|source| IndexError::Sqlite {
            context: "kind",
            source,
        })?;
    Ok(())
}

const NOT_VOLUNTEER: &str = "NOT EXISTS (
    SELECT 1 FROM volunteers
    WHERE volunteers.host = pages.host
       OR volunteers.host = 'www.' || pages.host
       OR pages.host = 'www.' || volunteers.host
)";

fn column_exists(conn: &Connection, name: &str) -> Result<bool, IndexError> {
    table_column_exists(conn, "pages", name)
}

fn table_column_exists(conn: &Connection, table: &str, name: &str) -> Result<bool, IndexError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|source| IndexError::Sqlite {
            context: "kind",
            source,
        })?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|source| IndexError::Sqlite {
            context: "kind",
            source,
        })?;
    for found in names {
        if found? == name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn ensure_queue_worth(conn: &Connection) -> Result<(), IndexError> {
    if table_column_exists(conn, "crawl_queue", "worth")? {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE crawl_queue ADD COLUMN worth INTEGER NOT NULL DEFAULT 3",
        [],
    )
    .map_err(|source| IndexError::Sqlite {
        context: "worth",
        source,
    })?;
    Ok(())
}

fn ensure_anchor_table(conn: &Connection) -> Result<(), IndexError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS anchor_text (
            url TEXT PRIMARY KEY,
            words TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS host_quiet (
            host TEXT PRIMARY KEY
        );",
    )
    .map_err(|source| IndexError::Sqlite {
        context: "anchor",
        source,
    })?;
    Ok(())
}

fn ensure_revision(conn: &Connection) -> Result<(), IndexError> {
    if column_exists(conn, "revision")? {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE pages ADD COLUMN revision INTEGER NOT NULL DEFAULT 0",
        [],
    )
    .map_err(|source| IndexError::Sqlite {
        context: "revision",
        source,
    })?;
    Ok(())
}

fn next_revision(conn: &Connection) -> Result<i64, IndexError> {
    let current: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(CAST(val AS INTEGER)), 0) FROM index_meta WHERE key = 'index_revision'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let next = current + 1;
    conn.execute(
        "INSERT INTO index_meta (key, val) VALUES ('index_revision', ?1)
         ON CONFLICT(key) DO UPDATE SET val = excluded.val",
        [next.to_string()],
    )?;
    Ok(next)
}

fn anchor_words_in(conn: &Connection, url: &str) -> Result<String, IndexError> {
    let words: String = conn
        .query_row(
            "SELECT words FROM anchor_text WHERE url = ?1",
            [url],
            |row| row.get(0),
        )
        .unwrap_or_default();
    Ok(words)
}

fn join_anchors(text: &str, anchors: &str) -> String {
    if anchors.trim().is_empty() {
        text.to_string()
    } else {
        format!("{text}\n{anchors}")
    }
}

fn ensure_queue_lease(conn: &Connection) -> Result<(), IndexError> {
    if table_column_exists(conn, "crawl_queue", "lease_until")? {
        return Ok(());
    }
    conn.execute("ALTER TABLE crawl_queue ADD COLUMN lease_until INTEGER NOT NULL DEFAULT 0", [])
        .map_err(|source| IndexError::Sqlite {
            context: "lease",
            source,
        })?;
    Ok(())
}

fn ensure_refresh_columns(conn: &Connection) -> Result<(), IndexError> {
    for (name, ddl) in [
        ("etag", "ALTER TABLE pages ADD COLUMN etag TEXT"),
        ("modified", "ALTER TABLE pages ADD COLUMN modified TEXT"),
        ("next_check", "ALTER TABLE pages ADD COLUMN next_check INTEGER NOT NULL DEFAULT 0"),
    ] {
        if column_exists(conn, name)? {
            continue;
        }
        conn.execute(ddl, []).map_err(|source| IndexError::Sqlite {
            context: "refresh",
            source,
        })?;
    }
    conn.execute(
        "CREATE TABLE IF NOT EXISTS refresh_gap (url TEXT PRIMARY KEY, secs INTEGER NOT NULL)",
        [],
    )
    .map_err(|source| IndexError::Sqlite {
        context: "refresh",
        source,
    })?;
    Ok(())
}

impl Index {
    fn fill_kinds(&self) -> Result<usize, IndexError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, url, title, text, kind FROM pages")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        let mut labeled = 0_usize;
        for row in rows {
            let (id, url, title, text, stored) = row?;
            let stored = stored as u32;
            let host = host_of_url(&url).unwrap_or_default();
            if is_press_host(&host) && !is_press_front(&url) {
                self.delete_id(id)?;
                labeled += 1;
                continue;
            }
            let fresh = kind_from_stored(&url, &title, &text);
            let kind = if is_channel_host(&host) {
                (stored | KIND_VIDEO | KIND_CHANNEL) & !KIND_NEWS
            } else if is_press_host(&host) {
                KIND_NEWS
            } else {
                stored | fresh
            };
            if kind == stored || (stored == 0 && kind == 0) {
                continue;
            }
            self.conn.execute("UPDATE pages SET kind = ?1 WHERE id = ?2", params![kind, id])?;
            labeled += 1;
        }
        drop(stmt);
        let queued = {
            let mut waiting = self.conn.prepare("SELECT url FROM crawl_queue WHERE visited = 0")?;
            let mapped = waiting.query_map([], |row| row.get::<_, String>(0))?;
            mapped.collect::<Result<Vec<_>, _>>()?
        };
        for url in queued {
            let host = host_of_url(&url).unwrap_or_default();
            if is_press_host(&host) && !is_press_front(&url) {
                self.conn.execute("DELETE FROM crawl_queue WHERE url = ?1", [&url])?;
            }
        }
        Ok(labeled)
    }

    pub fn accepts_more(&self, host: &str) -> Result<bool, IndexError> {
        let bare = bare_host(host);
        if self.is_volunteer_host(&bare)? {
            return Ok(true);
        }
        Ok(self.host_pressure(&bare)? < HOST_PAGE_CAP)
    }

    pub fn may_store(&self, host: &str, url: &str) -> Result<bool, IndexError> {
        let bare = bare_host(host);
        if is_press_host(&bare) && !is_press_front(url) {
            return Ok(false);
        }
        if is_marketplace_host(&bare) && is_product_card(url) {
            return Ok(false);
        }
        let exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pages WHERE url = ?1",
            [url],
            |row| row.get(0),
        )?;
        if exists > 0 {
            return Ok(true);
        }
        if self.is_volunteer_host(&bare)? {
            return Ok(true);
        }
        if self.has_picks(&bare)? {
            if !self.is_pick(&bare, url)? {
                return Ok(false);
            }
            return Ok(self.pick_pressure(&bare)? < HOST_PAGE_CAP);
        }
        self.accepts_more(host)
    }
}

fn init_schema(conn: &Connection) -> Result<(), IndexError> {
    const STEPS: &[(&str, &str)] = &[
        (
            "journal",
            "PRAGMA journal_mode = WAL;",
        ),
        (
            "pages",
            "CREATE TABLE IF NOT EXISTS pages (
                id INTEGER PRIMARY KEY,
                url TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                host TEXT NOT NULL,
                text TEXT NOT NULL,
                simhash INTEGER NOT NULL,
                scale REAL NOT NULL,
                vector BLOB NOT NULL,
                domain_hash BLOB NOT NULL,
                semantic_bucket INTEGER NOT NULL
            );",
        ),
        (
            "fts",
            "CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
                title,
                text,
                tokenize='unicode61 remove_diacritics 0'
            );
            CREATE INDEX IF NOT EXISTS idx_pages_bucket ON pages(semantic_bucket);
            CREATE TABLE IF NOT EXISTS crawl_queue (
                url TEXT PRIMARY KEY,
                visited INTEGER NOT NULL DEFAULT 0,
                added_at INTEGER NOT NULL DEFAULT 0
            );",
        ),
    ];
    for (context, sql) in STEPS {
        conn.execute_batch(sql).map_err(|source| IndexError::Sqlite {
            context,
            source,
        })?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS index_meta (key TEXT PRIMARY KEY, val TEXT NOT NULL);",
    )
        .map_err(|source| IndexError::Sqlite {
            context: "meta",
            source,
        })?;
    ensure_region_column(conn)?;
    conn.execute_batch(
        "        CREATE TABLE IF NOT EXISTS query_log (
            id INTEGER PRIMARY KEY,
            query TEXT NOT NULL,
            asked_at INTEGER NOT NULL,
            result_count INTEGER NOT NULL,
            word_hits INTEGER NOT NULL,
            meaning_hits INTEGER NOT NULL,
            adult_count INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS query_rank (
            id INTEGER PRIMARY KEY,
            log_id INTEGER NOT NULL,
            place INTEGER NOT NULL,
            url TEXT NOT NULL,
            host TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS query_tally (
            query TEXT NOT NULL,
            span TEXT NOT NULL,
            start TEXT NOT NULL,
            hits INTEGER NOT NULL,
            PRIMARY KEY (query, span, start)
        );
        CREATE TABLE IF NOT EXISTS visit_day (
            day TEXT PRIMARY KEY,
            visits INTEGER NOT NULL,
            views INTEGER NOT NULL,
            searches INTEGER NOT NULL,
            bounces INTEGER NOT NULL,
            dwell_seconds INTEGER NOT NULL,
            timed_visits INTEGER NOT NULL,
            empty_searches INTEGER NOT NULL,
            search_errors INTEGER NOT NULL,
            refused INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS volunteers (
            host TEXT PRIMARY KEY,
            seen_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS refused_hosts (
            host TEXT PRIMARY KEY
        );
        CREATE TABLE IF NOT EXISTS muted_hosts (
            host TEXT PRIMARY KEY
        );
        CREATE TABLE IF NOT EXISTS webshops (
            host TEXT PRIMARY KEY
        );
        CREATE TABLE IF NOT EXISTS host_picks (
            host TEXT NOT NULL,
            url TEXT NOT NULL,
            PRIMARY KEY (host, url)
        );",
    )
    .map_err(|source| IndexError::Sqlite {
        context: "query_log",
        source,
    })?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS shell_seen (
            day TEXT NOT NULL,
            id TEXT NOT NULL,
            PRIMARY KEY (day, id)
        );
        CREATE TABLE IF NOT EXISTS shell_pages (
            day TEXT PRIMARY KEY,
            pages INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS bee_day (
            host TEXT NOT NULL,
            day TEXT NOT NULL,
            pages INTEGER NOT NULL,
            PRIMARY KEY (host, day)
        );
        CREATE TABLE IF NOT EXISTS known_nodes (
            base TEXT PRIMARY KEY,
            added_at INTEGER NOT NULL,
            last_ok INTEGER,
            latency_ms INTEGER NOT NULL DEFAULT 0,
            alive INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS intake_day (
            day TEXT PRIMARY KEY,
            new_pages INTEGER NOT NULL,
            changed_pages INTEGER NOT NULL,
            same_pages INTEGER NOT NULL,
            new_hosts INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS load_hour (
            hour TEXT PRIMARY KEY,
            cpu_pct INTEGER NOT NULL,
            ram_pct INTEGER NOT NULL,
            samples INTEGER NOT NULL
        );",
    )
    .map_err(|source| IndexError::Sqlite {
        context: "intake",
        source,
    })?;
    let fts_ver: String = conn
        .query_row(
            "SELECT val FROM index_meta WHERE key = 'fts_yo_strict'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if fts_ver != "1" {
        conn.execute_batch(
            "DROP TABLE IF EXISTS pages_fts;
             CREATE VIRTUAL TABLE pages_fts USING fts5(
                title,
                text,
                tokenize='unicode61 remove_diacritics 0'
             );
             INSERT INTO pages_fts(rowid, title, text)
             SELECT id, title, text FROM pages;
             INSERT OR REPLACE INTO index_meta (key, val) VALUES ('fts_yo_strict', '1');",
        )
        .map_err(|source| IndexError::Sqlite {
            context: "migrate_fts",
            source,
        })?;
    }
    Ok(())
}

fn score_rows<I>(
    query: &[f32],
    rows: I,
) -> Result<Vec<(i64, f32)>, IndexError>
where
    I: Iterator<Item = Result<(i64, f32, Vec<u8>), rusqlite::Error>>,
{
    let mut scored = Vec::new();
    for row in rows {
        let (id, scale, blob) = row?;
        let values = blob.into_iter().map(|byte| byte as i8).collect::<Vec<_>>();
        let stored = QuantVector { scale, values };
        let score = cosine(query, &stored);
        scored.push((id, score));
    }
    Ok(scored)
}

pub fn fts_query(query: &str) -> Option<String> {
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(|token| format!("\"{token}\""))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" AND "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mds_types::{quantize, EMBED_DIMS};

    fn page(url: &str, title: &str, text: &str) -> FetchedPage {
        FetchedPage {
            url: url.into(),
            host: "shop.example".into(),
            title: title.into(),
            text: text.into(),
            simhash: 1,
            links: Vec::new(),
            anchors: Vec::new(),
            indexable: true,
            kind: 0,
            media_url: None,
            feed_url: None,
            news_feed: None,
            price: None,
            docs: Vec::new(),
            priority: Vec::new(),
        }
    }

    #[test]
    fn exact_token_is_a_word_hit() {
        let dir = std::env::temp_dir().join(format!("mds-index-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut index = Index::open(&dir.join("t.db")).unwrap();
        let mut vector = vec![0.0_f32; EMBED_DIMS];
        vector[0] = 1.0;
        let quant = quantize(&vector).unwrap();
        index
            .insert(
                &page(
                    "https://shop.example/bolt",
                    "Крепёж",
                    "Стальной крепёж MDSBOLT42 для каркаса",
                ),
                &quant,
                &[3; 16],
                1,
            )
            .unwrap();
        let hits = index.search_words("MDSBOLT42", 10, 0).unwrap();
        assert_eq!(hits.len(), 1);
        let missed = index.search_words("как лечить собаку", 10, 0).unwrap();
        assert!(missed.is_empty());
        let vec_hits = index.search_vectors(&vector, Some(1), 10, 0).unwrap();
        assert_eq!(vec_hits.len(), 1);
        index
            .enqueue_urls(&["https://shop.example/a".into(), "https://shop.example/b".into()])
            .unwrap();
        index
            .mark_visited(&["https://shop.example/a".into()])
            .unwrap();
        let pending = index.pending_urls(10).unwrap();
        assert_eq!(pending, vec!["https://shop.example/b".to_string()]);
        index.enqueue_next(&["https://shop.example/new".into()]).unwrap();
        let pending = index.pending_urls(10).unwrap();
        assert_eq!(pending[0], "https://shop.example/new");
    }

    #[test]
    fn hundred_first_page_waits_for_a_bee() {
        let dir = std::env::temp_dir().join(format!("mds-index-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("cap.db")).unwrap();
        let urls: Vec<String> = (0..101).map(|n| format!("https://plant.example/p{n}")).collect();
        index.enqueue_urls(&urls).unwrap();
        assert_eq!(index.queue_len().unwrap(), 100);
        index.remember_volunteer("plant.example").unwrap();
        index.enqueue_urls(&["https://plant.example/p100".into()]).unwrap();
        assert_eq!(index.queue_len().unwrap(), 101);
        index.note_webshop("shop.example").unwrap();
        index
            .enqueue_urls(&[
                "https://shop.example/product/bolt".into(),
                "https://shop.example/about".into(),
                "https://www.ozon.ru/product/phone".into(),
                "https://www.ozon.ru/category/phones".into(),
            ])
            .unwrap();
        let pending = index.pending_urls(200).unwrap();
        assert!(pending.iter().any(|url| url.contains("shop.example/product/")));
        assert!(pending.iter().any(|url| url.ends_with("/about")));
        assert!(pending.iter().any(|url| url.contains("/category/")));
        assert!(!pending.iter().any(|url| url.contains("ozon.ru/product/")));
    }

    #[test]
    fn listed_card_pages_replace_the_other_queue() {
        let dir = std::env::temp_dir().join(format!("mds-index-picks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("picks.db")).unwrap();
        index.enqueue_urls(&["https://zavod.example/random".into()]).unwrap();
        index
            .set_priority("www.zavod.example", &["https://zavod.example/keep".into()])
            .unwrap();
        let pending = index.pending_urls(20).unwrap();
        assert!(pending.iter().any(|url| url.ends_with("/keep")));
        assert!(!pending.iter().any(|url| url.ends_with("/random")));
        index.enqueue_urls(&["https://zavod.example/other".into()]).unwrap();
        let pending = index.pending_urls(20).unwrap();
        assert!(!pending.iter().any(|url| url.ends_with("/other")));
    }

    #[test]
    fn same_query_is_counted_by_day_week_and_month() {
        let dir = std::env::temp_dir().join(format!("mds-index-tally-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("tally.db")).unwrap();
        index.remember_query("мёд  липовый", 1, 1, 0, 0).unwrap();
        index.remember_query("мёд липовый", 1, 1, 0, 0).unwrap();
        index.remember_query("мед липовый", 1, 1, 0, 0).unwrap();
        for span in ["day", "week", "month"] {
            let rows = index.query_counts(span, 10).unwrap();
            assert_eq!(rows.iter().find(|(text, _)| text == "мёд липовый").map(|(_, hits)| *hits), Some(2));
            assert_eq!(rows.iter().find(|(text, _)| text == "мед липовый").map(|(_, hits)| *hits), Some(1));
        }
    }

    #[test]
    fn visit_day_is_readable_as_a_series() {
        let dir = std::env::temp_dir().join(format!("mds-index-visits-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("visits.db")).unwrap();
        index
            .add_visit(&VisitDelta {
                visits: 2,
                views: 3,
                searches: 1,
                ..VisitDelta::default()
            })
            .unwrap();
        let days = index.visit_days(14).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].visits, 2);
        assert_eq!(days[0].searches, 1);
        let today = index.visit_span("day").unwrap();
        assert_eq!(today.visits, 2);
        assert_eq!(today.searches, 1);
    }

    #[test]
    fn intake_counts_shells_bees_and_nodes_without_a_query() {
        let dir = std::env::temp_dir().join(format!("mds-index-intake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("intake.db")).unwrap();
        let pace = index.shell_pace().unwrap();
        assert_eq!(pace, ShellPace { batch: 2, gap_min: 10, window_max: 4 });
        index.set_shell_pace(99, 0, 3).unwrap();
        assert_eq!(index.shell_pace().unwrap(), ShellPace { batch: 8, gap_min: 2, window_max: 4 });
        let id = "0123456789abcdef0123456789abcdef";
        index.note_shell(id).unwrap();
        index.note_shell(id).unwrap();
        index.note_shell_pages(3).unwrap();
        let (apps, pages) = index.shell_span("day").unwrap();
        assert_eq!((apps, pages), (1, 3));
        index.note_bee_pages("www.Dartalab.ru", 2).unwrap();
        let bee = index.bee_totals("dartalab.ru").unwrap();
        assert_eq!(bee.day, 2);
        assert!(!bee.quiet);
        let mute = index.bee_totals("magtask.ru").unwrap();
        assert!(mute.quiet);
        index.remember_node("https://bee.example:8443/").unwrap();
        index.touch_node("https://bee.example:8443", true, 40).unwrap();
        let nodes = index.known_nodes().unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].alive);
        assert_eq!(nodes[0].latency_ms, 40);
        assert!(index.stored_simhash("https://example.test/").unwrap().is_none());
        index.note_intake(3, 1, 4, 1).unwrap();
        index.note_intake(2, 0, 0, 0).unwrap();
        assert_eq!(index.intake_sum("new_pages", "day").unwrap(), 5);
        assert_eq!(index.intake_sum("same_pages", "day").unwrap(), 4);
        assert_eq!(index.intake_sum("new_hosts", "yesterday").unwrap(), 0);
        let days = index.intake_days(7).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].new_pages, 5);
        assert_eq!(index.crawl_pace().unwrap(), CrawlPace { batch: 4, gap_secs: 10 });
        index.set_crawl_pace(99, 1).unwrap();
        assert_eq!(index.crawl_pace().unwrap(), CrawlPace { batch: 24, gap_secs: 2 });
        assert_eq!(index.bee_pace().unwrap(), BeePace { batch: 4, gap_secs: 600 });
        assert_eq!(index.shell_ceiling().unwrap(), ShellCeiling { max_batch: 8, min_gap: 1 });
        index.note_load(10, 40).unwrap();
        index.note_load(30, 60).unwrap();
        let load = index.load_hours(2).unwrap();
        assert_eq!(load.len(), 1);
        assert_eq!(load[0].cpu_pct, 20);
        assert_eq!(load[0].ram_pct, 50);
        index.mute_host("www.Example.test").unwrap();
        assert_eq!(index.muted_hosts().unwrap(), vec!["example.test".to_string()]);
        index.unmute_host("example.test").unwrap();
        assert!(index.muted_hosts().unwrap().is_empty());
        assert_eq!(index.shell_grants().unwrap(), 60);
        index.set_shell_grants(9_000).unwrap();
        assert_eq!(index.shell_grants().unwrap(), 600);
        assert_eq!(index.refresh_pace().unwrap(), RefreshPace { batch: 1, gap_secs: 120 });
    }

    #[test]
    fn lease_is_not_given_twice_and_enqueue_next_clears_it() {
        let dir = std::env::temp_dir().join(format!("mds-index-lease-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("lease.db")).unwrap();
        index
            .enqueue_urls(&["https://one.example/a".into(), "https://two.example/b".into()])
            .unwrap();
        let first = index.claim_urls(1, 600).unwrap();
        let second = index.claim_urls(1, 600).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_ne!(first, second);
        assert!(index.claim_urls(1, 600).unwrap().is_empty());
        assert_eq!(index.leased_count().unwrap(), 2);
        index.enqueue_next(&first).unwrap();
        let again = index.claim_urls(1, 600).unwrap();
        assert_eq!(again, first);
    }

    #[test]
    fn volunteer_is_left_out_of_refresh_and_quiet_gap_doubles() {
        let dir = std::env::temp_dir().join(format!("mds-index-refresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut index = Index::open(&dir.join("refresh.db")).unwrap();
        let mut vector = vec![0.0_f32; EMBED_DIMS];
        vector[0] = 1.0;
        let quant = quantize(&vector).unwrap();
        let mut own = page("https://bee.example/", "Свой", "Страница пчелы на своём сайте");
        own.host = "bee.example".into();
        let mut foreign = page("https://other.example/", "Чужой", "Обычная страница без своей пчелы");
        foreign.host = "other.example".into();
        index.insert(&own, &quant, &[3; 16], 1).unwrap();
        index.insert(&foreign, &quant, &[3; 16], 1).unwrap();
        index.remember_volunteer("bee.example").unwrap();
        index.set_next_check("https://bee.example/", 0).unwrap();
        index.set_next_check("https://other.example/", 0).unwrap();
        let due = index.claim_refresh(4).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].url, "https://other.example/");
        assert!(index.claim_refresh(4).unwrap().is_empty());
        index.refresh_quiet("https://other.example/", Some("\"abc\""), None).unwrap();
        let first = index.next_check_of("https://other.example/").unwrap();
        let now: i64 = index
            .conn
            .query_row("SELECT CAST(strftime('%s','now') AS INTEGER)", [], |row| row.get(0))
            .unwrap();
        assert!(first >= now + 86_400 - 5 && first <= now + 86_400 + 5);
        index.refresh_quiet("https://other.example/", Some("\"abc\""), None).unwrap();
        let second = index.next_check_of("https://other.example/").unwrap();
        assert!(second >= now + 2 * 86_400 - 5);
    }

    #[test]
    fn manual_and_known_host_come_before_a_cold_one() {
        let dir = std::env::temp_dir().join(format!("mds-index-worth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut index = Index::open(&dir.join("worth.db")).unwrap();
        let mut vector = vec![0.0_f32; EMBED_DIMS];
        vector[0] = 1.0;
        let quant = quantize(&vector).unwrap();
        let mut known = page("https://good.example/", "Известный", "Уже лежит в индексе");
        known.host = "good.example".into();
        index.insert(&known, &quant, &[2; 16], 1).unwrap();
        index
            .enqueue_urls(&["https://good.example/next".into(), "https://cold.example/a".into()])
            .unwrap();
        index.note_host_quiet("mute.example").unwrap();
        index.enqueue_urls(&["https://mute.example/z".into()]).unwrap();
        index.enqueue_next(&["https://now.example/top".into()]).unwrap();
        let urls = index.claim_urls(3, 600).unwrap();
        assert_eq!(
            urls,
            vec![
                "https://now.example/top".to_string(),
                "https://good.example/next".to_string(),
                "https://cold.example/a".to_string(),
            ]
        );
        assert_eq!(index.pending_urls(10).unwrap(), vec!["https://mute.example/z".to_string()]);
    }

    #[test]
    fn anchor_words_find_a_page_that_does_not_say_them() {
        let dir = std::env::temp_dir().join(format!("mds-index-anchor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut index = Index::open(&dir.join("anchor.db")).unwrap();
        let mut vector = vec![0.0_f32; EMBED_DIMS];
        vector[0] = 1.0;
        let quant = quantize(&vector).unwrap();
        let stored = page("https://shop.example/jar", "Банка", "Стеклянная тара с пасеки");
        index.remember_anchor("https://shop.example/jar", "мёд").unwrap();
        index.insert(&stored, &quant, &[4; 16], 1).unwrap();
        let hits = index.search_words("мёд", 10, 0).unwrap();
        assert_eq!(hits.len(), 1);
        let later = index.pages_after(0, 10).unwrap();
        assert_eq!(later.len(), 1);
        assert!(later[0].revision > 0);
    }

    #[test]
    fn geo_places_groups_sites_and_merges_host_contacts() {
        let dir = std::env::temp_dir().join(format!("mds-index-geo-places-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut index = Index::open(&dir.join("geo.db")).unwrap();
        let mut vector = vec![0.0_f32; EMBED_DIMS];
        vector[0] = 1.0;
        let quant = quantize(&vector).unwrap();

        let mut cafe_main = page(
            "https://cafe-khimki.example/",
            "Семейное кафе в Химках",
            "Домашняя кухня, обеды и выпечка в городе Химки, ул. Молодёжная, 10",
        );
        cafe_main.host = "cafe-khimki.example".into();
        index.insert(&cafe_main, &quant, &[5; 16], 1).unwrap();

        let mut cafe_contacts = page(
            "https://cafe-khimki.example/contacts",
            "Контакты кафе в Химках",
            "Geo: 55.8970, 37.4297 Tel: +74951112233 Email: hello@cafe-khimki.ru Msg: https://t.me/cafe_khimki Soc: https://vk.com/cafe_khimki",
        );
        cafe_contacts.host = "cafe-khimki.example".into();
        index.insert(&cafe_contacts, &quant, &[5; 16], 1).unwrap();

        let (sites, pages_cnt, rows) = index
            .geo_places(
                "кафе",
                0,
                None,
                Some("Химки"),
                Some((55.8970, 37.4297, 5.0)),
                None,
                50,
            )
            .unwrap();
        assert_eq!(sites, 1);
        assert_eq!(pages_cnt, 2);
        assert_eq!(rows[0].host, "cafe-khimki.example");
        assert_eq!(rows[0].contacts.phones, vec!["+74951112233"]);
        assert_eq!(rows[0].contacts.emails, vec!["hello@cafe-khimki.ru"]);
        assert_eq!(rows[0].contacts.messengers, vec!["https://t.me/cafe_khimki"]);
        assert_eq!(rows[0].contacts.socials, vec!["https://vk.com/cafe_khimki"]);
    }
}
