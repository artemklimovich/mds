use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mds_crawl::{is_broken_url, is_not_found, strip_boilerplate, CrawlPolicy, Crawler, FetchedPage};
use mds_dht::{shard_key, LocalDirectory, Lsh};
use mds_embed::Embedder;
use mds_index::{Index, QueryStat};
use mds_rank::{fuse, page_lane, query_lane, Lane};
use mds_types::{quantize, Hit, QuantVector};
use mds_wire::SearchRequest;

use crate::NodeError;

const CACHE_TTL: Duration = Duration::from_secs(300);
const CACHE_MAX: usize = 64;

struct CachedQuery {
    hits: Vec<Hit>,
    vector: QuantVector,
    saved_at: Instant,
}

pub struct Engine {
    index: Index,
    embedder: Embedder,
    lsh: Lsh,
    directory: LocalDirectory,
    cache: HashMap<String, CachedQuery>,
    db: PathBuf,
    load: SearchLoad,
}

struct SearchLoad {
    marks: VecDeque<(Instant, u128)>,
}

impl SearchLoad {
    fn note(&mut self, ms: u128) {
        self.prune();
        self.marks.push_back((Instant::now(), ms));
    }

    fn prune(&mut self) {
        let now = Instant::now();
        while self
            .marks
            .front()
            .is_some_and(|mark| now.duration_since(mark.0) > Duration::from_secs(60))
        {
            self.marks.pop_front();
        }
    }

    fn snapshot(&mut self) -> (u64, u64) {
        self.prune();
        let count = self.marks.len() as u64;
        let avg = if count == 0 {
            0
        } else {
            (self.marks.iter().map(|mark| mark.1).sum::<u128>() / u128::from(count)) as u64
        };
        (count, avg)
    }
}

impl Engine {
    pub fn open(db: &Path, models: &Path) -> Result<Self, NodeError> {
        eprintln!("MDS: модель эмбеддингов, каталог {}", models.display());
        let index = Index::open(db)?;
        let removed = index.scrub_stored_pages()?;
        if removed > 0 {
            eprintln!("MDS: из индекса убрано {removed} страниц без адреса, с ответом «не найдено» или со склеенной ссылкой");
        }
        Ok(Self {
            index,
            embedder: Embedder::open(models)?,
            lsh: Lsh::new(),
            directory: LocalDirectory::single(),
            cache: HashMap::new(),
            db: db.to_path_buf(),
            load: SearchLoad {
                marks: VecDeque::new(),
            },
        })
    }

    pub fn len(&self) -> Result<usize, NodeError> {
        Ok(self.index.len()?)
    }

    pub fn census(&self) -> Result<(usize, usize), NodeError> {
        Ok(self.index.census()?)
    }

    pub fn has_pending_urls(&self) -> Result<bool, NodeError> {
        Ok(!self.index.pending_urls(1)?.is_empty())
    }

    pub fn enqueue_urls(&self, urls: &[String]) -> Result<(), NodeError> {
        self.index.enqueue_urls(urls)?;
        Ok(())
    }

    pub fn enqueue_next(&self, urls: &[String]) -> Result<(), NodeError> {
        self.index.enqueue_next(urls)?;
        Ok(())
    }

    pub fn pending_urls(&self, limit: usize) -> Result<Vec<String>, NodeError> {
        Ok(self.index.pending_urls(limit)?)
    }

    pub fn record_crawl_step(
        &mut self,
        visited: &[String],
        discovered: &[String],
        gone: &[String],
        pages: Vec<FetchedPage>,
    ) -> Result<usize, NodeError> {
        self.index.mark_visited(visited)?;
        self.remember_priorities(&pages)?;
        self.index.enqueue_urls(discovered)?;
        self.index.delete_urls(gone)?;
        self.cache.clear();
        self.ingest(pages)
    }

    pub async fn refresh_urls(&mut self, urls: &[String], policy: &CrawlPolicy) -> Result<usize, NodeError> {
        let mut seed_policy = policy.clone();
        seed_policy.max_pages = urls.len().max(1);
        let mut crawler = Crawler::new(seed_policy)?;
        let pages = crawler.crawl(urls).await?;
        let gone = crawler.take_gone();
        let mut discovered = Vec::new();
        for page in &pages {
            discovered.extend(page.links.iter().cloned());
        }
        self.remember_priorities(&pages)?;
        self.index.enqueue_urls(&discovered)?;
        self.index.delete_urls(&gone)?;
        self.cache.clear();
        self.ingest_upsert(pages)
    }

