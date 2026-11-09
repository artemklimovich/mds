use std::collections::{HashMap, HashSet};
use std::path::Path;

use mds_crawl::{is_broken_url, is_near_duplicate, is_not_found, strip_boilerplate, tidy_url, FetchedPage};
use mds_types::{
    all_kinds, bare_host, channel_entity_url, contacts_in_text, host_in_focus, host_keeps_pages, host_of_url, is_channel_host, is_channel_item, is_channel_noise, is_document_url, is_government_host, is_hive_host, is_image_page, is_marketplace_host, is_model_inner, is_model_maker_host, is_press_front, is_press_host, is_product_card, is_raw_office_text, is_tool_docs_host, is_video_page, kind_from_stored, cosine, swap_yo, url_in_focus,
    QuantVector, EMBED_DIMS, HOST_PAGE_CAP, KIND_CHANNEL, KIND_IMAGE, KIND_NEWS, KIND_VIDEO, THEME_BITS,
};

fn news_shelf(kind_mask: u32) -> u32 {
    if kind_mask == KIND_NEWS {
        1
    } else {
        0
    }
}
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

/// Пока очередь длиннее, новые ссылки с обхода и CT не берём — разбираем уже найденное.
pub const QUEUE_GROW_CAP: i64 = 50_000;

pub fn queue_may_grow_len(len: i64) -> bool {
    len < QUEUE_GROW_CAP
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivitySlot {
    pub bucket: String,
    pub new_pages: i64,
    pub new_hosts: i64,
    pub walked: i64,
    pub shell_pages: i64,
    pub cpu: i64,
    pub ram: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchPaceSlot {
    pub minute: String,
    pub count: i64,
    pub avg_ms: i64,
    pub max_ms: i64,
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
pub struct VolunteerRow {
    pub host: String,
    pub seen_at: i64,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownNode {
    pub base: String,
    pub alive: bool,
    pub latency_ms: u64,
    pub last_ok: Option<i64>,
}

fn activity_sql(grain: &str) -> &'static str {
    match grain {
        "hour" => {
            "WITH keys AS (
                SELECT bucket AS k FROM intake_tick WHERE grain = 'hour'
                UNION
                SELECT hour FROM load_hour
            )
            SELECT k,
                COALESCE((SELECT new_pages FROM intake_tick WHERE grain = 'hour' AND bucket = k), 0),
                COALESCE((SELECT new_hosts FROM intake_tick WHERE grain = 'hour' AND bucket = k), 0),
                COALESCE((SELECT new_pages + changed_pages + same_pages FROM intake_tick WHERE grain = 'hour' AND bucket = k), 0),
                COALESCE((SELECT shell_pages FROM intake_tick WHERE grain = 'hour' AND bucket = k), 0),
                COALESCE((SELECT cpu_pct FROM load_hour WHERE hour = k), 0),
                COALESCE((SELECT ram_pct FROM load_hour WHERE hour = k), 0)
            FROM keys
            ORDER BY k DESC
            LIMIT ?1"
        }
        "day" => {
            "WITH keys AS (
                SELECT day AS k FROM intake_day
                UNION SELECT day FROM shell_pages
                UNION SELECT substr(hour, 1, 10) FROM load_hour
            )
            SELECT k,
                COALESCE((SELECT new_pages FROM intake_day WHERE day = k), 0),
                COALESCE((SELECT new_hosts FROM intake_day WHERE day = k), 0),
                COALESCE((SELECT new_pages + changed_pages + same_pages FROM intake_day WHERE day = k), 0),
                COALESCE((SELECT pages FROM shell_pages WHERE day = k), 0),
                CAST(COALESCE((SELECT AVG(cpu_pct) FROM load_hour WHERE substr(hour, 1, 10) = k), 0) AS INTEGER),
                CAST(COALESCE((SELECT AVG(ram_pct) FROM load_hour WHERE substr(hour, 1, 10) = k), 0) AS INTEGER)
            FROM keys
            ORDER BY k DESC
            LIMIT ?1"
        }
        "week" => {
            "WITH keys AS (
                SELECT strftime('%Y-%W', day) AS k FROM intake_day
                UNION SELECT strftime('%Y-%W', day) FROM shell_pages
                UNION SELECT strftime('%Y-%W', substr(hour, 1, 10)) FROM load_hour
            )
            SELECT k,
                COALESCE((SELECT SUM(new_pages) FROM intake_day WHERE strftime('%Y-%W', day) = k), 0),
                COALESCE((SELECT SUM(new_hosts) FROM intake_day WHERE strftime('%Y-%W', day) = k), 0),
                COALESCE((SELECT SUM(new_pages + changed_pages + same_pages) FROM intake_day WHERE strftime('%Y-%W', day) = k), 0),
                COALESCE((SELECT SUM(pages) FROM shell_pages WHERE strftime('%Y-%W', day) = k), 0),
                CAST(COALESCE((SELECT AVG(cpu_pct) FROM load_hour WHERE strftime('%Y-%W', substr(hour, 1, 10)) = k), 0) AS INTEGER),
                CAST(COALESCE((SELECT AVG(ram_pct) FROM load_hour WHERE strftime('%Y-%W', substr(hour, 1, 10)) = k), 0) AS INTEGER)
            FROM keys
            ORDER BY k DESC
            LIMIT ?1"
        }
        "month" => {
            "WITH keys AS (
                SELECT strftime('%Y-%m', day) AS k FROM intake_day
                UNION SELECT strftime('%Y-%m', day) FROM shell_pages
                UNION SELECT strftime('%Y-%m', substr(hour, 1, 10)) FROM load_hour
            )
            SELECT k,
                COALESCE((SELECT SUM(new_pages) FROM intake_day WHERE strftime('%Y-%m', day) = k), 0),
                COALESCE((SELECT SUM(new_hosts) FROM intake_day WHERE strftime('%Y-%m', day) = k), 0),
                COALESCE((SELECT SUM(new_pages + changed_pages + same_pages) FROM intake_day WHERE strftime('%Y-%m', day) = k), 0),
                COALESCE((SELECT SUM(pages) FROM shell_pages WHERE strftime('%Y-%m', day) = k), 0),
                CAST(COALESCE((SELECT AVG(cpu_pct) FROM load_hour WHERE strftime('%Y-%m', substr(hour, 1, 10)) = k), 0) AS INTEGER),
                CAST(COALESCE((SELECT AVG(ram_pct) FROM load_hour WHERE strftime('%Y-%m', substr(hour, 1, 10)) = k), 0) AS INTEGER)
            FROM keys
            ORDER BY k DESC
            LIMIT ?1"
        }
        _ => {
            "WITH keys AS (
                SELECT bucket AS k FROM intake_tick WHERE grain = 'minute'
                UNION
                SELECT minute FROM load_minute
            )
            SELECT k,
                COALESCE((SELECT new_pages FROM intake_tick WHERE grain = 'minute' AND bucket = k), 0),
                COALESCE((SELECT new_hosts FROM intake_tick WHERE grain = 'minute' AND bucket = k), 0),
                COALESCE((SELECT new_pages + changed_pages + same_pages FROM intake_tick WHERE grain = 'minute' AND bucket = k), 0),
                COALESCE((SELECT shell_pages FROM intake_tick WHERE grain = 'minute' AND bucket = k), 0),
                COALESCE((SELECT cpu_pct FROM load_minute WHERE minute = k), 0),
                COALESCE((SELECT ram_pct FROM load_minute WHERE minute = k), 0)
            FROM keys
            ORDER BY k DESC
            LIMIT ?1"
        }
    }
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

pub struct FocusSweep {
    pub dropped: usize,
    pub trimmed: usize,
    pub queue: usize,
}

#[derive(Clone, Debug)]
pub struct HostInspect {
    pub host: String,
    pub pages: i64,
    pub queued: i64,
    pub banned: bool,
    pub muted: bool,
    pub keep: bool,
    pub volunteer: bool,
    pub government: bool,
    pub hive: bool,
    pub tool_docs: bool,
    pub model_maker: bool,
    pub in_focus: bool,
}

