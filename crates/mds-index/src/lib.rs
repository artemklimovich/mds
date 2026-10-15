use std::path::Path;

use mds_crawl::{is_near_duplicate, FetchedPage};
use mds_types::{cosine, QuantVector, EMBED_DIMS};
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
        Ok(Self { conn })
    }

    pub fn len(&self) -> Result<usize, IndexError> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))?;
        Ok(count as usize)
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
        tx.execute(
            "INSERT INTO pages (url, title, host, text, simhash, scale, vector, domain_hash, semantic_bucket)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(url) DO UPDATE SET
                title=excluded.title,
                host=excluded.host,
                text=excluded.text,
                simhash=excluded.simhash,
                scale=excluded.scale,
                vector=excluded.vector,
                domain_hash=excluded.domain_hash,
                semantic_bucket=excluded.semantic_bucket",
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

    pub fn search_words(&self, query: &str, limit: usize) -> Result<Vec<WordMatch>, IndexError> {
        let Some(match_query) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT pages.id
             FROM pages_fts
             JOIN pages ON pages.id = pages_fts.rowid
             WHERE pages_fts MATCH ?1
             ORDER BY bm25(pages_fts)
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![match_query, limit as i64], |row| row.get::<_, i64>(0))?;
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
    ) -> Result<Vec<VectorMatch>, IndexError> {
        let mut scored = match bucket {
            Some(bucket) => self.score_bucket_neighbors(query, bucket)?,
            None => Vec::new(),
        };
        if scored.len() < limit {
            scored = self.score_all_vectors(query)?;
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
    ) -> Result<Vec<(i64, f32)>, IndexError> {
        let mut b = [0_i64; 17];
        b[0] = bucket as i64;
        for i in 0..16 {
            b[i + 1] = (bucket ^ (1 << i)) as i64;
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, scale, vector FROM pages
             WHERE semantic_bucket IN (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
        )?;
        let rows = stmt.query_map(
            params![
                b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8],
                b[9], b[10], b[11], b[12], b[13], b[14], b[15], b[16]
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

    fn score_all_vectors(&self, query: &[f32]) -> Result<Vec<(i64, f32)>, IndexError> {
        let mut stmt = self.conn.prepare("SELECT id, scale, vector FROM pages")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f32>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        score_rows(query, rows)
    }

    pub fn enqueue_urls(&self, urls: &[String]) -> Result<(), IndexError> {
        for url in urls {
            let trimmed = url.trim();
            if trimmed.is_empty() {
                continue;
            }
            self.conn.execute(
                "INSERT OR IGNORE INTO crawl_queue (url, visited, added_at) VALUES (?1, 0, strftime('%s','now'))",
                params![trimmed],
            )?;
        }
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
                "SELECT id, url, title, host, text FROM pages WHERE id = ?1",
                [id],
                |row| {
                    Ok(PageRow {
                        id: row.get(0)?,
                        url: row.get(1)?,
                        title: row.get(2)?,
                        host: row.get(3)?,
                        text: row.get(4)?,
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
        let trimmed = query.trim();
        if trimmed.is_empty() || trimmed.chars().count() > 300 {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO query_log (query, asked_at, result_count, word_hits, meaning_hits, adult_count)
             VALUES (?1, strftime('%s','now'), ?2, ?3, ?4, ?5)",
            params![trimmed, results, words, meaning, adult],
        )?;
        self.conn.execute(
            "DELETE FROM query_log WHERE id NOT IN (
                SELECT id FROM query_log ORDER BY id DESC LIMIT 500
             )",
            [],
        )?;
        Ok(())
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
}

#[derive(Clone, Debug)]
pub struct QueryStat {
    pub text: String,
    pub results: i64,
    pub words: i64,
    pub meaning: i64,
    pub adult: i64,
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
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS query_log (
            id INTEGER PRIMARY KEY,
            query TEXT NOT NULL,
            asked_at INTEGER NOT NULL,
            result_count INTEGER NOT NULL,
            word_hits INTEGER NOT NULL,
            meaning_hits INTEGER NOT NULL,
            adult_count INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS volunteers (
            host TEXT PRIMARY KEY,
            seen_at INTEGER NOT NULL
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
        let hits = index.search_words("MDSBOLT42", 10).unwrap();
        assert_eq!(hits.len(), 1);
        let missed = index.search_words("как лечить собаку", 10).unwrap();
        assert!(missed.is_empty());
        let vec_hits = index.search_vectors(&vector, Some(1), 10).unwrap();
        assert_eq!(vec_hits.len(), 1);
        index
            .enqueue_urls(&["https://shop.example/a".into(), "https://shop.example/b".into()])
            .unwrap();
        index
            .mark_visited(&["https://shop.example/a".into()])
            .unwrap();
        let pending = index.pending_urls(10).unwrap();
        assert_eq!(pending, vec!["https://shop.example/b".to_string()]);
    }
}