    pub async fn index_urls(&mut self, urls: &[String], policy: &CrawlPolicy) -> Result<usize, NodeError> {
        self.index.enqueue_urls(urls)?;
        let pending = self.index.pending_urls(policy.max_pages.max(urls.len()))?;
        let seeds = if pending.is_empty() {
            urls.to_vec()
        } else {
            pending
        };
        let mut crawler = Crawler::new(policy.clone())?;
        let pages = crawler.crawl(&seeds).await?;
        let gone = crawler.take_gone();
        let mut visited: Vec<String> = pages.iter().map(|p| p.url.clone()).collect();
        visited.extend(seeds);
        let mut discovered = Vec::new();
        for page in &pages {
            discovered.extend(page.links.iter().cloned());
        }
        self.index.mark_visited(&visited)?;
        self.remember_priorities(&pages)?;
        self.index.enqueue_urls(&discovered)?;
        self.index.delete_urls(&gone)?;
        self.cache.clear();
        self.ingest(pages)
    }

    pub fn search(&mut self, query: &str) -> Result<Vec<Hit>, NodeError> {
        let (hits, _) = self.search_with_vector(query, 0)?;
        Ok(hits)
    }

    pub fn search_with_vector(&mut self, query: &str, kind_mask: u32) -> Result<(Vec<Hit>, QuantVector), NodeError> {
        let key = format!("{kind_mask}:{}", query.trim().to_lowercase());
        if let Some(entry) = self.cache.get(&key) {
            if entry.saved_at.elapsed() < CACHE_TTL {
                return Ok((entry.hits.clone(), entry.vector.clone()));
            }
        }
        let vector = self.embedder.embed_query(query)?;
        let quant = quantize(&vector).map_err(|err| NodeError::Message(err.to_string()))?;
        let hits = self.search_local(query, &vector, kind_mask)?;
        if self.cache.len() >= CACHE_MAX {
            self.cache.clear();
        }
        self.cache.insert(
            key,
            CachedQuery {
                hits: hits.clone(),
                vector: quant.clone(),
                saved_at: Instant::now(),
            },
        );
        Ok((hits, quant))
    }

    pub fn search_wire(&self, req: &SearchRequest) -> Result<Vec<Hit>, NodeError> {
        let vector = req.vector.dequantize();
        self.search_local(&req.text, &vector, 0)
    }

    fn search_local(&self, query: &str, vector: &[f32], kind_mask: u32) -> Result<Vec<Hit>, NodeError> {
        let bucket = self.lsh.bucket(vector);
        let words = self.index.search_words(query, 100, kind_mask)?;
        let meanings = self.index.search_vectors(vector, Some(bucket), 100, kind_mask)?;
        let mut ids: Vec<i64> = words
            .iter()
            .map(|hit| hit.id)
            .chain(meanings.iter().map(|hit| hit.id))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let pages = self.index.pages_by_ids(&ids)?;
        Ok(fuse(query, &words, &meanings, &pages))
    }

    fn ingest(&mut self, pages: Vec<FetchedPage>) -> Result<usize, NodeError> {
        self.ingest_inner(pages, false)
    }

    fn ingest_upsert(&mut self, pages: Vec<FetchedPage>) -> Result<usize, NodeError> {
        self.ingest_inner(pages, true)
    }

    fn ingest_inner(&mut self, pages: Vec<FetchedPage>, force_upsert: bool) -> Result<usize, NodeError> {
        let mut candidates = Vec::new();
        let mut passages = Vec::new();
        let mut rejected = Vec::new();
        for mut page in pages {
            page.text = strip_boilerplate(&page.text);
            if page_lane(&page.url, &page.title, &page.text) == Lane::Hidden
                || !page.indexable
                || is_broken_url(&page.url)
                || is_not_found(&page.title, &page.text)
            {
                eprintln!("MDS: страница убрана из индекса {}", page.url);
                rejected.push(page.url);
                continue;
            }
            if page.text.trim().is_empty() {
                rejected.push(page.url);
                continue;
            }
            if mds_types::is_webshop(&page.title, &page.text) {
                self.index.note_webshop(&page.host)?;
            }
            if !self.index.may_store(&page.host, &page.url)? {
                continue;
            }
            if !force_upsert && self.index.is_duplicate(page.simhash)? {
                eprintln!("MDS: пропуск дубля {}", page.url);
                continue;
            }
            passages.push(format!("{}\n{}", page.title, page.text));
            candidates.push(page);
        }
        self.index.delete_urls(&rejected)?;
        if candidates.is_empty() {
            return Ok(0);
        }
        let vectors = self.embedder.embed_passages(&passages)?;
        let mut stored = 0_usize;
        for (page, vector) in candidates.into_iter().zip(vectors) {
            let quant = quantize(&vector).map_err(|err| NodeError::Message(err.to_string()))?;
            let key = shard_key(&page.host, &vector, &self.lsh);
            if self.directory.locate(&key).is_empty() {
                return Err(NodeError::Message("нет ноды для шарда".into()));
            }
            self.index
                .insert(&page, &quant, &key.domain_hash, key.semantic_bucket)?;
            eprintln!(
                "MDS: в индексе {} (бакет {})",
                page.url, key.semantic_bucket
            );
            stored += 1;
        }
        Ok(stored)
    }