impl HostInspect {
    pub fn rule(&self) -> &'static str {
        if self.banned {
            "не обходить"
        } else if !self.in_focus {
            "вне фокуса, в очередь не ставится"
        } else if self.volunteer || self.government || self.hive || self.tool_docs || self.keep {
            "без потолка"
        } else if self.model_maker {
            "только главная"
        } else if mds_types::is_channel_visit_host(&self.host) {
            "карточки каналов, без потолка"
        } else {
            "потолок 100 страниц"
        }
    }
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
        conn.busy_timeout(std::time::Duration::from_secs(3)).map_err(|source| IndexError::Sqlite {
            context: "open",
            source,
        })?;
        init_schema(&conn)?;
        ensure_region_column(&conn)?;
        ensure_kind_columns(&conn)?;
        ensure_queue_lease(&conn)?;
        ensure_queue_worth(&conn)?;
        ensure_refresh_columns(&conn)?;
        ensure_volunteer_version(&conn)?;
        ensure_anchor_table(&conn)?;
        ensure_revision(&conn)?;
        let index = Self { conn };
        index.seed_revisions()?;
        index.seed_refresh()?;
        let _ = index.drain_model_inner_queue();
        if index.meta_get("boot_scan")?.as_deref() != Some("1") {
            let _ = index.fill_kinds()?;
            let (labeled, total) = index.fill_regions()?;
            if total > 0 {
                eprintln!("MDS: регион подписан у {labeled} страниц из {total}");
            }
            let _ = index.meta_set("boot_scan", "1");
        }
        if index.meta_get("kind_pass")?.as_deref() != Some("132") {
            let labeled = index.fill_kinds()?;
            if labeled > 0 {
                eprintln!("MDS: метки пересчитаны у {labeled} страниц");
            }
            let harvested = index.harvest_channel_entities()?;
            if harvested > 0 {
                eprintln!("MDS: в очередь каналов поставлено {harvested}");
            }
            let _ = index.meta_set("kind_pass", "132");
        }
        Ok(index)
    }

    /// Второе соединение только для чтения. Обход пишет в первое, кабинет читает здесь и не ждёт замок записи.
    pub fn open_read(path: &Path) -> Result<Self, IndexError> {
        let conn = Connection::open(path).map_err(|source| IndexError::Sqlite {
            context: "open-read",
            source,
        })?;
        conn.busy_timeout(std::time::Duration::from_secs(3)).map_err(|source| IndexError::Sqlite {
            context: "open-read",
            source,
        })?;
        conn.execute_batch("PRAGMA query_only=ON;").map_err(|source| IndexError::Sqlite {
            context: "open-read",
            source,
        })?;
        Ok(Self { conn })
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

    pub fn search_words(&self, query: &str, limit: usize, kind_mask: u32, theme_mask: u32) -> Result<Vec<WordMatch>, IndexError> {
        let tokens = content_tokens(query);
        if tokens.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut queries = Vec::new();
        if let Some(near) = fts_near(&tokens, false) {
            queries.push(near);
        }
        if let Some(near) = fts_near(&tokens, true) {
            queries.push(near);
        }
        let exact = fts_and(&tokens, false);
        let loose = fts_and(&tokens, true);
        queries.push(exact);
        if loose != *queries.last().expect("exact") {
            queries.push(loose);
        }
        let mut seen = HashSet::new();
        let mut ids = Vec::new();
        for match_query in queries {
            if ids.len() >= limit {
                break;
            }
            for id in self.word_ids(&match_query, limit, kind_mask, theme_mask)? {
                if seen.insert(id) {
                    ids.push(id);
                    if ids.len() >= limit {
                        break;
                    }
                }
            }
        }
        Ok(ids
            .into_iter()
            .enumerate()
            .map(|(offset, id)| WordMatch {
                id,
                rank: offset + 1,
            })
            .collect())
    }

    fn word_ids(&self, match_query: &str, limit: usize, kind_mask: u32, theme_mask: u32) -> Result<Vec<i64>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT pages.id
             FROM pages_fts
             JOIN pages ON pages.id = pages_fts.rowid
             WHERE pages_fts MATCH ?1
               AND ((?3 != 0 AND (pages.kind & ?3) != 0) OR (?3 = 0 AND (pages.kind & ?4) = 0))
               AND (?5 = 0 OR (pages.kind & ?5) != 0 OR (?6 != 0 AND (pages.kind & ?7) = 0))
             ORDER BY bm25(pages_fts)
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![
                match_query,
                limit as i64,
                kind_mask,
                KIND_NEWS | KIND_CHANNEL,
                theme_mask,
                news_shelf(kind_mask),
                THEME_BITS
            ],
            |row| row.get::<_, i64>(0),
        )?;
        let mut out = Vec::new();
        for id in rows {
            out.push(id?);
        }
        Ok(out)
    }

    pub fn search_vectors(
        &self,
        query: &[f32],
        bucket: Option<u16>,
        limit: usize,
        kind_mask: u32,
        theme_mask: u32,
    ) -> Result<Vec<VectorMatch>, IndexError> {
        let mut scored = match bucket {
            Some(bucket) => self.score_bucket_neighbors(query, bucket, kind_mask, theme_mask)?,
            None => self.score_all_vectors(query, kind_mask, theme_mask)?,
        };
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
        theme_mask: u32,
    ) -> Result<Vec<(i64, f32)>, IndexError> {
        let mut b = [0_i64; 17];
        b[0] = bucket as i64;
        for i in 0..16 {
            b[i + 1] = (bucket ^ (1 << i)) as i64;
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, scale, vector, kind FROM pages
             WHERE semantic_bucket IN (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
               AND ((?18 != 0 AND (kind & ?18) != 0) OR (?18 = 0 AND (kind & ?19) = 0))
               AND (?20 = 0 OR (kind & ?20) != 0 OR (?21 != 0 AND (kind & ?22) = 0))",
        )?;
        let rows = stmt.query_map(
            params![
                b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8],
                b[9], b[10], b[11], b[12], b[13], b[14], b[15], b[16], kind_mask, KIND_NEWS | KIND_CHANNEL,
                theme_mask, news_shelf(kind_mask), THEME_BITS
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

    fn score_all_vectors(&self, query: &[f32], kind_mask: u32, theme_mask: u32) -> Result<Vec<(i64, f32)>, IndexError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, scale, vector FROM pages
                 WHERE ((?1 != 0 AND (kind & ?1) != 0) OR (?1 = 0 AND (kind & ?2) = 0))
                   AND (?3 = 0 OR (kind & ?3) != 0 OR (?4 != 0 AND (kind & ?5) = 0))",
            )?;
        let rows = stmt.query_map(
            params![kind_mask, KIND_NEWS | KIND_CHANNEL, theme_mask, news_shelf(kind_mask), THEME_BITS],
            |row| {
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
            if is_document_url(&trimmed) || is_channel_noise(&trimmed) {
                continue;
            }
            let Some(host) = host_of_url(&trimmed) else {
                continue;
            };
            let bare = bare_host(&host);
            if !url_in_focus(&trimmed) {
                continue;
            }
            if is_marketplace_host(&bare) && is_product_card(&trimmed) {
                continue;
            }
            if is_press_host(&bare) && !is_press_front(&trimmed) {
                continue;
            }
            if is_model_inner(&trimmed) {
                continue;
            }
            if self.is_banned_host(&bare)? {
                continue;
            }
            let open = self.host_open(&bare)? || is_channel_host(&bare);
            if self.has_picks(&bare)? && !open && !self.is_pick(&bare, &trimmed)? {
                continue;
            }
            if !open {
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

    /// Ссылки, только что найденные на странице, идут раньше холодной очереди.
    pub fn pull_ahead(&self, urls: &[String]) -> Result<(), IndexError> {
        for url in urls {
            let trimmed = url.trim();
            if trimmed.is_empty() {
                continue;
            }
            self.conn.execute(
                "UPDATE crawl_queue SET worth = 1 WHERE url = ?1 AND visited = 0 AND worth > 1",
                [trimmed],
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

    pub fn is_volunteer_host(&self, bare: &str) -> Result<bool, IndexError> {
        let found: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM volunteers WHERE host = ?1 OR host = ?2",
            params![bare, format!("www.{bare}")],
            |row| row.get(0),
        )?;
        Ok(found > 0)
    }

    pub fn host_counts(&self, host: &str) -> Result<(i64, i64), IndexError> {
        let bare = bare_host(host);
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
        Ok((pages, queued))
    }

    fn host_pressure(&self, bare: &str) -> Result<i64, IndexError> {
        let (pages, queued) = self.host_counts(bare)?;
        Ok(pages + queued)
    }

    pub fn enqueue_next(&self, urls: &[String]) -> Result<(), IndexError> {
        for url in urls {
            let trimmed = url.trim();
            if trimmed.is_empty() || is_broken_url(trimmed) || is_document_url(trimmed) || is_channel_noise(trimmed) {
                continue;
            }
            if let Some(host) = host_of_url(trimmed) {
                if !url_in_focus(trimmed)
                    || self.is_banned_host(&host)?
                    || (is_press_host(&host) && !is_press_front(trimmed))
                    || is_model_inner(trimmed)
                {
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

    pub fn sweep_focus(&self) -> Result<FocusSweep, IndexError> {
        let volunteers = self.volunteer_names()?;
        let pages = {
            let mut stmt = self.conn.prepare("SELECT id, host, url FROM pages").map_err(|source| IndexError::Sqlite {
                context: "focus",
                source,
            })?;
            let mapped = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))?;
            mapped.collect::<Result<Vec<_>, _>>().map_err(|source| IndexError::Sqlite {
                context: "focus",
                source,
            })?
        };
        let mut dropped = Vec::new();
        let mut grouped: HashMap<String, Vec<(i64, usize)>> = HashMap::new();
        for (id, host, url) in pages {
            if !url_in_focus(&url) {
                dropped.push(id);
                continue;
            }
            grouped.entry(bare_host(&host)).or_default().push((id, url.len()));
        }
        let mut trimmed = Vec::new();
        let mut full = HashSet::new();
        for (host, mut rows) in grouped {
            let bee = volunteers.contains(&host) || volunteers.contains(&format!("www.{host}"));
            let keep = bee || host_keeps_pages(&host) || self.is_keep_host(&host).unwrap_or(false) || is_channel_host(&host);
            if !keep && rows.len() as i64 >= HOST_PAGE_CAP {
                full.insert(host.clone());
            }
            if keep || rows.len() as i64 <= HOST_PAGE_CAP {
                continue;
            }
            rows.sort_by_key(|(id, len)| (*len, *id));
            trimmed.extend(rows.into_iter().skip(HOST_PAGE_CAP as usize).map(|(id, _)| id));
        }
        let page_ids: Vec<i64> = dropped.iter().copied().chain(trimmed.iter().copied()).collect();
        if !page_ids.is_empty() {
            let tx = self.conn.unchecked_transaction().map_err(|source| IndexError::Sqlite {
                context: "focus",
                source,
            })?;
            {
                let mut fts = tx.prepare("DELETE FROM pages_fts WHERE rowid = ?1").map_err(|source| IndexError::Sqlite {
                    context: "focus",
                    source,
                })?;
                let mut page = tx.prepare("DELETE FROM pages WHERE id = ?1").map_err(|source| IndexError::Sqlite {
                    context: "focus",
                    source,
                })?;
                for id in &page_ids {
                    fts.execute([id]).map_err(|source| IndexError::Sqlite {
                        context: "focus",
                        source,
                    })?;
                    page.execute([id]).map_err(|source| IndexError::Sqlite {
                        context: "focus",
                        source,
                    })?;
                }
            }
            tx.commit().map_err(|source| IndexError::Sqlite {
                context: "focus",
                source,
            })?;
        }
        let queue = self.sweep_foreign_queue(&full)?;
        Ok(FocusSweep {
            dropped: dropped.len(),
            trimmed: trimmed.len(),
            queue,
        })
    }

    fn volunteer_names(&self) -> Result<HashSet<String>, IndexError> {
        let mut stmt = self.conn.prepare("SELECT host FROM volunteers").map_err(|source| IndexError::Sqlite {
            context: "focus",
            source,
        })?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|source| IndexError::Sqlite {
                context: "focus",
                source,
            })?;
        let mut names = HashSet::new();
        for row in rows {
            names.insert(bare_host(&row.map_err(|source| IndexError::Sqlite {
                context: "focus",
                source,
            })?));
        }
        Ok(names)
    }

    fn sweep_foreign_queue(&self, full: &HashSet<String>) -> Result<usize, IndexError> {
        let urls = {
            let mut stmt = self.conn.prepare("SELECT url FROM crawl_queue").map_err(|source| IndexError::Sqlite {
                context: "focus_queue",
                source,
            })?;
            let mapped = stmt.query_map([], |row| row.get::<_, String>(0))?;
            mapped.collect::<Result<Vec<_>, _>>().map_err(|source| IndexError::Sqlite {
                context: "focus_queue",
                source,
            })?
        };
        let mut hosts = HashSet::new();
        let mut drop_urls = Vec::new();
        for url in urls {
            let Some(host) = host_of_url(&url) else {
                continue;
            };
            let bare = bare_host(&host);
            if mds_types::is_channel_visit_host(&bare) {
                if is_channel_item(&url) || !url_in_focus(&url) {
                    drop_urls.push(url);
                }
                continue;
            }
            if !host_in_focus(&bare) || full.contains(&bare) {
                hosts.insert(host);
            }
        }
        let mut extra = 0_usize;
        for url in &drop_urls {
            extra += self
                .conn
                .execute("DELETE FROM crawl_queue WHERE url = ?1", [url])
                .map_err(|source| IndexError::Sqlite {
                    context: "focus_queue",
                    source,
                })?;
        }
        let mut removed = extra;
        for host in hosts {
            removed += self
                .conn
                .execute(
                    "DELETE FROM crawl_queue WHERE url LIKE ?1 OR url LIKE ?2 OR url = ?3 OR url = ?4",
                    params![
                        format!("https://{host}/%"),
                        format!("http://{host}/%"),
                        format!("https://{host}"),
                        format!("http://{host}"),
                    ],
                )
                .map_err(|source| IndexError::Sqlite {
                    context: "focus_queue",
                    source,
                })?;
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
    /// С одного хоста не больше двух за заход, чтобы очередь гиганта не забивала весь обход.
    pub fn claim_urls(&self, limit: usize, hold_secs: i64) -> Result<Vec<String>, IndexError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let hold = hold_secs.clamp(1, 14 * 24 * 3600);
        let pool = (limit * 8).clamp(limit, 192);
        let candidates = {
            let mut stmt = self.conn.prepare(
                "SELECT url FROM (
                    SELECT url, worth, added_at,
                        ROW_NUMBER() OVER (
                            PARTITION BY CASE
                                WHEN instr(replace(replace(lower(url), 'https://', ''), 'http://', ''), '/') > 0
                                THEN substr(
                                    replace(replace(lower(url), 'https://', ''), 'http://', ''),
                                    1,
                                    instr(replace(replace(lower(url), 'https://', ''), 'http://', ''), '/') - 1
                                )
                                ELSE replace(replace(lower(url), 'https://', ''), 'http://', '')
                            END
                            ORDER BY worth ASC, added_at ASC
                        ) AS rn
                    FROM crawl_queue
                    WHERE visited = 0 AND worth < 5 AND lease_until < strftime('%s','now')
                )
                WHERE rn <= 2
                ORDER BY worth ASC, added_at ASC
                LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![pool as i64], |row| row.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };
        let mut chosen = Vec::new();
        let mut drop = Vec::new();
        let mut per_host: HashMap<String, usize> = HashMap::new();
        for url in candidates {
            if chosen.len() >= limit {
                break;
            }
            let host = host_of_url(&url).map(|host| bare_host(&host)).unwrap_or_default();
            if per_host.get(&host).copied().unwrap_or(0) >= 2 {
                continue;
            }
            if !self.queue_should_fetch(&url)? {
                drop.push(url);
                continue;
            }
            *per_host.entry(host).or_insert(0) += 1;
            chosen.push(url);
        }
        self.mark_visited(&drop)?;
        let tx = self.conn.unchecked_transaction().map_err(|source| IndexError::Sqlite {
            context: "lease",
            source,
        })?;
        for url in &chosen {
            tx.execute(
                "UPDATE crawl_queue SET lease_until = strftime('%s','now') + ?1 WHERE url = ?2 AND visited = 0",
                params![hold, url],
            )?;
        }
        tx.commit().map_err(|source| IndexError::Sqlite {
            context: "lease",
            source,
        })?;
        Ok(chosen)
    }

    fn queue_should_fetch(&self, url: &str) -> Result<bool, IndexError> {
        let exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pages WHERE url = ?1",
            [url],
            |row| row.get(0),
        )?;
        if exists > 0 {
            return Ok(false);
        }
        let Some(host) = host_of_url(url) else {
            return Ok(false);
        };
        self.may_store(&host, url)
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

    pub fn pages_of_host(&self, host: &str, kind_mask: u32, theme_mask: u32, limit: usize) -> Result<Vec<PageRow>, IndexError> {
        let bare = mds_types::bare_host(host);
        let www = format!("www.{bare}");
        let limit = limit.min(100) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT id, url, title, host, text, region, place, lat, lon, kind, media_url, feed_url, price, news_feed, simhash
             FROM pages
             WHERE (host = ?1 OR host = ?2)
               AND ((?3 != 0 AND (kind & ?3) != 0) OR (?3 = 0 AND (kind & ?4) = 0))
               AND (?5 = 0 OR (kind & ?5) != 0 OR (?6 != 0 AND (kind & ?7) = 0))
             ORDER BY id DESC
             LIMIT ?8",
        )?;
        let rows = stmt.query_map(
            params![
                bare,
                www,
                kind_mask,
                mds_types::KIND_NEWS | mds_types::KIND_CHANNEL,
                theme_mask,
                news_shelf(kind_mask),
                THEME_BITS,
                limit
            ],
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

    pub fn visit_windows(&self) -> Result<(String, String, String), IndexError> {
        self.conn
            .query_row(
                "SELECT
                    date('now','localtime'),
                    date('now','localtime','-' || ((cast(strftime('%w','now','localtime') as integer) + 6) % 7) || ' days'),
                    date('now','localtime','start of month')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(IndexError::from)
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
            if !bare.is_empty() && !is_channel_host(&bare) {
                if let Some(hc) = read_host_contacts(&self.conn, &bare) {
                    out.merge(&hc);
                }
            }
        } else if let Some(host) = mds_types::host_of_url(url) {
            let bare = mds_types::bare_host(&host);
            if !is_channel_host(&bare) {
                if let Some(hc) = read_host_contacts(&self.conn, &bare) {
                    out.merge(&hc);
                }
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
            let matches = self.search_words(q_trim, 2000, kind_mask, 0)?;
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
            if !bare.is_empty() && !is_channel_host(&bare) {
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

    pub fn queue_may_grow(&self) -> Result<bool, IndexError> {
        Ok(queue_may_grow_len(self.queue_len()? as i64))
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
        self.remember_volunteer_version(host, "")
    }

    pub fn remember_volunteer_version(&self, host: &str, version: &str) -> Result<(), IndexError> {
        let host = host.trim().to_ascii_lowercase();
        if host.is_empty() {
            return Ok(());
        }
        let version = version.trim();
        self.conn.execute(
            "INSERT INTO volunteers (host, seen_at, version) VALUES (?1, strftime('%s','now'), ?2)
             ON CONFLICT(host) DO UPDATE SET
                seen_at = excluded.seen_at,
                version = CASE WHEN excluded.version = '' THEN volunteers.version ELSE excluded.version END",
            params![host, version],
        )?;
        Ok(())
    }

    pub fn volunteers(&self) -> Result<Vec<String>, IndexError> {
        Ok(self.volunteer_seen()?.into_iter().map(|row| row.host).collect())
    }

    pub fn volunteer_seen(&self) -> Result<Vec<VolunteerRow>, IndexError> {
        let mut stmt = self.conn.prepare("SELECT host, seen_at, COALESCE(version, '') FROM volunteers ORDER BY host")?;
        let rows = stmt.query_map([], |row| {
            Ok(VolunteerRow {
                host: row.get(0)?,
                seen_at: row.get(1)?,
                version: row.get(2)?,
            })
        })?;
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

    fn drain_model_inner_queue(&self) -> Result<usize, IndexError> {
        if self.meta_get("model_front_q2")?.as_deref() == Some("1") {
            return Ok(0);
        }
        let queued = {
            let mut stmt = self.conn.prepare("SELECT url FROM crawl_queue WHERE visited = 0")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };
        let mut drop = Vec::new();
        for url in queued {
            if host_of_url(&url).is_none() {
                continue;
            }
            if is_model_inner(&url) {
                drop.push(url);
            }
        }
        let n = drop.len();
        if !drop.is_empty() {
            let tx = self.conn.unchecked_transaction().map_err(|source| IndexError::Sqlite {
                context: "queue",
                source,
            })?;
            for url in &drop {
                tx.execute("UPDATE crawl_queue SET visited = 1 WHERE url = ?1", [url])?;
            }
            tx.commit().map_err(|source| IndexError::Sqlite {
                context: "queue",
                source,
            })?;
            eprintln!("MDS: из очереди убраны внутренние адреса моделей, {n}");
        }
        self.meta_set("model_front_q2", "1")?;
        Ok(n)
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
        self.bump_shell_tick(
            "minute",
            "strftime('%Y-%m-%d %H:%M','now','localtime')",
            "strftime('%Y-%m-%d %H:%M','now','localtime','-3 hours')",
            pages,
        )?;
        self.bump_shell_tick(
            "hour",
            "strftime('%Y-%m-%d %H','now','localtime')",
            "strftime('%Y-%m-%d %H','now','localtime','-14 days')",
            pages,
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
        self.bump_intake_tick(
            "minute",
            "strftime('%Y-%m-%d %H:%M','now','localtime')",
            "strftime('%Y-%m-%d %H:%M','now','localtime','-3 hours')",
            new_pages,
            changed_pages,
            same_pages,
            new_hosts,
        )?;
        self.bump_intake_tick(
            "hour",
            "strftime('%Y-%m-%d %H','now','localtime')",
            "strftime('%Y-%m-%d %H','now','localtime','-14 days')",
            new_pages,
            changed_pages,
            same_pages,
            new_hosts,
        )?;
        Ok(())
    }

    fn bump_intake_tick(
        &self,
        grain: &str,
        bucket_expr: &str,
        keep_expr: &str,
        new_pages: i64,
        changed_pages: i64,
        same_pages: i64,
        new_hosts: i64,
    ) -> Result<(), IndexError> {
        self.conn.execute(
            &format!(
                "INSERT INTO intake_tick (grain, bucket, new_pages, changed_pages, same_pages, new_hosts, shell_pages)
                 VALUES (?1, {bucket_expr}, ?2, ?3, ?4, ?5, 0)
                 ON CONFLICT(grain, bucket) DO UPDATE SET
                    new_pages = new_pages + excluded.new_pages,
                    changed_pages = changed_pages + excluded.changed_pages,
                    same_pages = same_pages + excluded.same_pages,
                    new_hosts = new_hosts + excluded.new_hosts"
            ),
            params![grain, new_pages, changed_pages, same_pages, new_hosts],
        )?;
        self.conn.execute(
            &format!("DELETE FROM intake_tick WHERE grain = ?1 AND bucket < {keep_expr}"),
            params![grain],
        )?;
        Ok(())
    }

    fn bump_shell_tick(&self, grain: &str, bucket_expr: &str, keep_expr: &str, pages: i64) -> Result<(), IndexError> {
        self.conn.execute(
            &format!(
                "INSERT INTO intake_tick (grain, bucket, new_pages, changed_pages, same_pages, new_hosts, shell_pages)
                 VALUES (?1, {bucket_expr}, 0, 0, 0, 0, ?2)
                 ON CONFLICT(grain, bucket) DO UPDATE SET
                    shell_pages = shell_pages + excluded.shell_pages"
            ),
            params![grain, pages],
        )?;
        self.conn.execute(
            &format!("DELETE FROM intake_tick WHERE grain = ?1 AND bucket < {keep_expr}"),
            params![grain],
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
        self.conn.execute(
            "INSERT INTO load_minute (minute, cpu_pct, ram_pct)
             VALUES (strftime('%Y-%m-%d %H:%M','now','localtime'), ?1, ?2)
             ON CONFLICT(minute) DO UPDATE SET
                cpu_pct = excluded.cpu_pct,
                ram_pct = excluded.ram_pct",
            params![cpu_pct, ram_pct],
        )?;
        self.conn.execute(
            "DELETE FROM load_minute WHERE minute < strftime('%Y-%m-%d %H:%M','now','localtime','-3 hours')",
            [],
        )?;
        Ok(())
    }

    pub fn note_search_pace(&self, elapsed_ms: i64) -> Result<(), IndexError> {
        let elapsed_ms = elapsed_ms.max(0);
        self.conn.execute(
            "INSERT INTO search_minute (minute, count, avg_ms, max_ms)
             VALUES (strftime('%Y-%m-%d %H:%M','now','localtime'), 1, ?1, ?1)
             ON CONFLICT(minute) DO UPDATE SET
                avg_ms = (avg_ms * count + excluded.avg_ms) / (count + 1),
                max_ms = MAX(max_ms, excluded.max_ms),
                count = count + 1",
            [elapsed_ms],
        )?;
        self.conn.execute(
            "DELETE FROM search_minute WHERE minute < strftime('%Y-%m-%d %H:%M','now','localtime','-3 hours')",
            [],
        )?;
        Ok(())
    }

    pub fn search_pace(&self, limit: usize) -> Result<Vec<SearchPaceSlot>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT minute, count, avg_ms, max_ms FROM search_minute ORDER BY minute DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(SearchPaceSlot {
                minute: row.get(0)?,
                count: row.get(1)?,
                avg_ms: row.get(2)?,
                max_ms: row.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out.reverse();
        Ok(out)
    }

    pub fn search_peak_ms(&self, hours: i64) -> Result<i64, IndexError> {
        let hours = hours.clamp(1, 24);
        let peak: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(max_ms), 0) FROM search_minute
             WHERE minute >= strftime('%Y-%m-%d %H:%M','now','localtime', ?1)",
            [format!("-{hours} hours")],
            |row| row.get(0),
        )?;
        Ok(peak)
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

    pub fn activity_span(&self, grain: &str, limit: usize) -> Result<Vec<ActivitySlot>, IndexError> {
        let mut stmt = self.conn.prepare(activity_sql(grain))?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(ActivitySlot {
                bucket: row.get(0)?,
                new_pages: row.get(1)?,
                new_hosts: row.get(2)?,
                walked: row.get(3)?,
                shell_pages: row.get(4)?,
                cpu: row.get(5)?,
                ram: row.get(6)?,
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

    /// Раздел «Всё о пчёлах». Пустая метка значит «включён»: чужая нода выключает его сама.
    pub fn bees_page(&self) -> Result<bool, IndexError> {
        Ok(self.meta_get("bees_page")?.as_deref() != Some("0"))
    }

    pub fn set_bees_page(&self, on: bool) -> Result<(), IndexError> {
        self.meta_set("bees_page", if on { "1" } else { "0" })
    }

    /// Один раз ставит страницы с символом битой кодировки в ближайшее перечитывание
    /// и забывает метку сервера, чтобы ответ «не изменилась» не сохранил старый текст.
    pub fn queue_garbled_reread(&self) -> Result<usize, IndexError> {
        if self.meta_get("garbled_reread")?.as_deref() == Some("1") {
            return Ok(0);
        }
        let changed = self.conn.execute(
            "UPDATE pages SET next_check = strftime('%s','now'), etag = NULL, modified = NULL
             WHERE instr(title, char(65533)) > 0 OR instr(substr(text, 1, 800), char(65533)) > 0",
            [],
        )?;
        self.meta_set("garbled_reread", "1")?;
        Ok(changed)
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
                    "SELECT url,
                            CASE WHEN instr(title, char(65533)) > 0 THEN NULL ELSE etag END,
                            CASE WHEN instr(title, char(65533)) > 0 THEN NULL ELSE modified END
                     FROM pages
                     WHERE next_check <= strftime('%s','now') AND {NOT_VOLUNTEER}
                     ORDER BY (instr(title, char(65533)) > 0) DESC, next_check ASC, id ASC LIMIT ?1"
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

    pub fn passage_kind(&self) -> Result<Option<String>, IndexError> {
        self.meta_get("passage_kind")
    }

    pub fn set_passage_kind(&self, kind: &str) -> Result<(), IndexError> {
        self.meta_set("passage_kind", kind)
    }

    pub fn passage_after(&self) -> Result<i64, IndexError> {
        Ok(self
            .meta_get("passage_after")?
            .and_then(|value| value.parse().ok())
            .unwrap_or(0))
    }

    pub fn set_passage_after(&self, id: i64) -> Result<(), IndexError> {
        self.meta_set("passage_after", &id.to_string())
    }

    pub fn pages_for_refresh(&self, after: i64, limit: usize) -> Result<Vec<(i64, String, String)>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, text FROM pages WHERE id > ?1 ORDER BY id LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![after, limit as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn put_vector(
        &mut self,
        id: i64,
        vector: &QuantVector,
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
        let revision = next_revision(&tx)?;
        tx.execute(
            "UPDATE pages SET scale = ?1, vector = ?2, semantic_bucket = ?3, revision = ?4 WHERE id = ?5",
            params![vector.scale, blob, semantic_bucket as i64, revision, id],
        )?;
        tx.commit()?;
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
        self.table_hosts("muted_hosts")
    }

    pub fn keep_host(&self, host: &str) -> Result<(), IndexError> {
        self.insert_host("keep_hosts", host)
    }

    pub fn unkeep_host(&self, host: &str) -> Result<(), IndexError> {
        self.delete_host("keep_hosts", host)
    }

    pub fn keep_hosts(&self) -> Result<Vec<String>, IndexError> {
        self.table_hosts("keep_hosts")
    }

    pub fn is_keep_host(&self, host: &str) -> Result<bool, IndexError> {
        self.host_in_table("keep_hosts", host)
    }

    pub fn ban_host(&self, host: &str) -> Result<(), IndexError> {
        self.insert_host("banned_hosts", host)?;
        let host = bare_host(host);
        if host.is_empty() {
            return Ok(());
        }
        for prefix in ["https://", "http://"] {
            for name in [&host, &format!("www.{host}")] {
                self.conn.execute(
                    "UPDATE crawl_queue SET visited = 1 WHERE visited = 0 AND (url = ?1 OR url LIKE ?2)",
                    params![format!("{prefix}{name}/"), format!("{prefix}{name}/%")],
                )?;
            }
        }
        Ok(())
    }

    pub fn unban_host(&self, host: &str) -> Result<(), IndexError> {
        self.delete_host("banned_hosts", host)
    }

    pub fn banned_hosts(&self) -> Result<Vec<String>, IndexError> {
        self.table_hosts("banned_hosts")
    }

    pub fn is_banned_host(&self, host: &str) -> Result<bool, IndexError> {
        self.host_in_table("banned_hosts", host)
    }

    pub fn is_muted_host(&self, host: &str) -> Result<bool, IndexError> {
        self.host_in_table("muted_hosts", host)
    }

    pub fn inspect_host(&self, host: &str) -> Result<HostInspect, IndexError> {
        let host = if let Some(found) = host_of_url(host) {
            bare_host(&found)
        } else {
            bare_host(host.split(['/', '?', '#']).next().unwrap_or(""))
        };
        let (pages, queued) = self.host_counts(&host)?;
        let tool_docs = is_tool_docs_host(&host);
        Ok(HostInspect {
            host: host.clone(),
            pages,
            queued,
            banned: self.is_banned_host(&host)?,
            muted: self.is_muted_host(&host)?,
            keep: self.is_keep_host(&host)?,
            volunteer: self.is_volunteer_host(&host)?,
            government: is_government_host(&host),
            hive: is_hive_host(&host),
            tool_docs,
            model_maker: is_model_maker_host(&host) && !tool_docs,
            in_focus: host_in_focus(&host) || mds_types::is_channel_visit_host(&host),
        })
    }

    fn host_open(&self, host: &str) -> Result<bool, IndexError> {
        let bare = bare_host(host);
        Ok(self.is_volunteer_host(&bare)? || host_keeps_pages(&bare) || self.is_keep_host(&bare)?)
    }

    fn insert_host(&self, table: &str, host: &str) -> Result<(), IndexError> {
        let host = bare_host(host);
        if host.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            &format!("INSERT OR IGNORE INTO {table} (host) VALUES (?1)"),
            [host],
        )?;
        Ok(())
    }

    fn delete_host(&self, table: &str, host: &str) -> Result<(), IndexError> {
        let host = bare_host(host);
        self.conn.execute(&format!("DELETE FROM {table} WHERE host = ?1"), [host])?;
        Ok(())
    }

    fn table_hosts(&self, table: &str) -> Result<Vec<String>, IndexError> {
        let mut stmt = self.conn.prepare(&format!("SELECT host FROM {table} ORDER BY host"))?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn host_in_table(&self, table: &str, host: &str) -> Result<bool, IndexError> {
        let host = bare_host(host);
        if host.is_empty() {
            return Ok(false);
        }
        let n: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE host = ?1 OR host = ?2"),
            params![host, format!("www.{host}")],
            |row| row.get(0),
        )?;
        Ok(n > 0)
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

fn ensure_volunteer_version(conn: &Connection) -> Result<(), IndexError> {
    if table_column_exists(conn, "volunteers", "version")? {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE volunteers ADD COLUMN version TEXT NOT NULL DEFAULT ''",
        [],
    )
    .map_err(|source| IndexError::Sqlite {
        context: "volunteer",
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
            if is_model_inner(&url) || is_channel_noise(&url) || is_raw_office_text(&title, &text) {
                self.delete_id(id)?;
                labeled += 1;
                continue;
            }
            let fresh = kind_from_stored(&url, &title, &text);
            let kind = if is_press_host(&host) {
                KIND_NEWS
            } else if is_channel_host(&host) {
                fresh
            } else {
                let mut kind = fresh;
                if is_image_page(&url) {
                    kind |= stored & KIND_IMAGE;
                }
                if is_video_page(&url) {
                    kind |= stored & KIND_VIDEO;
                }
                kind
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
            if (is_press_host(&host) && !is_press_front(&url))
                || is_model_inner(&url)
                || is_channel_noise(&url)
                || is_document_url(&url)
            {
                self.conn.execute("DELETE FROM crawl_queue WHERE url = ?1", [&url])?;
            }
        }
        Ok(labeled)
    }

    fn harvest_channel_entities(&self) -> Result<usize, IndexError> {
        let mut stmt = self.conn.prepare("SELECT title, text FROM pages")?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        let mut urls = Vec::new();
        let mut seen = HashSet::new();
        for row in rows {
            let (title, text) = row?;
            let blob = format!("{title}\n{text}");
            let contacts = contacts_in_text(&blob);
            for link in contacts.messengers.iter().chain(contacts.socials.iter()) {
                if let Some(entity) = channel_entity_url(link) {
                    if seen.insert(entity.clone()) {
                        urls.push(entity);
                    }
                }
            }
            for token in blob.split_whitespace() {
                let token = token.trim_matches(|c: char| matches!(c, ',' | ';' | ')' | '(' | '"' | '\''));
                if let Some(entity) = channel_entity_url(token) {
                    if seen.insert(entity.clone()) {
                        urls.push(entity);
                    }
                }
            }
        }
        drop(stmt);
        if let Ok(mut stmt) = self.conn.prepare("SELECT messengers, socials FROM host_contacts") {
            if let Ok(rows) = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))) {
                for row in rows.flatten() {
                    let (messengers, socials) = row;
                    for field in [messengers, socials] {
                        for part in field.split([',', ' ', '\n']) {
                            if let Some(entity) = channel_entity_url(part.trim()) {
                                if seen.insert(entity.clone()) {
                                    urls.push(entity);
                                }
                            }
                        }
                    }
                }
            }
        }
        let n = urls.len();
        if !urls.is_empty() {
            self.enqueue_urls(&urls)?;
        }
        Ok(n)
    }

    pub fn accepts_more(&self, host: &str) -> Result<bool, IndexError> {
        let bare = bare_host(host);
        if self.host_open(&bare)? || is_channel_host(&bare) {
            return Ok(true);
        }
        Ok(self.host_pressure(&bare)? < HOST_PAGE_CAP)
    }

    pub fn may_store(&self, host: &str, url: &str) -> Result<bool, IndexError> {
        let bare = bare_host(host);
        if self.is_banned_host(&bare)? {
            return Ok(false);
        }
        if !url_in_focus(url) {
            return Ok(false);
        }
        if is_channel_item(url) {
            return Ok(false);
        }
        if is_press_host(&bare) && !is_press_front(url) {
            return Ok(false);
        }
        if is_model_inner(url) {
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
        if self.host_open(&bare)? || is_channel_host(&bare) {
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
        CREATE TABLE IF NOT EXISTS keep_hosts (
            host TEXT PRIMARY KEY
        );
        CREATE TABLE IF NOT EXISTS banned_hosts (
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
        );
        CREATE TABLE IF NOT EXISTS intake_tick (
            grain TEXT NOT NULL,
            bucket TEXT NOT NULL,
            new_pages INTEGER NOT NULL,
            changed_pages INTEGER NOT NULL,
            same_pages INTEGER NOT NULL,
            new_hosts INTEGER NOT NULL,
            shell_pages INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (grain, bucket)
        );
        CREATE TABLE IF NOT EXISTS load_minute (
            minute TEXT PRIMARY KEY,
            cpu_pct INTEGER NOT NULL,
            ram_pct INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS search_minute (
            minute TEXT PRIMARY KEY,
            count INTEGER NOT NULL,
            avg_ms INTEGER NOT NULL,
            max_ms INTEGER NOT NULL
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
    let tokens = content_tokens(query);
    if tokens.is_empty() {
        None
    } else {
        Some(fts_and(&tokens, false))
    }
}

fn content_tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(|token| token.to_string())
        .collect()
}

fn fts_atom(token: &str) -> String {
    format!("\"{token}\"")
}

fn fts_and(tokens: &[String], fold: bool) -> String {
    tokens
        .iter()
        .map(|token| {
            let other = swap_yo(token);
            if fold && other != *token {
                format!("({} OR {})", fts_atom(token), fts_atom(&other))
            } else {
                fts_atom(token)
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn fts_near(tokens: &[String], fold: bool) -> Option<String> {
    if tokens.len() < 2 {
        return None;
    }
    let mut parts = Vec::new();
    let mut changed = false;
    for token in tokens {
        let swapped = swap_yo(token);
        if fold && swapped != *token {
            changed = true;
            parts.push(fts_atom(&swapped));
        } else {
            parts.push(fts_atom(token));
        }
    }
    if fold && !changed {
        return None;
    }
    Some(format!("NEAR({}, 8)", parts.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mds_types::{quantize, EMBED_DIMS};

    fn page(url: &str, title: &str, text: &str) -> FetchedPage {
        FetchedPage {
            url: url.into(),
            host: "shop.ru".into(),
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
                    "https://shop.ru/bolt",
                    "Крепёж",
                    "Стальной крепёж MDSBOLT42 для каркаса",
                ),
                &quant,
                &[3; 16],
                1,
            )
            .unwrap();
        let hits = index.search_words("MDSBOLT42", 10, 0, 0).unwrap();
        assert_eq!(hits.len(), 1);
        let missed = index.search_words("как лечить собаку", 10, 0, 0).unwrap();
        assert!(missed.is_empty());
        let vec_hits = index.search_vectors(&vector, Some(1), 10, 0, 0).unwrap();
        assert_eq!(vec_hits.len(), 1);
        let far = index.search_vectors(&vector, Some(6), 10, 0, 0).unwrap();
        assert!(far.is_empty());
        let mut other = vec![0.0_f32; EMBED_DIMS];
        other[1] = 1.0;
        let other_quant = quantize(&other).unwrap();
        index
            .insert(
                &page(
                    "https://shop.ru/other",
                    "Другое",
                    "Соседняя страница без крепежа",
                ),
                &other_quant,
                &[3; 16],
                6,
            )
            .unwrap();
        let all = index.search_vectors(&vector, None, 10, 0, 0).unwrap();
        assert_eq!(all[0].id, vec_hits[0].id);
        assert!(all.iter().any(|hit| hit.cosine < 0.2));
        let rows = index.pages_for_refresh(0, 10).unwrap();
        assert_eq!(rows.len(), 2);
        index.put_vector(rows[1].0, &quant, 1).unwrap();
        let moved = index.search_vectors(&vector, Some(1), 10, 0, 0).unwrap();
        assert_eq!(moved.len(), 2);
        index
            .enqueue_urls(&["https://shop.ru/a".into(), "https://shop.ru/b".into()])
            .unwrap();
        index
            .mark_visited(&["https://shop.ru/a".into()])
            .unwrap();
        let pending = index.pending_urls(10).unwrap();
        assert_eq!(pending, vec!["https://shop.ru/b".to_string()]);
        index.enqueue_next(&["https://shop.ru/new".into()]).unwrap();
        let pending = index.pending_urls(10).unwrap();
        assert_eq!(pending[0], "https://shop.ru/new");
    }

    #[test]
    fn hundred_first_page_waits_for_a_bee() {
        let dir = std::env::temp_dir().join(format!("mds-index-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("cap.db")).unwrap();
        let urls: Vec<String> = (0..101).map(|n| format!("https://plant.ru/p{n}")).collect();
        index.enqueue_urls(&urls).unwrap();
        assert_eq!(index.queue_len().unwrap(), 100);
        index.remember_volunteer("plant.ru").unwrap();
        index.enqueue_urls(&["https://plant.ru/p100".into()]).unwrap();
        assert_eq!(index.queue_len().unwrap(), 101);
        index.note_webshop("shop.ru").unwrap();
        index
            .enqueue_urls(&[
                "https://shop.ru/product/bolt".into(),
                "https://shop.ru/about".into(),
                "https://www.ozon.ru/product/phone".into(),
                "https://www.ozon.ru/category/phones".into(),
            ])
            .unwrap();
        let pending = index.pending_urls(200).unwrap();
        assert!(pending.iter().any(|url| url.contains("shop.ru/product/")));
        assert!(pending.iter().any(|url| url.ends_with("/about")));
        assert!(pending.iter().any(|url| url.contains("/category/")));
        assert!(!pending.iter().any(|url| url.contains("ozon.ru/product/")));
    }

    #[test]
    fn government_and_hive_pages_are_not_capped() {
        let dir = std::env::temp_dir().join(format!("mds-index-gov-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("gov.db")).unwrap();
        let gov: Vec<String> = (0..101).map(|n| format!("https://fas.gov.ru/news/{n}")).collect();
        index.enqueue_urls(&gov).unwrap();
        assert_eq!(index.queue_len().unwrap(), 101);
        let hive: Vec<String> = (0..101).map(|n| format!("https://beesearch.ru/p{n}")).collect();
        index.enqueue_urls(&hive).unwrap();
        assert_eq!(index.queue_len().unwrap(), 202);
        let plant: Vec<String> = (0..101).map(|n| format!("https://plant.ru/p{n}")).collect();
        index.enqueue_urls(&plant).unwrap();
        assert_eq!(index.queue_len().unwrap(), 302);
        let docs: Vec<String> = (0..101).map(|n| format!("https://docs.openclaw.ai/plugins/{n}")).collect();
        index.enqueue_urls(&docs).unwrap();
        assert_eq!(index.queue_len().unwrap(), 403);
        let claude: Vec<String> = (0..101).map(|n| format!("https://docs.anthropic.com/en/api/{n}")).collect();
        index.enqueue_urls(&claude).unwrap();
        assert_eq!(index.queue_len().unwrap(), 504);
        index
            .enqueue_urls(&["https://chatgpt.com/c/secret".into(), "https://developers.sber.ru/docs/ru/gigachat/api/main".into()])
            .unwrap();
        let pending = index.pending_urls(600).unwrap();
        assert!(!pending.iter().any(|url| url.contains("chatgpt.com/c/")));
        assert!(pending.iter().any(|url| url.contains("developers.sber.ru")));
    }

    #[test]
    fn channel_entities_skip_the_host_cap_and_items_stay_out() {
        let dir = std::env::temp_dir().join(format!("mds-index-ch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("ch.db")).unwrap();
        let urls: Vec<String> = (0..101).map(|n| format!("https://max.ru/channel{n}")).collect();
        index.enqueue_urls(&urls).unwrap();
        assert_eq!(index.queue_len().unwrap(), 101);
        index
            .enqueue_urls(&[
                "https://t.me/clinic_msk".into(),
                "https://t.me/c/1".into(),
                "https://rutube.ru/video/abc".into(),
                "https://rutube.ru/u/stident/".into(),
                "https://max.ru/".into(),
            ])
            .unwrap();
        let pending = index.pending_urls(300).unwrap();
        assert!(pending.iter().any(|url| url.contains("t.me/clinic_msk")));
        assert!(!pending.iter().any(|url| url.contains("t.me/c/")));
        assert!(!pending.iter().any(|url| url.contains("/video/")));
        assert!(pending.iter().any(|url| url.contains("rutube.ru/u/stident")));
        assert!(!pending.iter().any(|url| url == "https://max.ru" || url.ends_with("://max.ru/")));
    }

    #[test]
    fn kind_pass_drops_channel_items_and_harvests_entities() {
        let dir = std::env::temp_dir().join(format!("mds-index-ch2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("ch2.db");
        let mut index = Index::open(&db).unwrap();
        let mut vector = vec![0.0_f32; EMBED_DIMS];
        vector[0] = 1.0;
        let quant = quantize(&vector).unwrap();
        let hash = [0_u8; 16];
        let mut video = page(
            "https://rutube.ru/video/abc/",
            "ролик",
            "смотреть видео",
        );
        video.host = "rutube.ru".into();
        let mut chan = page(
            "https://rutube.ru/u/stident/",
            "S.T.I. Dent",
            "канал стоматологической практики",
        );
        chan.host = "rutube.ru".into();
        let mut site = page(
            "https://klinika.ru/",
            "Клиника",
            "Пишите https://t.me/clinic_msk и https://max.ru/clinic_bot",
        );
        site.host = "klinika.ru".into();
        index.insert(&video, &quant, &hash, 0).unwrap();
        index.insert(&chan, &quant, &hash, 0).unwrap();
        index.insert(&site, &quant, &hash, 0).unwrap();
        index.meta_set("kind_pass", "130").unwrap();
        drop(index);
        let index = Index::open(&db).unwrap();
        assert_eq!(index.len().unwrap(), 2);
        let pending = index.pending_urls(20).unwrap();
        assert!(pending.iter().any(|url| url.contains("t.me/clinic_msk")));
        assert!(pending.iter().any(|url| url.contains("max.ru/clinic_bot")));
        assert!(!pending.iter().any(|url| url.contains("/video/")));
    }

    #[test]
    fn operator_keep_lifts_the_cap_and_ban_skips_the_queue() {
        let dir = std::env::temp_dir().join(format!("mds-index-lists-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("lists.db")).unwrap();
        index.keep_host("plant.ru").unwrap();
        let plant: Vec<String> = (0..101).map(|n| format!("https://plant.ru/p{n}")).collect();
        index.enqueue_urls(&plant).unwrap();
        assert_eq!(index.queue_len().unwrap(), 101);
        index.ban_host("spam.ru").unwrap();
        index
            .enqueue_urls(&["https://spam.ru/a".into(), "https://plant.ru/p102".into()])
            .unwrap();
        let pending = index.pending_urls(200).unwrap();
        assert!(!pending.iter().any(|url| url.contains("spam.ru")));
        assert!(pending.iter().any(|url| url.ends_with("/p102")));
        let kept = index.inspect_host("https://www.plant.ru/a").unwrap();
        assert_eq!(kept.host, "plant.ru");
        assert!(kept.keep);
        assert!(kept.queued >= 101);
        assert_eq!(kept.rule(), "без потолка");
        let banned = index.inspect_host("spam.ru").unwrap();
        assert!(banned.banned);
        assert_eq!(banned.rule(), "не обходить");
        let docs = index.inspect_host("docs.coze.com").unwrap();
        assert!(docs.tool_docs);
        assert_eq!(docs.rule(), "без потолка");
        let maker = index.inspect_host("openai.com").unwrap();
        assert!(maker.model_maker);
        assert_eq!(maker.rule(), "только главная");
    }

    #[test]
    fn listed_card_pages_replace_the_other_queue() {
        let dir = std::env::temp_dir().join(format!("mds-index-picks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("picks.db")).unwrap();
        index.enqueue_urls(&["https://zavod.ru/random".into()]).unwrap();
        index
            .set_priority("www.zavod.ru", &["https://zavod.ru/keep".into()])
            .unwrap();
        let pending = index.pending_urls(20).unwrap();
        assert!(pending.iter().any(|url| url.ends_with("/keep")));
        assert!(!pending.iter().any(|url| url.ends_with("/random")));
        index.enqueue_urls(&["https://zavod.ru/other".into()]).unwrap();
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
        let (day, week, month) = index.visit_windows().unwrap();
        assert_eq!(day.len(), 10);
        assert!(week <= day);
        assert!(month <= day);
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
        index.note_search_pace(120).unwrap();
        index.note_search_pace(400).unwrap();
        let pace = index.search_pace(5).unwrap();
        assert_eq!(pace.len(), 1);
        assert_eq!(pace[0].count, 2);
        assert_eq!(pace[0].max_ms, 400);
        assert_eq!(index.search_peak_ms(3).unwrap(), 400);
        let load = index.load_hours(2).unwrap();
        assert_eq!(load.len(), 1);
        assert_eq!(load[0].cpu_pct, 20);
        assert_eq!(load[0].ram_pct, 50);
        let minute = index.activity_span("minute", 60).unwrap();
        assert_eq!(minute.len(), 1);
        assert_eq!(minute[0].new_pages, 5);
        assert_eq!(minute[0].new_hosts, 1);
        assert_eq!(minute[0].walked, 10);
        assert_eq!(minute[0].shell_pages, 3);
        assert_eq!(minute[0].cpu, 30);
        assert_eq!(minute[0].ram, 60);
        let days = index.activity_span("day", 30).unwrap();
        assert!(!days.is_empty());
        assert_eq!(days.last().unwrap().new_pages, 5);
        index.mute_host("www.Example.test").unwrap();
        assert_eq!(index.muted_hosts().unwrap(), vec!["example.test".to_string()]);
        index.unmute_host("example.test").unwrap();
        assert!(index.muted_hosts().unwrap().is_empty());
        let reader = Index::open_read(&dir.join("intake.db")).unwrap();
        assert_eq!(reader.intake_sum("new_pages", "day").unwrap(), 5);
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
            .enqueue_urls(&["https://one.ru/a".into(), "https://two.ru/b".into()])
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
    fn claim_takes_two_from_a_giant_and_lets_small_hosts_in() {
        let dir = std::env::temp_dir().join(format!("mds-index-fair-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("fair.db")).unwrap();
        let mut urls = Vec::new();
        for i in 0..20 {
            urls.push(format!("https://giant.ru/{i}"));
        }
        for i in 0..8 {
            urls.push(format!("https://plant{i}.ru/about"));
        }
        index.enqueue_urls(&urls).unwrap();
        let claimed = index.claim_urls(8, 600).unwrap();
        assert_eq!(claimed.len(), 8);
        let giant = claimed.iter().filter(|url| url.contains("giant.ru")).count();
        assert_eq!(giant, 2);
        let plants = claimed.iter().filter(|url| url.contains("plant")).count();
        assert_eq!(plants, 6);
    }

    #[test]
    fn model_chat_stays_out_of_the_queue() {
        let dir = std::env::temp_dir().join(format!("mds-index-model-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("model.db")).unwrap();
        index
            .enqueue_urls(&[
                "https://chatgpt.com/".into(),
                "https://chatgpt.com/c/abc".into(),
                "https://chatgpt.com/?hints=search&q=Read%20from%20https://docs.openclaw.ai/index.md".into(),
                "https://claude.ai/chat/1".into(),
                "https://claude.ai/new?q=Read%20from%20https://docs.openclaw.ai/index.md".into(),
            ])
            .unwrap();
        let pending = index.pending_urls(10).unwrap();
        assert_eq!(pending, vec!["https://chatgpt.com/".to_string()]);
        index.remember_volunteer_version("dartalab.ru", "0.1.1").unwrap();
        let row = &index.volunteer_seen().unwrap()[0];
        assert_eq!(row.host, "dartalab.ru");
        assert_eq!(row.version, "0.1.1");
        assert!(row.seen_at > 0);
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
        let mut known = page("https://good.ru/", "Известный", "Уже лежит в индексе");
        known.host = "good.ru".into();
        index.insert(&known, &quant, &[2; 16], 1).unwrap();
        index
            .enqueue_urls(&["https://good.ru/next".into(), "https://cold.ru/a".into()])
            .unwrap();
        index.note_host_quiet("mute.ru").unwrap();
        index.enqueue_urls(&["https://mute.ru/z".into()]).unwrap();
        index.enqueue_next(&["https://now.ru/top".into()]).unwrap();
        let urls = index.claim_urls(3, 600).unwrap();
        assert_eq!(
            urls,
            vec![
                "https://now.ru/top".to_string(),
                "https://good.ru/next".to_string(),
                "https://cold.ru/a".to_string(),
            ]
        );
        assert_eq!(index.pending_urls(10).unwrap(), vec!["https://mute.ru/z".to_string()]);
    }

    #[test]
    fn anchor_words_find_a_page_that_does_not_say_them() {
        let dir = std::env::temp_dir().join(format!("mds-index-anchor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut index = Index::open(&dir.join("anchor.db")).unwrap();
        let mut vector = vec![0.0_f32; EMBED_DIMS];
        vector[0] = 1.0;
        let quant = quantize(&vector).unwrap();
        let stored = page("https://shop.ru/jar", "Банка", "Стеклянная тара с пасеки");
        index.remember_anchor("https://shop.ru/jar", "мёд").unwrap();
        index.insert(&stored, &quant, &[4; 16], 1).unwrap();
        let hits = index.search_words("мёд", 10, 0, 0).unwrap();
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

    #[test]
    fn a_fresh_link_moves_ahead_of_the_cold_queue() {
        let dir = std::env::temp_dir().join(format!("mds-index-ahead-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("ahead.db")).unwrap();
        index.enqueue_urls(&["https://old.ru/a".into()]).unwrap();
        index.enqueue_urls(&["https://new.ru/b".into()]).unwrap();
        index.pull_ahead(&["https://new.ru/b".into()]).unwrap();
        let pending = index.pending_urls(2).unwrap();
        assert_eq!(pending[0], "https://new.ru/b");
    }

    #[test]
    fn a_nearby_phrase_outranks_the_same_words_apart_and_the_yo_spelling() {
        let dir = std::env::temp_dir().join(format!("mds-index-phrase-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut index = Index::open(&dir.join("phrase.db")).unwrap();
        let mut vector = vec![0.0_f32; EMBED_DIMS];
        vector[0] = 1.0;
        let quant = quantize(&vector).unwrap();
        let near = page("https://paseka.ru/honey", "Липовый мёд", "Липовый мёд с пасеки в Лотошине");
        let swapped = page("https://paseka.ru/med", "Липовый мед", "В продаже липовый мед оптом");
        let far = page(
            "https://paseka.ru/far",
            "Каталог",
            &format!("Липовый {} мёд", "раздел ".repeat(20)),
        );
        index.insert(&near, &quant, &[3; 16], 1).unwrap();
        index.insert(&swapped, &quant, &[3; 16], 1).unwrap();
        index.insert(&far, &quant, &[3; 16], 1).unwrap();
        let hits = index.search_words("мёд липовый", 10, 0, 0).unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].id, index_id(&index, "https://paseka.ru/honey"));
        assert_eq!(hits[1].id, index_id(&index, "https://paseka.ru/med"));
        assert_eq!(hits[2].id, index_id(&index, "https://paseka.ru/far"));
    }

    #[test]
    fn focus_drops_foreign_pages_and_keeps_a_model_maker() {
        let dir = std::env::temp_dir().join(format!("mds-index-focus-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut index = Index::open(&dir.join("focus.db")).unwrap();
        let mut vector = vec![0.0_f32; EMBED_DIMS];
        vector[0] = 1.0;
        let quant = quantize(&vector).unwrap();
        let mut docs = page("https://docs.example.co.uk/readme", "Git", "Developer documentation");
        docs.host = "docs.example.co.uk".into();
        let mut maker = page("https://openai.com/api", "API", "Модель и сервис");
        maker.host = "openai.com".into();
        let mut local = page("https://paseka.ru/", "Пасека", "Мёд");
        local.host = "paseka.ru".into();
        index.insert(&docs, &quant, &[3; 16], 1).unwrap();
        index.insert(&maker, &quant, &[3; 16], 1).unwrap();
        index.insert(&local, &quant, &[3; 16], 1).unwrap();
        index
            .conn
            .execute(
                "INSERT INTO crawl_queue (url, visited, added_at, worth) VALUES ('https://docs.example.co.uk/a', 0, 1, 0)",
                [],
            )
            .unwrap();
        index
            .conn
            .execute(
                "INSERT INTO crawl_queue (url, visited, added_at, worth) VALUES ('https://paseka.ru/a', 0, 1, 0)",
                [],
            )
            .unwrap();
        let swept = index.sweep_focus().unwrap();
        assert_eq!(swept.dropped, 1);
        assert_eq!(index.len().unwrap(), 2);
        assert!(index.search_words("Developer", 10, 0, 0).unwrap().is_empty());
        assert_eq!(index.pending_urls(10).unwrap(), vec!["https://paseka.ru/a".to_string()]);
    }

    fn index_id(index: &Index, url: &str) -> i64 {
        index
            .conn
            .query_row("SELECT id FROM pages WHERE url = ?1", [url], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_long_queue_does_not_grow() {
        assert!(queue_may_grow_len(0));
        assert!(queue_may_grow_len(49_999));
        assert!(!queue_may_grow_len(50_000));
        assert!(!queue_may_grow_len(725_000));
    }

    #[test]
    fn an_empty_index_may_grow_the_queue() {
        let dir = std::env::temp_dir().join(format!("mds-index-grow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::open(&dir.join("grow.db")).unwrap();
        assert!(index.queue_may_grow().unwrap());
    }
}
