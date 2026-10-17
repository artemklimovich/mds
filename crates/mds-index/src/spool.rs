use std::path::Path;

use rusqlite::{params, Connection};

use crate::IndexError;

/// Текст страницы, который ещё не встроен в поисковый файл.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParkedPage {
    pub url: String,
    pub host: String,
    pub title: String,
    pub text: String,
    pub simhash: u64,
    pub kind: u32,
    pub media_url: Option<String>,
    pub feed_url: Option<String>,
    pub news_feed: Option<String>,
    pub price: Option<String>,
    /// Изменённая страница пишется поверх. Новый обход близкий дубль пропускает.
    pub force: bool,
}

/// Отдельный файл очереди текстов. Поиск его не читает.
pub struct TextSpool {
    conn: Connection,
}

impl TextSpool {
    pub fn open(path: &Path) -> Result<Self, IndexError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        let conn = Connection::open(path).map_err(|source| IndexError::Sqlite {
            context: "spool",
            source,
        })?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS spool (
                url TEXT PRIMARY KEY,
                host TEXT NOT NULL,
                title TEXT NOT NULL,
                text TEXT NOT NULL,
                simhash INTEGER NOT NULL,
                kind INTEGER NOT NULL,
                media_url TEXT,
                feed_url TEXT,
                news_feed TEXT,
                price TEXT,
                force INTEGER NOT NULL DEFAULT 0
             );",
        )
        .map_err(|source| IndexError::Sqlite {
            context: "spool",
            source,
        })?;
        Ok(Self { conn })
    }

    pub fn park(&self, pages: &[ParkedPage]) -> Result<usize, IndexError> {
        let mut stored = 0;
        for page in pages {
            if page.url.trim().is_empty() || page.text.trim().is_empty() {
                continue;
            }
            self.conn.execute(
                "INSERT INTO spool (url, host, title, text, simhash, kind, media_url, feed_url, news_feed, price, force)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(url) DO UPDATE SET
                    host=excluded.host,
                    title=excluded.title,
                    text=excluded.text,
                    simhash=excluded.simhash,
                    kind=excluded.kind,
                    media_url=excluded.media_url,
                    feed_url=excluded.feed_url,
                    news_feed=excluded.news_feed,
                    price=excluded.price,
                    force=excluded.force",
                params![
                    page.url,
                    page.host,
                    page.title,
                    page.text,
                    page.simhash as i64,
                    page.kind,
                    page.media_url,
                    page.feed_url,
                    page.news_feed,
                    page.price,
                    page.force as i64,
                ],
            )?;
            stored += 1;
        }
        Ok(stored)
    }

    pub fn take(&self, limit: usize) -> Result<Vec<ParkedPage>, IndexError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT url, host, title, text, simhash, kind, media_url, feed_url, news_feed, price, force
             FROM spool ORDER BY rowid ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(ParkedPage {
                url: row.get(0)?,
                host: row.get(1)?,
                title: row.get(2)?,
                text: row.get(3)?,
                simhash: row.get::<_, i64>(4)? as u64,
                kind: row.get(5)?,
                media_url: row.get(6)?,
                feed_url: row.get(7)?,
                news_feed: row.get(8)?,
                price: row.get(9)?,
                force: row.get::<_, i64>(10)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn ack(&self, urls: &[String]) -> Result<(), IndexError> {
        for url in urls {
            self.conn.execute("DELETE FROM spool WHERE url = ?1", [url.as_str()])?;
        }
        Ok(())
    }

    pub fn len(&self) -> Result<usize, IndexError> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM spool", [], |row| row.get(0))?;
        Ok(count as usize)
    }
}

pub fn spool_path(db: &Path) -> std::path::PathBuf {
    let name = db.file_name().and_then(|item| item.to_str()).unwrap_or("mds.db");
    db.with_file_name(format!("{name}.spool"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parked_page_is_taken_once() {
        let dir = std::env::temp_dir().join(format!("mds-spool-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let spool = TextSpool::open(&dir.join("t.db.spool")).unwrap();
        spool
            .park(&[ParkedPage {
                url: "https://good.example/a".into(),
                host: "good.example".into(),
                title: "Мёд".into(),
                text: "Банка мёда".into(),
                simhash: 7,
                kind: 0,
                media_url: None,
                feed_url: None,
                news_feed: None,
                price: None,
                force: false,
            }])
            .unwrap();
        let first = spool.take(8).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].title, "Мёд");
        spool.ack(&["https://good.example/a".into()]).unwrap();
        assert!(spool.take(8).unwrap().is_empty());
    }
}