    pub fn remember_query(&self, query: &str, hits: &[Hit]) -> Result<(), NodeError> {
        if query_lane(query) == Lane::Hidden {
            return Ok(());
        }
        let words = hits.iter().filter(|hit| hit.via_words).count() as i64;
        let meaning = hits.iter().filter(|hit| hit.via_meaning).count() as i64;
        let adult = hits.iter().filter(|hit| hit.adult).count() as i64;
        let open = hits.len() as i64 - adult;
        Ok(self.index.remember_query(query, open, words, meaning, adult)?)
    }

    fn remember_priorities(&self, pages: &[FetchedPage]) -> Result<(), NodeError> {
        for page in pages {
            if page.priority.is_empty() {
                continue;
            }
            self.index.set_priority(&page.host, &page.priority)?;
        }
        Ok(())
    }

    pub fn remember_volunteer(&self, host: &str) -> Result<(), NodeError> {
        Ok(self.index.remember_volunteer(host)?)
    }

    pub fn volunteers(&self) -> Result<Vec<String>, NodeError> {
        Ok(self.index.volunteers()?)
    }

    pub fn forget_volunteer(&self, host: &str) -> Result<(), NodeError> {
        Ok(self.index.forget_volunteer(host)?)
    }

    pub fn refuse_host(&self, host: &str) -> Result<(), NodeError> {
        Ok(self.index.refuse_host(host)?)
    }

    pub fn restore_host(&self, host: &str) -> Result<(), NodeError> {
        Ok(self.index.restore_host(host)?)
    }

    pub fn refused_hosts(&self) -> Result<Vec<String>, NodeError> {
        Ok(self.index.refused_hosts()?)
    }

    pub fn recent_queries(&self, limit: usize) -> Result<Vec<QueryStat>, NodeError> {
        Ok(self.index.recent_queries(limit)?)
    }

    pub fn note_search(&mut self, elapsed_ms: u128) {
        self.load.note(elapsed_ms);
    }

    pub fn load_snapshot(&mut self) -> (u64, u64) {
        self.load.snapshot()
    }

    pub fn queue_len(&self) -> Result<usize, NodeError> {
        Ok(self.index.queue_len()?)
    }

    pub fn bucket_report(&self) -> Result<(Vec<u16>, bool), NodeError> {
        Ok(self.index.bucket_report()?)
    }

    pub fn db_bytes(&self) -> u64 {
        std::fs::metadata(&self.db).map(|meta| meta.len()).unwrap_or(0)
    }

    pub fn semantic_bucket(&self, vector: &QuantVector) -> u16 {
        self.lsh.bucket(&vector.dequantize())
    }

    pub fn pages_for_hive(&self, limit: usize) -> Result<Vec<(String, String, String, String)>, NodeError> {
        Ok(self.index.recent_pages(limit)?)
    }

    pub fn accept_remote_pages(
        &mut self,
        pages: Vec<(String, String, String, String)>,
    ) -> Result<usize, NodeError> {
        let fetched: Vec<FetchedPage> = pages
            .into_iter()
            .take(8)
            .filter_map(|(url, title, host, text)| {
                let text: String = text.chars().take(80_000).collect();
                if url.trim().is_empty() || text.trim().is_empty() {
                    return None;
                }
                let simhash = mds_crawl::simhash(&text);
                let kind = mds_types::kind_from_stored(&url, &title, &text);
                Some(FetchedPage {
                    url,
                    host,
                    title,
                    text,
                    simhash,
                    links: Vec::new(),
                    indexable: true,
                    kind,
                    media_url: None,
                    feed_url: None,
                    price: None,
                    docs: Vec::new(),
                    priority: Vec::new(),
                })
            })
            .collect();
        let urls: Vec<String> = fetched.iter().map(|page| page.url.clone()).collect();
        let stored = self.ingest_upsert(fetched)?;
        self.index.mark_visited(&urls)?;
        Ok(stored)
    }
}
