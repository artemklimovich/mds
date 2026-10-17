use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mds_crawl::{is_broken_url, is_not_found, strip_boilerplate, tidy_url, CrawlPolicy, Crawler, FetchedPage};
use mds_dht::{shard_key, LocalDirectory, Lsh};
use mds_embed::Embedder;
use mds_index::{GeoPlaceRow, Index, IndexLine, ParkedPage, QueryStat, RankPlace, StoredLeaf, TextSpool, VisitDay, VisitDelta, VisitReport};
use mds_rank::{drop_near_copies, fuse, page_lane, query_lane, Lane};
use mds_types::{bare_host, host_of_url, quantize, Hit, QuantVector};
use mds_wire::SearchRequest;

use crate::NodeError;

const CACHE_TTL: Duration = Duration::from_secs(300);
const CACHE_MAX: usize = 64;

struct CachedQuery {
    hits: Vec<Hit>,
    vector: QuantVector,
    saved_at: Instant,
}

/// Пишет `warm` и ждёт появления `go`. База в это время не открывается.
pub fn wait_for_go(go: Option<&Path>, warm: Option<&Path>) -> Result<(), NodeError> {
    if let Some(warm) = warm {
        if let Some(parent) = warm.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(warm, b"1\n")?;
    }
    let Some(go) = go else {
        return Ok(());
    };
    eprintln!("MDS: модель в памяти, база откроется когда появится {}", go.display());
    while !go.exists() {
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = std::fs::remove_file(go);
    Ok(())
}

pub struct Engine {
    index: Index,
    spool: TextSpool,
    embedder: Option<Embedder>,
    /// Свежий запрос просит модель раньше следующей пачки склада.
    search_wants_model: bool,
    /// Адреса, которые сейчас считаются вне замка.
    embedding: Vec<String>,
    lsh: Lsh,
    directory: LocalDirectory,
    cache: HashMap<String, CachedQuery>,
    db: PathBuf,
    load: SearchLoad,
    self_name: String,
    shard_peers: Vec<String>,
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

/// Пачка склада, которую модель считает без замка поиска.
pub struct PassageBatch {
    embedder: Embedder,
    pages: Vec<FetchedPage>,
    passages: Vec<String>,
}

impl PassageBatch {
    pub fn embed(&mut self) -> Result<Vec<Vec<f32>>, NodeError> {
        self.embedder
            .embed_passages(&self.passages)
            .map_err(|err| NodeError::Message(err.to_string()))
    }
}

pub enum PassageClaim {
    /// На складе пусто.
    Idle,
    /// Модель нужна свежему запросу.
    Yield,
    /// Со склада убраны страницы без вектора, можно брать дальше.
    Again,
    Ready(PassageBatch),
}

impl Engine {
    pub fn open(db: &Path, models: &Path) -> Result<Self, NodeError> {
        Self::open_when(db, models, None, None)
    }

    /// Сначала модель, потом база. Если задан `go`, процесс ждёт этот файл и не открывает базу,
    /// пока старый улей её не отпустил.
    pub fn open_when(
        db: &Path,
        models: &Path,
        go: Option<&Path>,
        warm: Option<&Path>,
    ) -> Result<Self, NodeError> {
        let embedder = Self::load_model(models)?;
        if go.is_some() {
            wait_for_go(go, warm)?;
        }
        Self::open_ready(db, embedder)
    }

    pub fn load_model(models: &Path) -> Result<Embedder, NodeError> {
        eprintln!("MDS: модель эмбеддингов, каталог {}", models.display());
        Ok(Embedder::open(models)?)
    }

    fn open_ready(db: &Path, embedder: Embedder) -> Result<Self, NodeError> {
        let index = Index::open(db)?;
        let spool = TextSpool::open(&mds_index::spool_path(db))?;
        let removed = index.scrub_stored_pages()?;
        if removed > 0 {
            eprintln!("MDS: из индекса убрано {removed} страниц без адреса, с ответом «не найдено» или со склеенной ссылкой");
        }
        Ok(Self {
            index,
            spool,
            embedder: Some(embedder),
            search_wants_model: false,
            embedding: Vec::new(),
            lsh: Lsh::new(),
            directory: LocalDirectory::single(),
            cache: HashMap::new(),
            db: db.to_path_buf(),
            load: SearchLoad {
                marks: VecDeque::new(),
            },
            self_name: String::new(),
            shard_peers: Vec::new(),
        })
    }

    /// Второй улей включается своим публичным адресом и списком соседей.
    /// Без своего адреса этот улей по-прежнему держит все домены.
    pub fn set_shard_circle(&mut self, self_base: &str, peers: Vec<String>) {
        let self_name = self_base.trim().trim_end_matches('/').to_string();
        if self_name.is_empty() {
            self.self_name.clear();
            self.shard_peers.clear();
            let _ = self.index.release_deferred();
            return;
        }
        self.self_name = self_name;
        self.shard_peers = peers
            .into_iter()
            .map(|peer| peer.trim().trim_end_matches('/').to_string())
            .filter(|peer| !peer.is_empty() && peer != &self.self_name)
            .collect();
    }

    pub fn shares_shards(&self) -> bool {
        !self.self_name.is_empty() && !self.shard_peers.is_empty()
    }

    fn keeps(&self, host: &str) -> bool {
        mds_dht::keeps_domain(&self.self_name, &self.shard_peers, host)
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

    pub fn claim_urls(&self, limit: usize, hold_secs: i64) -> Result<Vec<String>, NodeError> {
        let urls = self.index.claim_urls(limit, hold_secs)?;
        if !self.shares_shards() {
            return Ok(urls);
        }
        let mut mine = Vec::new();
        let mut rest = Vec::new();
        for url in urls {
            let host = host_of_url(&url).unwrap_or_default();
            if self.keeps(&host) {
                mine.push(url);
            } else {
                rest.push(url);
            }
        }
        if !rest.is_empty() {
            self.index.defer_urls(&rest)?;
        }
        Ok(mine)
    }

    pub fn release_lease(&self, urls: &[String]) -> Result<(), NodeError> {
        Ok(self.index.release_lease(urls)?)
    }

    pub fn leased_count(&self) -> Result<i64, NodeError> {
        Ok(self.index.leased_count()?)
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
        self.note_page_anchors(&pages)?;
        self.index.enqueue_urls(discovered)?;
        self.index.delete_urls(gone)?;
        self.cache.clear();
        self.park_pages(&pages, false)?;
        self.spool_waiting()
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
        self.note_page_anchors(&pages)?;
        self.index.enqueue_urls(&discovered)?;
        self.index.delete_urls(&gone)?;
        self.cache.clear();
        self.park_pages(&pages, false)?;
        self.drain_until_quiet()
    }

    pub fn search(&mut self, query: &str) -> Result<Vec<Hit>, NodeError> {
        let (hits, _) = self.search_with_vector(query, 0)?;
        Ok(hits)
    }

    pub fn set_search_first(&mut self, first: bool) {
        self.search_wants_model = first;
    }

    pub fn take_model(&mut self) -> Option<Embedder> {
        self.embedder.take()
    }

    pub fn give_model(&mut self, embedder: Embedder) {
        self.embedder = Some(embedder);
    }

    /// Повтор за последние пять минут. Модель не нужна.
    pub fn cached_search(&mut self, query: &str, kind_mask: u32) -> Option<(Vec<Hit>, QuantVector)> {
        let key = format!("{kind_mask}:{}", query.trim().to_lowercase());
        let entry = self.cache.get(&key)?;
        if entry.saved_at.elapsed() >= CACHE_TTL {
            return None;
        }
        Some((entry.hits.clone(), entry.vector.clone()))
    }

    pub fn finish_search(
        &mut self,
        query: &str,
        kind_mask: u32,
        vector: Vec<f32>,
    ) -> Result<(Vec<Hit>, QuantVector), NodeError> {
        let quant = quantize(&vector).map_err(|err| NodeError::Message(err.to_string()))?;
        let hits = self.search_local(query, &vector, kind_mask)?;
        self.remember_cached(query, kind_mask, &hits, quant.clone());
        Ok((hits, quant))
    }

    pub fn search_with_vector(&mut self, query: &str, kind_mask: u32) -> Result<(Vec<Hit>, QuantVector), NodeError> {
        if let Some(found) = self.cached_search(query, kind_mask) {
            return Ok(found);
        }
        let embedder = self
            .embedder
            .as_mut()
            .ok_or_else(|| NodeError::Message("модель занята складом".into()))?;
        let vector = embedder.embed_query(query)?;
        self.finish_search(query, kind_mask, vector)
    }

    fn remember_cached(&mut self, query: &str, kind_mask: u32, hits: &[Hit], vector: QuantVector) {
        let key = format!("{kind_mask}:{}", query.trim().to_lowercase());
        if self.cache.len() >= CACHE_MAX {
            self.cache.clear();
        }
        self.cache.insert(
            key,
            CachedQuery {
                hits: hits.to_vec(),
                vector,
                saved_at: Instant::now(),
            },
        );
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
        let muted = self.index.muted_hosts().unwrap_or_default();
        let mut hits = fuse(query, &words, &meanings, &pages);
        let bees = self.index.volunteers().unwrap_or_default();
        hits = drop_near_copies(hits, &pages, &bees);
        if !muted.is_empty() {
            hits.retain(|hit| {
                host_of_url(&hit.url)
                    .map(|host| !muted.iter().any(|item| item == &bare_host(&host)))
                    .unwrap_or(true)
            });
        }
        Ok(hits)
    }

    fn ingest(&mut self, pages: Vec<FetchedPage>) -> Result<usize, NodeError> {
        self.ingest_inner(pages, false)
    }

    fn ingest_upsert(&mut self, pages: Vec<FetchedPage>) -> Result<usize, NodeError> {
        self.ingest_inner(pages, true)
    }

    fn collect_candidates(
        &mut self,
        pages: Vec<FetchedPage>,
        force_upsert: bool,
    ) -> Result<(Vec<FetchedPage>, Vec<String>), NodeError> {
        let mut candidates = Vec::new();
        let mut passages = Vec::new();
        let mut rejected = Vec::new();
        let mut seen_hosts = std::collections::HashSet::new();
        let mut fresh = 0_i64;
        let mut changed = 0_i64;
        let mut same = 0_i64;
        let mut new_hosts = 0_i64;
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
            if !self.keeps(&page.host) {
                continue;
            }
            let _ = self.index.clear_host_quiet(&page.host);
            let stored = self.index.stored_simhash(&page.url)?;
            let (add_fresh, add_changed, add_same) = classify_store(stored, page.simhash);
            if add_same == 1 {
                same += 1;
                continue;
            }
            if !force_upsert && self.index.is_duplicate(page.simhash)? {
                eprintln!("MDS: пропуск дубля {}", page.url);
                continue;
            }
            fresh += add_fresh;
            changed += add_changed;
            if add_fresh == 1 && !seen_hosts.contains(&page.host) && self.index.host_pages(&page.host)? == 0 {
                new_hosts += 1;
                seen_hosts.insert(page.host.clone());
            }
            passages.push(format!("{}\n{}", page.title, page.text));
            candidates.push(page);
        }
        self.index.delete_urls(&rejected)?;
        let _ = self.index.note_intake(fresh, changed, same, new_hosts);
        Ok((candidates, passages))
    }

    fn write_vectors(&mut self, candidates: Vec<FetchedPage>, vectors: Vec<Vec<f32>>) -> Result<usize, NodeError> {
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

    fn ingest_inner(&mut self, pages: Vec<FetchedPage>, force_upsert: bool) -> Result<usize, NodeError> {
        let (candidates, passages) = self.collect_candidates(pages, force_upsert)?;
        if candidates.is_empty() {
            return Ok(0);
        }
        let embedder = self
            .embedder
            .as_mut()
            .ok_or_else(|| NodeError::Message("модель занята поиском".into()))?;
        let vectors = embedder.embed_passages(&passages)?;
        self.write_vectors(candidates, vectors)
    }

    pub fn remember_query(&self, query: &str, hits: &[Hit]) -> Result<(), NodeError> {
        if query_lane(query) == Lane::Hidden {
            return Ok(());
        }
        let words = hits.iter().filter(|hit| hit.via_words).count() as i64;
        let meaning = hits.iter().filter(|hit| hit.via_meaning).count() as i64;
        let adult = hits.iter().filter(|hit| hit.adult).count() as i64;
        let open = hits.len() as i64 - adult;
        let places: Vec<RankPlace> = hits
            .iter()
            .take(10)
            .enumerate()
            .filter_map(|(place, hit)| {
                let host = mds_dht::host_of(&hit.url)?;
                Some(RankPlace {
                    place: place as i64 + 1,
                    url: hit.url.clone(),
                    host,
                })
            })
            .collect();
        Ok(self
            .index
            .remember_ranked(query, open, words, meaning, adult, &places)?)
    }

    pub fn key_queries(&self) -> Result<Vec<(String, i64)>, NodeError> {
        Ok(self.index.key_queries(20)?)
    }

    pub fn query_counts(&self, span: &str) -> Result<Vec<(String, i64)>, NodeError> {
        Ok(self.index.query_counts(span, 20)?)
    }

    pub fn add_visit(&self, delta: &VisitDelta) -> Result<(), NodeError> {
        Ok(self.index.add_visit(delta)?)
    }

    pub fn visit_span(&self, span: &str) -> Result<VisitReport, NodeError> {
        Ok(self.index.visit_span(span)?)
    }

    pub fn visit_days(&self, limit: usize) -> Result<Vec<VisitDay>, NodeError> {
        Ok(self.index.visit_days(limit)?)
    }

    pub fn has_news_feed(&self, url: &str) -> Result<bool, NodeError> {
        Ok(self.index.has_news_feed(url)?)
    }

    pub fn has_shop_feed(&self, url: &str) -> Result<bool, NodeError> {
        Ok(self.index.has_shop_feed(url)?)
    }

    pub fn pages_of_site(&self, host: &str, words: &str, kind_mask: u32) -> Result<Vec<Hit>, NodeError> {
        let pages = self.index.pages_of_host(host, kind_mask, 100)?;
        let needles: Vec<String> = words
            .split_whitespace()
            .map(|word| word.to_lowercase())
            .filter(|word| !word.is_empty())
            .collect();
        let mut hits = Vec::new();
        for page in pages {
            let lane = page_lane(&page.url, &page.title, &page.text);
            if lane == Lane::Hidden {
                continue;
            }
            let hay = format!("{}\n{}", page.title, page.text).to_lowercase();
            if !needles.iter().all(|word| hay.contains(word)) {
                continue;
            }
            hits.push(Hit {
                url: page.url,
                title: page.title.clone(),
                snippet: mds_types::snippet(&page.text, if words.trim().is_empty() { page.title.as_str() } else { words }),
                score: 1.0,
                via_words: !needles.is_empty(),
                via_meaning: false,
                adult: lane == Lane::Adult,
                volunteer: false,
                region: page.region,
                place: page.place,
                distance_km: None,
                lat: page.lat,
                lon: page.lon,
                kind: page.kind,
                media_url: page.media_url,
                feed_url: page.feed_url,
                news_feed: page.news_feed,
                price: page.price,
            });
        }
        Ok(hits)
    }

    pub fn searches_last_minute(&self) -> Result<u64, NodeError> {
        Ok(self.index.queries_last_minute()? as u64)
    }

    pub fn site_places(&self, host: &str) -> Result<Vec<(String, i64, String)>, NodeError> {
        Ok(self.index.site_places(host, 20)?)
    }

    pub fn list_index(
        &self,
        host: Option<&str>,
        kind: Option<u32>,
        after: i64,
    ) -> Result<Vec<IndexLine>, NodeError> {
        Ok(self.index.list_index(host, kind, after, 20)?)
    }

    pub fn kind_counts(&self) -> Result<Vec<(String, String, String, i64)>, NodeError> {
        Ok(self.index.kind_counts()?)
    }

    pub fn contacts_for_url(&self, url: &str) -> mds_types::Contacts {
        self.index.contacts_for_url(url)
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
    ) -> Result<(usize, usize, Vec<GeoPlaceRow>), NodeError> {
        Ok(self
            .index
            .geo_places(q, kind_mask, region, place, around, bbox, limit)?)
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

    pub fn note_shell(&self, id: &str) -> Result<(), NodeError> {
        Ok(self.index.note_shell(id)?)
    }

    pub fn note_shell_pages(&self, pages: i64) -> Result<(), NodeError> {
        Ok(self.index.note_shell_pages(pages)?)
    }

    pub fn shell_span(&self, span: &str) -> Result<(i64, i64), NodeError> {
        Ok(self.index.shell_span(span)?)
    }

    pub fn shell_pace(&self) -> Result<mds_index::ShellPace, NodeError> {
        Ok(self.index.shell_pace()?)
    }

    pub fn set_shell_pace(&self, batch: i64, gap_min: i64, window_max: i64) -> Result<(), NodeError> {
        Ok(self.index.set_shell_pace(batch, gap_min, window_max)?)
    }

    pub fn shell_ceiling(&self) -> Result<mds_index::ShellCeiling, NodeError> {
        Ok(self.index.shell_ceiling()?)
    }

    pub fn set_shell_ceiling(&self, max_batch: i64, min_gap: i64) -> Result<(), NodeError> {
        Ok(self.index.set_shell_ceiling(max_batch, min_gap)?)
    }

    pub fn shell_grants(&self) -> Result<i64, NodeError> {
        Ok(self.index.shell_grants()?)
    }

    pub fn set_shell_grants(&self, grants: i64) -> Result<(), NodeError> {
        Ok(self.index.set_shell_grants(grants)?)
    }

    pub fn refresh_pace(&self) -> Result<mds_index::RefreshPace, NodeError> {
        Ok(self.index.refresh_pace()?)
    }

    pub fn set_refresh_pace(&self, batch: i64, gap_secs: i64) -> Result<(), NodeError> {
        Ok(self.index.set_refresh_pace(batch, gap_secs)?)
    }

    pub fn refresh_due(&self) -> Result<i64, NodeError> {
        Ok(self.index.refresh_due()?)
    }

    pub fn claim_refresh(&self, limit: usize) -> Result<Vec<mds_index::RefreshJob>, NodeError> {
        Ok(self.index.claim_refresh(limit)?)
    }

    pub fn refresh_quiet(&self, url: &str, etag: Option<&str>, modified: Option<&str>) -> Result<(), NodeError> {
        Ok(self.index.refresh_quiet(url, etag, modified)?)
    }

    pub fn refresh_retry(&self, url: &str) -> Result<(), NodeError> {
        Ok(self.index.refresh_retry(url)?)
    }

    pub fn refresh_after(&self, url: &str, secs: i64) -> Result<(), NodeError> {
        Ok(self.index.refresh_after(url, secs)?)
    }

    pub fn drop_urls(&self, urls: &[String]) -> Result<(), NodeError> {
        self.index.delete_urls(urls)?;
        Ok(())
    }

    pub fn store_refresh(
        &mut self,
        page: FetchedPage,
        etag: Option<String>,
        modified: Option<String>,
    ) -> Result<(), NodeError> {
        let mut page = page;
        page.url = tidy_url(&page.url);
        let links = page.links.clone();
        let url = page.url.clone();
        let hash = page.simhash;
        self.index.enqueue_urls(&links)?;
        if self.index.stored_simhash(&url)? == Some(hash) {
            self.index.note_intake(0, 0, 1, 0)?;
            self.index.refresh_quiet(&url, etag.as_deref(), modified.as_deref())?;
            return Ok(());
        }
        self.park_pages(&[page], true)?;
        self.index.refresh_changed(&url, etag.as_deref(), modified.as_deref())?;
        Ok(())
    }

    pub fn crawl_pace(&self) -> Result<mds_index::CrawlPace, NodeError> {
        Ok(self.index.crawl_pace()?)
    }

    pub fn set_crawl_pace(&self, batch: i64, gap_secs: i64) -> Result<(), NodeError> {
        Ok(self.index.set_crawl_pace(batch, gap_secs)?)
    }

    pub fn bee_pace(&self) -> Result<mds_index::BeePace, NodeError> {
        Ok(self.index.bee_pace()?)
    }

    pub fn set_bee_pace(&self, batch: i64, gap_secs: i64) -> Result<(), NodeError> {
        Ok(self.index.set_bee_pace(batch, gap_secs)?)
    }

    pub fn intake_sum(&self, column: &str, span: &str) -> Result<i64, NodeError> {
        Ok(self.index.intake_sum(column, span)?)
    }

    pub fn intake_days(&self, limit: usize) -> Result<Vec<mds_index::IntakeDay>, NodeError> {
        Ok(self.index.intake_days(limit)?)
    }

    pub fn mark_seen(&self, urls: &[String]) -> Result<(), NodeError> {
        Ok(self.index.mark_visited(urls)?)
    }

    pub fn note_load(&self, cpu_pct: i64, ram_pct: i64) -> Result<(), NodeError> {
        Ok(self.index.note_load(cpu_pct, ram_pct)?)
    }

    pub fn load_hours(&self, limit: usize) -> Result<Vec<mds_index::LoadHour>, NodeError> {
        Ok(self.index.load_hours(limit)?)
    }

    pub fn fresh_urls(&self, pages: &[(String, u64)]) -> Result<Vec<String>, NodeError> {
        let mut want = Vec::new();
        for (url, hash) in pages {
            if self.index.stored_simhash(url)? != Some(*hash) {
                want.push(url.clone());
            }
        }
        Ok(want)
    }

    pub fn note_bee_pages(&self, host: &str, pages: i64) -> Result<(), NodeError> {
        Ok(self.index.note_bee_pages(host, pages)?)
    }

    pub fn bee_totals(&self, host: &str) -> Result<mds_index::BeeTotals, NodeError> {
        Ok(self.index.bee_totals(host)?)
    }

    pub fn remember_node(&self, base: &str) -> Result<(), NodeError> {
        Ok(self.index.remember_node(base)?)
    }

    pub fn forget_node(&self, base: &str) -> Result<(), NodeError> {
        Ok(self.index.forget_node(base)?)
    }

    pub fn known_nodes(&self) -> Result<Vec<mds_index::KnownNode>, NodeError> {
        Ok(self.index.known_nodes()?)
    }

    pub fn touch_node(&self, base: &str, alive: bool, latency_ms: u64) -> Result<(), NodeError> {
        Ok(self.index.touch_node(base, alive, latency_ms)?)
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

    pub fn mute_host(&mut self, host: &str) -> Result<(), NodeError> {
        self.index.mute_host(host)?;
        self.cache.clear();
        Ok(())
    }

    pub fn unmute_host(&mut self, host: &str) -> Result<(), NodeError> {
        self.index.unmute_host(host)?;
        self.cache.clear();
        Ok(())
    }

    pub fn muted_hosts(&self) -> Result<Vec<String>, NodeError> {
        Ok(self.index.muted_hosts()?)
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

    pub fn accept_remote_pages(&mut self, pages: Vec<mds_wire::PushedPage>) -> Result<usize, NodeError> {
        let mut discovered = Vec::new();
        let mut fetched = Vec::new();
        for page in pages.into_iter().take(40) {
            let text: String = page.text.chars().take(80_000).collect();
            if page.url.trim().is_empty() || text.trim().is_empty() {
                continue;
            }
            discovered.extend(owned_links(&page.host, &page.links));
            for anchor in &page.anchors {
                let _ = self.index.remember_anchor(&anchor.url, &anchor.text);
            }
            let simhash = if page.simhash == 0 {
                mds_crawl::simhash(&text)
            } else {
                page.simhash
            };
            let kind = mds_types::kind_from_stored(&page.url, &page.title, &text);
            fetched.push(FetchedPage {
                url: page.url,
                host: page.host,
                title: page.title,
                text,
                simhash,
                links: Vec::new(),
                anchors: Vec::new(),
                indexable: true,
                kind,
                media_url: None,
                feed_url: None,
                news_feed: None,
                price: None,
                docs: Vec::new(),
                priority: Vec::new(),
            });
        }
        let urls: Vec<String> = fetched.iter().map(|page| page.url.clone()).collect();
        let parked = self.park_pages(&fetched, true)?;
        self.index.enqueue_urls(&discovered)?;
        self.index.mark_visited(&urls)?;
        Ok(parked)
    }

    /// Забирает пачку из склада, считает векторы разом и один раз пишет индекс.
    fn drain_until_quiet(&mut self) -> Result<usize, NodeError> {
        let mut stored = 0;
        for _ in 0..4 {
            if self.spool_waiting()? == 0 {
                break;
            }
            stored += self.drain_parked(8)?;
        }
        Ok(stored)
    }

    pub fn drain_parked(&mut self, limit: usize) -> Result<usize, NodeError> {
        let parked = self.spool.take(limit.max(1).min(8))?;
        if parked.is_empty() {
            return Ok(0);
        }
        let urls: Vec<String> = parked.iter().map(|page| page.url.clone()).collect();
        let mut plain = Vec::new();
        let mut forced = Vec::new();
        for page in parked {
            let rewrite = page.force;
            let fetched = parked_as_page(page);
            if rewrite {
                forced.push(fetched);
            } else {
                plain.push(fetched);
            }
        }
        let mut stored = 0;
        if !plain.is_empty() {
            stored += self.ingest(plain)?;
        }
        if !forced.is_empty() {
            stored += self.ingest_upsert(forced)?;
        }
        self.spool.ack(&urls)?;
        self.cache.clear();
        Ok(stored)
    }

    pub fn spool_waiting(&self) -> Result<usize, NodeError> {
        Ok(self.spool.len()?)
    }

    /// Забирает до восьми страниц и модель. Векторы считаются снаружи, запись — в `finish_passage_batch`.
    pub fn claim_passage_batch(&mut self) -> Result<PassageClaim, NodeError> {
        if self.search_wants_model {
            return Ok(PassageClaim::Yield);
        }
        if self.spool_waiting()? == 0 {
            return Ok(PassageClaim::Idle);
        }
        let scanned = self.spool.take(64)?;
        let mut picked = Vec::new();
        for page in scanned {
            if self.embedding.iter().any(|url| url == &page.url) {
                continue;
            }
            picked.push(page);
            if picked.len() == 8 {
                break;
            }
        }
        if picked.is_empty() {
            return Ok(PassageClaim::Idle);
        }
        let urls: Vec<String> = picked.iter().map(|page| page.url.clone()).collect();
        self.embedding.extend(urls.iter().cloned());
        let Some(embedder) = self.embedder.take() else {
            self.embedding.retain(|url| !urls.iter().any(|item| item == url));
            return Ok(PassageClaim::Yield);
        };
        let mut plain = Vec::new();
        let mut forced = Vec::new();
        for page in picked {
            let rewrite = page.force;
            let fetched = parked_as_page(page);
            if rewrite {
                forced.push(fetched);
            } else {
                plain.push(fetched);
            }
        }
        let (mut candidates, mut passages) = self.collect_candidates(plain, false)?;
        let (more, more_passages) = self.collect_candidates(forced, true)?;
        candidates.extend(more);
        passages.extend(more_passages);
        let keep: HashSet<String> = candidates.iter().map(|page| page.url.clone()).collect();
        let skip: Vec<String> = urls.into_iter().filter(|url| !keep.contains(url)).collect();
        self.spool.ack(&skip)?;
        self.embedding.retain(|url| keep.contains(url));
        if candidates.is_empty() {
            self.embedder = Some(embedder);
            return Ok(PassageClaim::Again);
        }
        Ok(PassageClaim::Ready(PassageBatch {
            embedder,
            pages: candidates,
            passages,
        }))
    }

    pub fn finish_passage_batch(&mut self, batch: PassageBatch, vectors: Vec<Vec<f32>>) -> Result<usize, NodeError> {
        self.embedder = Some(batch.embedder);
        let urls: Vec<String> = batch.pages.iter().map(|page| page.url.clone()).collect();
        let stored = self.write_vectors(batch.pages, vectors)?;
        self.spool.ack(&urls)?;
        self.embedding.retain(|url| !urls.iter().any(|done| done == url));
        self.cache.clear();
        Ok(stored)
    }

    pub fn cancel_passage_batch(&mut self, batch: PassageBatch) {
        self.embedder = Some(batch.embedder);
        let urls: Vec<String> = batch.pages.iter().map(|page| page.url.clone()).collect();
        self.embedding.retain(|url| !urls.iter().any(|done| done == url));
    }

    fn park_pages(&self, pages: &[FetchedPage], force: bool) -> Result<usize, NodeError> {
        let parked: Vec<ParkedPage> = pages
            .iter()
            .filter(|page| page.indexable && !page.text.trim().is_empty())
            .map(|page| ParkedPage {
                url: tidy_url(&page.url),
                host: page.host.clone(),
                title: page.title.clone(),
                text: page.text.clone(),
                simhash: page.simhash,
                kind: page.kind,
                media_url: page.media_url.clone(),
                feed_url: page.feed_url.clone(),
                news_feed: page.news_feed.clone(),
                price: page.price.clone(),
                force,
            })
            .collect();
        Ok(self.spool.park(&parked)?)
    }

    fn note_page_anchors(&self, pages: &[FetchedPage]) -> Result<(), NodeError> {
        for page in pages {
            for anchor in &page.anchors {
                self.index.remember_anchor(&tidy_url(&anchor.url), &anchor.text)?;
            }
        }
        Ok(())
    }

    pub fn export_shard(&self, from: u64, limit: usize) -> Result<mds_wire::ShardBlockV1, NodeError> {
        if !self.shares_shards() {
            return Ok(empty_block(from));
        }
        let leaves = self.index.pages_after(from, limit.min(32))?;
        Ok(block_from(from, leaves))
    }

    pub fn accept_shard_block(&mut self, block: &mds_wire::ShardBlockV1) -> Result<usize, NodeError> {
        if shard_hash(&block.pages) != block.block_sha256 {
            return Ok(0);
        }
        let mut pages = Vec::new();
        for leaf in &block.pages {
            if self.shares_shards() && !self.keeps(&leaf.host) {
                continue;
            }
            pages.push(FetchedPage {
                url: leaf.url.clone(),
                host: leaf.host.clone(),
                title: leaf.title.clone(),
                text: leaf.text.clone(),
                simhash: leaf.simhash,
                links: Vec::new(),
                anchors: Vec::new(),
                indexable: true,
                kind: mds_types::kind_from_stored(&leaf.url, &leaf.title, &leaf.text),
                media_url: None,
                feed_url: None,
                news_feed: None,
                price: None,
                docs: Vec::new(),
                priority: Vec::new(),
            });
        }
        self.park_pages(&pages, true)
    }

    pub fn note_host_quiet(&self, host: &str) -> Result<(), NodeError> {
        Ok(self.index.note_host_quiet(host)?)
    }

    pub fn shard_manifest(&self) -> mds_wire::ShardManifestV1 {
        let revision = self.index.index_revision().unwrap_or(0);
        let pages = self.index.len().unwrap_or(0) as u64;
        mds_wire::ShardManifestV1 {
            shard: self.self_name.clone(),
            generation: 1,
            revision,
            pages,
            root_sha256: mds_update::sha256_hex(format!("{revision}:{pages}").as_bytes()),
        }
    }

    pub fn peer_allowed(&self, base: &str) -> bool {
        let base = base.trim().trim_end_matches('/');
        self.shard_peers.iter().any(|peer| peer == base)
    }
}

fn parked_as_page(page: ParkedPage) -> FetchedPage {
    FetchedPage {
        url: page.url,
        host: page.host,
        title: page.title,
        text: page.text,
        simhash: page.simhash,
        links: Vec::new(),
        anchors: Vec::new(),
        indexable: true,
        kind: page.kind,
        media_url: page.media_url,
        feed_url: page.feed_url,
        news_feed: page.news_feed,
        price: page.price,
        docs: Vec::new(),
        priority: Vec::new(),
    }
}

fn empty_block(from: u64) -> mds_wire::ShardBlockV1 {
    mds_wire::ShardBlockV1 {
        from_revision: from,
        to_revision: from,
        pages: Vec::new(),
        block_sha256: shard_hash(&[]),
    }
}

fn block_from(from: u64, leaves: Vec<StoredLeaf>) -> mds_wire::ShardBlockV1 {
    let pages: Vec<mds_wire::ShardLeaf> = leaves
        .into_iter()
        .map(|leaf| mds_wire::ShardLeaf {
            url: leaf.url,
            host: leaf.host,
            title: leaf.title,
            text: leaf.text,
            simhash: leaf.simhash,
            revision: leaf.revision,
        })
        .collect();
    let to_revision = pages.iter().map(|page| page.revision).max().unwrap_or(from);
    let block_sha256 = shard_hash(&pages);
    mds_wire::ShardBlockV1 {
        from_revision: from,
        to_revision,
        pages,
        block_sha256,
    }
}

pub fn shard_hash(pages: &[mds_wire::ShardLeaf]) -> String {
    mds_update::sha256_hex(&shard_bytes(pages))
}

fn shard_bytes(pages: &[mds_wire::ShardLeaf]) -> Vec<u8> {
    let mut body = Vec::new();
    for page in pages {
        body.extend(page.url.as_bytes());
        body.push(b'\n');
        body.extend(page.simhash.to_string().as_bytes());
        body.push(b'\n');
        body.extend(page.host.as_bytes());
        body.push(b'\n');
        body.extend(page.title.as_bytes());
        body.push(b'\n');
        body.extend(page.text.as_bytes());
        body.push(b'\n');
    }
    body
}

fn owned_links(host: &str, links: &[String]) -> Vec<String> {
    let bare = bare_host(host);
    if bare.is_empty() {
        return Vec::new();
    }
    links
        .iter()
        .filter_map(|url| {
            let url = url.trim();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return None;
            }
            let found = host_of_url(url)?;
            if bare_host(&found) == bare {
                Some(url.to_string())
            } else {
                None
            }
        })
        .take(20)
        .collect()
}

/// `(новые, изменённые, без изменений)`. Одинаковый отпечаток в изменённые не входит.
fn classify_store(stored: Option<u64>, hash: u64) -> (i64, i64, i64) {
    if stored == Some(hash) {
        (0, 0, 1)
    } else if stored.is_none() {
        (1, 0, 0)
    } else {
        (0, 1, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_simhash_is_not_a_changed_page() {
        assert_eq!(classify_store(Some(7), 7), (0, 0, 1));
        assert_eq!(classify_store(Some(7), 8), (0, 1, 0));
        assert_eq!(classify_store(None, 8), (1, 0, 0));
    }

    #[test]
    fn shard_hash_changes_when_the_text_changes() {
        let page = mds_wire::ShardLeaf {
            url: "https://a.example/".into(),
            host: "a.example".into(),
            title: "Банка".into(),
            text: "мёд".into(),
            simhash: 1,
            revision: 2,
        };
        let same = shard_hash(std::slice::from_ref(&page));
        let mut other = page.clone();
        other.text = "мед".into();
        assert_ne!(same, shard_hash(&[other]));
        assert_eq!(same, shard_hash(&[page]));
        let mut broken = block_from(0, Vec::new());
        broken.block_sha256 = "nope".into();
        assert_ne!(broken.block_sha256, shard_hash(&broken.pages));
    }

    #[test]
    fn warm_signal_does_not_need_the_database() {
        let dir = std::env::temp_dir().join(format!("mds-warm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let go = dir.join("go");
        let warm = dir.join("warm");
        let _ = std::fs::remove_file(&go);
        let go_for_wait = go.clone();
        let warm_for_wait = warm.clone();
        let waiter = std::thread::spawn(move || wait_for_go(Some(&go_for_wait), Some(&warm_for_wait)));
        for _ in 0..50 {
            if warm.is_file() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(warm.is_file());
        std::fs::write(&go, b"go\n").unwrap();
        waiter.join().unwrap().unwrap();
        assert!(!go.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
