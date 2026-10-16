use std::collections::HashMap;
use std::path::Path;

use mds_crawl::{is_broken_url, is_near_duplicate, is_not_found, strip_boilerplate, FetchedPage};
use mds_types::{
    bare_host, host_of_url, is_channel_host, is_press_front, is_press_host, is_product_url, kind_from_stored, cosine,
    QuantVector, EMBED_DIMS, HOST_PAGE_CAP, KIND_CHANNEL, KIND_NEWS, KIND_VIDEO,
};
use rusqlite::{params, Connection};
use thiserror::Error;

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

#[derive(Clone, Debug)]
pub struct PageRow {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub host: String,
    pub text: String,
    pub region: Option<String>,
    pub kind: u32,
    pub media_url: Option<String>,
    pub feed_url: Option<String>,
    pub price: Option<String>,
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
        let index = Self { conn };
        let _ = index.fill_kinds()?;
        let (labeled, total) = index.fill_regions()?;
        if total > 0 {
            eprintln!("MDS: регион подписан у {labeled} страниц из {total}");
        }
        Ok(index)
    }

    pub fn fill_regions(&self) -> Result<(usize, usize), IndexError> {
        let mut stmt = self.conn.prepare("SELECT id, title, text FROM pages")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        let mut labeled = 0;
        let mut total = 0;
        let mut pending = Vec::new();
        for row in rows {
            let (id, title, text) = row?;
            total += 1;
            let region = mds_types::region_in(&format!("{title}\n{text}")).map(str::to_string);
            if region.is_some() {
                labeled += 1;
            }
            pending.push((region, id));
        }
        drop(stmt);
        for (region, id) in pending {
            self.conn
                .execute("UPDATE pages SET region = ?1 WHERE id = ?2", params![region, id])?;
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
        let region = mds_types::region_in(&format!("{}\n{}", page.title, page.text));
        tx.execute(
            "INSERT INTO pages (url, title, host, text, simhash, scale, vector, domain_hash, semantic_bucket, region, kind, media_url, feed_url, price)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
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
                kind=excluded.kind,
                media_url=excluded.media_url,
                feed_url=excluded.feed_url,
                price=excluded.price",
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
                page.kind,
                page.media_url,
                page.feed_url,
                page.price,
            ],
        )?;
        let id: i64 = tx.query_row("SELECT id FROM pages WHERE url = ?1", [&page.url], |row| row.get(0))?;
        tx.execute("DELETE FROM pages_fts WHERE rowid = ?1", [id])?;
        tx.execute(
            "INSERT INTO pages_fts(rowid, title, text) VALUES (?1, ?2, ?3)",
            params![id, page.title, page.text],
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
            let trimmed = url.trim();
            if trimmed.is_empty() || is_broken_url(trimmed) {
                continue;
            }
            let Some(host) = host_of_url(trimmed) else {
                continue;
            };
            let bare = bare_host(&host);
            if self.is_webshop_host(&bare)? && is_product_url(trimmed) {
                continue;
            }
            if is_press_host(&bare) && !is_press_front(trimmed) {
                continue;
            }
            let volunteer = self.is_volunteer_host(&bare)?;
            if self.has_picks(&bare)? && !volunteer && !self.is_pick(&bare, trimmed)? {
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
                pressure.insert(bare, load + 1);
            }
            self.conn.execute(
                "INSERT OR IGNORE INTO crawl_queue (url, visited, added_at) VALUES (?1, 0, strftime('%s','now'))",
                params![trimmed],
            )?;
        }
        Ok(())
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
                "INSERT INTO crawl_queue (url, visited, added_at) VALUES (?1, 0, 20)
                 ON CONFLICT(url) DO UPDATE SET visited = 0, added_at = 20",
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

    fn is_webshop_host(&self, bare: &str) -> Result<bool, IndexError> {
        let found: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM webshops WHERE host = ?1",
            [bare],
            |row| row.get(0),
        )?;
        Ok(found > 0)
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
                    "INSERT INTO crawl_queue (url, visited, added_at) VALUES (?1, 0, 40)
                     ON CONFLICT(url) DO UPDATE SET visited = 0, added_at = 40",
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
        let mut stmt = self
            .conn
            .prepare("SELECT url FROM crawl_queue WHERE visited = 0 ORDER BY added_at ASC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn pages_by_ids(&self, ids: &[i64]) -> Result<Vec<PageRow>, IndexError> {
        let mut out = Vec::new();
        for id in ids {
            let row = self.conn.query_row(
                "SELECT id, url, title, host, text, region, kind, media_url, feed_url, price FROM pages WHERE id = ?1",
                [id],
                |row| {
                    Ok(PageRow {
                        id: row.get(0)?,
                        url: row.get(1)?,
                        title: row.get(2)?,
                        host: row.get(3)?,
                        text: row.get(4)?,
                        region: row.get(5)?,
                        kind: row.get(6)?,
                        media_url: row.get(7)?,
                        feed_url: row.get(8)?,
                        price: row.get(9)?,
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
    let mut stmt = conn.prepare("PRAGMA table_info(pages)").map_err(|source| IndexError::Sqlite {
        context: "region",
        source,
    })?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|source| IndexError::Sqlite {
            context: "region",
            source,
        })?;
    for name in names {
        if name? == "region" {
            return Ok(());
        }
    }
    conn.execute("ALTER TABLE pages ADD COLUMN region TEXT", [])
        .map_err(|source| IndexError::Sqlite {
            context: "region",
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
    ] {
        if column_exists(conn, name)? {
            continue;
        }
        conn.execute(ddl, []).map_err(|source| IndexError::Sqlite {
            context: "kind",
            source,
        })?;
    }
    Ok(())
}

fn column_exists(conn: &Connection, name: &str) -> Result<bool, IndexError> {
    let mut stmt = conn.prepare("PRAGMA table_info(pages)").map_err(|source| IndexError::Sqlite {
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
            } else if stored == 0 {
                fresh
            } else {
                stored | (fresh & KIND_NEWS)
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
            indexable: true,
            kind: 0,
            media_url: None,
            feed_url: None,
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
            .enqueue_urls(&["https://shop.example/product/bolt".into(), "https://shop.example/about".into()])
            .unwrap();
        let pending = index.pending_urls(200).unwrap();
        assert!(!pending.iter().any(|url| url.contains("/product/")));
        assert!(pending.iter().any(|url| url.ends_with("/about")));
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
}
