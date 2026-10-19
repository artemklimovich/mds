use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{ConnectInfo, Form, OriginalUri, Path, Query, Request, State};
use axum::middleware::Next;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use mds_dht::{Lsh, Swarm};
use mds_types::{
    bare_host, canonical_region, geo_in, host_of_url, on_shelf, quantize, site_ask, tab_mask, theme_mask, valid_coords, Hit, QuantVector,
};
use mds_update::{read_model_manifest, ReleaseManifest};
use mds_rank::{page_lane, prefer_geo_radius, query_lane, Lane};
use mds_web::{
    hive_card, offer_region, page_add, page_android, page_computer, page_download, page_for_sites, page_help, page_bees, page_hive, page_how, page_indexing, page_llms, page_lost, page_manifesto, page_operator,
    AddLine,
    cabinet_sheets, render, render_results, robots_txt, sitemap_xml, stamp_census, BeeLine, CabinetSheets, MapLine, NodeChip,
    OperatorReport, PeerLine, ShellLine, VisitNumbers,
};
use mds_wire::{CrawlWork, PeerHello, PeerStatus, PushedPage, SearchHit, SearchRequest, SearchResponse};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_util::io::ReaderStream;

use crate::updates::{announce, watch};
use crate::engine::{hits_for, site_hits, ReadPool};
use crate::{Engine, NodeError};

#[derive(Clone)]
struct AppState {
    engine: Arc<Mutex<Engine>>,
    /// Чтение поиска и счётчика. Не тот замок, которым обход пишет страницы.
    readers: Arc<ReadPool>,
    lsh: Lsh,
    /// Два числа главной. Не выдача.
    census: Arc<Mutex<Option<(Instant, usize, usize)>>>,
    /// Чтение кабинета. Не тот замок, которым обход пишет страницы.
    ledger: Arc<Mutex<mds_index::Index>>,
    db_path: PathBuf,
    releases: PathBuf,
    models: PathBuf,
    downloads: PathBuf,
    peers: Vec<String>,
    self_base: String,
    client: reqwest::Client,
    swarm: Arc<Mutex<Swarm>>,
    volunteers: Arc<Mutex<HashSet<String>>>,
    region: Option<String>,
    bees: Arc<Mutex<crate::bees::BeeBook>>,
    map: Arc<Mutex<Option<mds_wire::NetworkMap>>>,
    door: Option<Arc<crate::admin::Door>>,
    nonces: Arc<crate::gate::Nonces>,
    model: String,
    limits: Arc<Mutex<HashMap<String, (u32, u64)>>>,
    shards: Arc<Mutex<mds_dht::ShardBook>>,
    visits: Arc<Mutex<crate::visits::VisitBook>>,
    shells: Arc<Mutex<crate::shell::ShellBook>>,
    dials: Arc<Mutex<Vec<String>>>,
    note: Arc<Mutex<String>>,
    face_token: String,
}

pub struct ServeConfig {
    pub listen: String,
    pub peers: Vec<String>,
    pub releases: PathBuf,
    pub models: PathBuf,
    pub downloads: PathBuf,
    pub background_policy: Option<mds_crawl::CrawlPolicy>,
    pub discover_ct: bool,
    pub hive: Option<String>,
    pub swarm_key: String,
    pub label: String,
    pub region: Option<String>,
    pub map: PathBuf,
    pub admin_listen: String,
    pub alone: bool,
    pub mcp_path: PathBuf,
    /// Публичный адрес этого улья. Вместе с соседями делит домены. Пусто — этот улей держит все.
    pub self_base: Option<String>,
}

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
    page: Option<usize>,
    adult: Option<String>,
    region: Option<String>,
    tab: Option<String>,
    theme: Option<String>,
    #[serde(default, deserialize_with = "blank_f64")]
    lat: Option<f64>,
    #[serde(default, deserialize_with = "blank_f64")]
    lon: Option<f64>,
    #[serde(default, deserialize_with = "blank_f64")]
    radius: Option<f64>,
}

fn parse_blank_f64(raw: Option<&str>) -> Option<f64> {
    let text = raw?.trim();
    if text.is_empty() {
        None
    } else {
        text.parse().ok()
    }
}

fn blank_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(parse_blank_f64(Option::<String>::deserialize(deserializer)?.as_deref()))
}

pub async fn serve(engine: Engine, config: ServeConfig) -> Result<(), NodeError> {
    let releases = config.releases.clone();
    let peers = config.peers.clone();
    tokio::spawn(watch(peers, releases));
    let db_path = engine.database_path().to_path_buf();
    let readers = Arc::new(ReadPool::open(&db_path)?);
    let ledger = Arc::new(Mutex::new(mds_index::Index::open_read(&db_path)?));
    let engine = Arc::new(Mutex::new(engine));
    if let Some(base) = config.self_base.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        engine
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .set_shard_circle(base, config.peers.clone());
    }
    let volunteers = {
        let known = engine
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .volunteers()
            .unwrap_or_default();
        Arc::new(Mutex::new(known.into_iter().collect::<HashSet<_>>()))
    };
    let bees = {
        let refused = engine
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .refused_hosts()
            .unwrap_or_default();
        let mut book = crate::bees::BeeBook::default();
        book.load_refused(refused);
        Arc::new(Mutex::new(book))
    };
    if let Some(policy) = config.background_policy.clone() {
        tokio::spawn(background_crawl_loop(
            engine.clone(),
            policy,
            config.discover_ct,
        ));
    }
    tokio::spawn(spool_loop(engine.clone()));
    tokio::spawn(load_loop(engine.clone()));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(2500))
        .build()
        .map_err(|err| NodeError::Message(err.to_string()))?;
    let mut swarm = Swarm::default();
    for peer in &config.peers {
        let base = peer.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            continue;
        }
        swarm.upsert(mds_dht::PeerCard {
            base: base.clone(),
            label: base,
            buckets: Vec::new(),
            holds_all: false,
            latency_ms: 0,
            alive: false,
            reachable: false,
            pages: 0,
            queue: 0,
        });
    }
    let swarm = Arc::new(Mutex::new(swarm));
    let dials = {
        let mut list = config.peers.clone();
        if let Ok(nodes) = engine.lock().unwrap_or_else(|poison| poison.into_inner()).known_nodes() {
            for node in nodes {
                if !list.iter().any(|item| item == &node.base) {
                    list.push(node.base);
                }
            }
        }
        Arc::new(Mutex::new(list))
    };
    tokio::spawn(poll_peers(dials.clone(), swarm.clone(), client.clone(), engine.clone()));
    if let Some(hive) = config.hive.clone() {
        tokio::spawn(dial_hive(
            hive,
            config.swarm_key.clone(),
            config.label.clone(),
            engine.clone(),
            client.clone(),
        ));
    }
    let model = model_generation(&config.models);
    let state = AppState {
        engine,
        readers,
        lsh: Lsh::new(),
        census: Arc::new(Mutex::new(None)),
        ledger,
        db_path,
        releases: config.releases,
        models: config.models,
        downloads: config.downloads,
        peers: config.peers.clone(),
        self_base: config.self_base.clone().unwrap_or_default(),
        client,
        swarm,
        volunteers,
        region: config.region.clone(),
        bees,
        map: Arc::new(Mutex::new(crate::admin::read_map(&config.map))),
        door: crate::admin::door_from_env(config.mcp_path.clone()).map(Arc::new),
        nonces: Arc::new(crate::gate::Nonces::default()),
        model,
        limits: Arc::new(Mutex::new(HashMap::new())),
        shards: Arc::new(Mutex::new(mds_dht::ShardBook::default())),
        visits: Arc::new(Mutex::new(crate::visits::VisitBook::default())),
        shells: Arc::new(Mutex::new(crate::shell::ShellBook::default())),
        dials,
        note: Arc::new(Mutex::new(String::new())),
        face_token: crate::face::ensure_face_token(
            &config
                .mcp_path
                .parent()
                .unwrap_or(std::path::Path::new("data"))
                .join("face.token"),
        )?,
    };
    if !config.alone {
        remember_map_neighbors(&state);
    }
    if !state.self_base.is_empty() && !state.peers.is_empty() {
        tokio::spawn(shard_sync_loop(
            state.engine.clone(),
            state.client.clone(),
            state.peers.clone(),
            state.self_base.clone(),
        ));
    }
    if state.door.is_some() {
        let admin = admin_router(state.clone());
        match TcpListener::bind(&config.admin_listen).await {
            Ok(listener) => {
                eprintln!("MDS: кабинет на http://{}/operator", config.admin_listen);
                tokio::spawn(async move {
                    if let Err(err) = axum::serve(
                        listener,
                        admin.into_make_service_with_connect_info::<SocketAddr>(),
                    )
                    .await
                    {
                        eprintln!("MDS: кабинет остановился: {err}");
                    }
                });
            }
            Err(err) => {
                eprintln!("MDS: кабинет не открылся: {err}");
            }
        }
    }
    let app = Router::new()
        .route("/", get(home))
        .route("/search", get(search))
        .route("/internal/face/meta", get(face_meta))
        .route("/internal/face/search", get(face_search))
        .route("/internal/face/cabinet", get(face_cabinet))
        .route("/manifesto", get(manifesto_page))
        .route("/how", get(how_page))
        .route("/help", get(help_page))
        .route("/bees", get(bees_page))
        .route("/for-sites", get(for_sites_page))
        .route("/indexing", get(indexing_page))
        .route("/android", get(android_page))
        .route("/hive", get(hive_page))
        .route("/computer", get(computer_page))
        .route("/add", get(add_page).post(add_site))
        .route("/llms", get(llms_page))
        .route("/llms.txt", get(llms_card))
        .route("/llms.md", get(llms_card))
        .route("/robots.txt", get(robots))
        .route("/sitemap.xml", get(sitemap))
        .route("/download", get(download_page))
        .route("/download/mds.exe", get(download_bin))
        .route("/download/pchela.zip", get(download_zip))
        .route("/download/pchela-linux-x64.tar.gz", get(download_linux))
        .route("/download/pchela-windows-x64.zip", get(download_windows))
        .route("/peer/search", post(peer_search))
        .route("/peer/v1/search", post(peer_search_v1))
        .route("/peer/v1/shard", post(peer_shard_v1))
        .route("/peer/shard/manifest", get(peer_shard_manifest))
        .route("/peer/shard/delta", get(peer_shard_delta))
        .route("/peer/status", get(peer_status))
        .route("/peer/join", post(peer_join))
        .route("/peer/pages", post(peer_pages))
        .route("/peer/work", get(peer_work))
        .route("/peer/fresh", post(peer_fresh))
        .route("/peer/update", get(peer_update))
        .route("/peer/update.bin", get(peer_blob))
        .route("/peer/model", get(peer_model))
        .route("/peer/model/{name}", get(peer_model_file))
        .route("/peer/notice", get(peer_notice).post(peer_notice_from))
        .route("/wire", get(wire))
        .route("/offers", get(offers))
        .route("/shell/version", get(shell_version))
        .route("/shell/work", get(shell_work))
        .route("/shell/pages", post(shell_pages))
        .route("/download/pchela-android.apk", get(download_android))
        .fallback(lost_page)
        .layer(axum::middleware::from_fn_with_state(state.clone(), count_visit))
        .with_state(state);
    let listener = TcpListener::bind(&config.listen).await?;
    eprintln!("MDS: поиск на http://{}/", config.listen);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

fn stamp_search(mut response: Response, mark: crate::visits::SearchMark) -> Response {
    response.extensions_mut().insert(mark);
    response
}

async fn lost_page() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(page_lost()))
        .unwrap_or_else(|_| StatusCode::NOT_FOUND.into_response())
}

fn person_page(path: &str) -> bool {
    matches!(
        path,
        "/" | "/search" | "/manifesto" | "/how" | "/help" | "/bees" | "/for-sites" | "/indexing" | "/android" | "/hive" | "/computer" | "/add" | "/llms" | "/download"
    )
}

fn visit_token(cookie: Option<&str>) -> (String, bool) {
    if let Some(raw) = cookie {
        for part in raw.split(';') {
            let part = part.trim();
            if let Some(value) = part.strip_prefix("mds_visit=") {
                if value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return (value.to_string(), false);
                }
            }
        }
    }
    (crate::visits::fresh_token(), true)
}

async fn count_visit(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let cookie = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut response = next.run(request).await;
    let mark = response.extensions().get::<crate::visits::SearchMark>().copied();
    let fresh = remember_visit(&state, &path, cookie.as_deref(), mark).await;
    if let Some(token) = fresh {
        if let Ok(value) = HeaderValue::from_str(&format!(
            "mds_visit={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=1800"
        )) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

async fn remember_visit(
    state: &AppState,
    path: &str,
    cookie: Option<&str>,
    mark: Option<crate::visits::SearchMark>,
) -> Option<String> {
    if !person_page(path) {
        return None;
    }
    let (token, fresh) = visit_token(cookie);
    let now = std::time::Instant::now();
    let delta = {
        let mut book = state.visits.lock().unwrap_or_else(|poison| poison.into_inner());
        let mut delta = book.page(&token, now);
        if let Some(mark) = mark {
            delta.merge(book.search(&token, now, mark));
        }
        delta
    };
    let engine = state.engine.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.add_visit(&delta)
    })
    .await;
    fresh.then_some(token)
}

#[derive(Deserialize)]
struct FacePath {
    path: Option<String>,
}

fn face_allowed(state: &AppState, peer: SocketAddr, headers: &HeaderMap) -> bool {
    if !operator_on_this_machine(peer, headers) {
        return false;
    }
    let given = headers.get("x-mds-face").and_then(|value| value.to_str().ok()).unwrap_or("");
    crate::face::token_matches(&state.face_token, given)
}

async fn face_meta(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(ask): Query<FacePath>,
) -> Result<Json<crate::face::FaceMeta>, StatusCode> {
    if !face_allowed(&state, peer, &headers) {
        return Err(StatusCode::NOT_FOUND);
    }
    let path = ask.path.unwrap_or_else(|| "/".into());
    let cookie = headers.get(header::COOKIE).and_then(|value| value.to_str().ok());
    let visit_cookie = remember_visit(&state, &path, cookie, None).await;
    let (sites, pages) = census_cached(&state);
    let notice = mds_update::load_notice(&state.releases);
    let release_version = mds_update::load_newest(&state.releases)
        .ok()
        .and_then(|found| found.map(|(manifest, _)| manifest.version));
    Ok(Json(crate::face::FaceMeta {
        sites,
        pages,
        notice,
        region: state.region.clone(),
        release_version,
        visit_cookie,
        bees_page: bees_open(&state),
    }))
}

async fn face_search(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
) -> Result<Json<crate::face::FaceSearch>, StatusCode> {
    if !face_allowed(&state, peer, &headers) {
        return Err(StatusCode::NOT_FOUND);
    }
    let paint = gather_search(&state, &headers, &params).await;
    let cookie = headers.get(header::COOKIE).and_then(|value| value.to_str().ok());
    let visit_cookie = remember_visit(&state, "/search", cookie, paint.mark).await;
    let (sites, pages) = census_cached(&state);
    let notice = mds_update::load_notice(&state.releases);
    let (offer_label, offer_href) = match paint.offer {
        Some((label, href)) => (Some(label), Some(href)),
        None => (None, None),
    };
    Ok(Json(crate::face::FaceSearch {
        query: paint.query,
        open: paint.open,
        adult: paint.adult,
        adult_open: paint.adult_open,
        page: paint.page,
        error: paint.error,
        region: paint.region,
        tab: paint.tab,
        theme: paint.theme,
        lat: paint.lat,
        lon: paint.lon,
        radius: paint.radius,
        offer_label,
        offer_href,
        refused: paint.refused,
        adult_cookie: paint.adult_cookie,
        sites,
        pages,
        notice,
        visit_cookie,
        bees_page: bees_open(&state),
    }))
}

async fn face_cabinet(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<crate::face::FaceCabinet>, StatusCode> {
    if !face_allowed(&state, peer, &headers) {
        return Err(StatusCode::NOT_FOUND);
    }
    let Some(csrf) = cabinet_csrf(&state, &headers) else {
        return Ok(Json(crate::face::FaceCabinet {
            login: true,
            report: None,
            sheets: None,
        }));
    };
    let Some((report, sheets)) = compose_cabinet(&state, &csrf, "/admin").await else {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    Ok(Json(crate::face::FaceCabinet {
        login: false,
        report: Some(report),
        sheets: Some(sheets),
    }))
}

#[derive(Deserialize)]
struct WireAsk {
    #[serde(default)]
    feed: String,
}

async fn wire(State(state): State<AppState>, Query(ask): Query<WireAsk>) -> Json<Vec<crate::wire_feed::WireLine>> {
    if ask.feed.trim().is_empty() {
        return Json(crate::wire_feed::headlines().await);
    }
    let Some(feed) = mds_types::open_feed(&ask.feed) else {
        return Json(Vec::new());
    };
    if !mds_types::press_rss(&feed) {
        let engine = state.engine.clone();
        let saved = feed.clone();
        let known = tokio::task::spawn_blocking(move || {
            engine
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .has_news_feed(&saved)
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false);
        if !known {
            return Json(Vec::new());
        }
    }
    Json(crate::wire_feed::one(&feed).await)
}

async fn offers(State(state): State<AppState>, Query(ask): Query<WireAsk>) -> Json<Vec<crate::offers::ShopLine>> {
    let Some(feed) = mds_types::open_feed(&ask.feed) else {
        return Json(Vec::new());
    };
    let engine = state.engine.clone();
    let saved = feed.clone();
    let known = tokio::task::spawn_blocking(move || {
        engine
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .has_shop_feed(&saved)
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !known {
        return Json(Vec::new());
    }
    Json(crate::offers::read(&feed).await)
}

#[derive(serde::Serialize)]
struct ShellRelease {
    version: String,
    apk: String,
}

async fn shell_version(State(state): State<AppState>) -> Json<ShellRelease> {
    let ready = state.downloads.join("pchela-android.apk").is_file();
    Json(ShellRelease {
        version: if ready { "0.1.9" } else { "0.1.0" }.to_string(),
        apk: "/download/pchela-android.apk".to_string(),
    })
}

fn pace_for_wish(
    wish: &str,
    normal: mds_index::ShellPace,
    ceiling: mds_index::ShellCeiling,
) -> mds_index::ShellPace {
    let max_batch = ceiling.max_batch.max(normal.batch).min(30);
    let min_gap = ceiling.min_gap.min(normal.gap_min).clamp(1, 60);
    match wish {
        "server" => mds_index::ShellPace {
            batch: max_batch,
            gap_min: min_gap,
            window_max: normal.window_max,
        },
        "fast" => mds_index::ShellPace {
            batch: normal.batch.max(8).min(max_batch),
            gap_min: normal.gap_min.min(2).max(min_gap),
            window_max: normal.window_max,
        },
        _ => normal,
    }
}

fn shell_header(headers: &HeaderMap) -> Option<String> {
    let id = headers.get("x-mds-shell").and_then(|value| value.to_str().ok())?.trim();
    if crate::shell::shell_id_ok(id) {
        Some(id.to_string())
    } else {
        None
    }
}

async fn shell_work(State(state): State<AppState>, headers: HeaderMap) -> Json<CrawlWork> {
    let Some(id) = shell_header(&headers) else {
        return Json(CrawlWork { urls: Vec::new(), pause_secs: 600 });
    };
    let engine = state.engine.clone();
    let saved = id.clone();
    let wish = headers
        .get("x-mds-wish")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("hive")
        .trim()
        .to_string();
    let pace = tokio::task::spawn_blocking(move || {
        let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        let _ = guard.note_shell(&saved);
        let normal = guard.shell_pace().unwrap_or(mds_index::ShellPace { batch: 2, gap_min: 10, window_max: 4 });
        let ceiling = guard.shell_ceiling().unwrap_or(mds_index::ShellCeiling { max_batch: 8, min_gap: 1 });
        let pace = pace_for_wish(&wish, normal, ceiling);
        let grants = guard.shell_grants().unwrap_or(60);
        (pace, grants)
    })
    .await
    .unwrap_or((mds_index::ShellPace { batch: 2, gap_min: 10, window_max: 4 }, 60));
    let (pace, grants) = pace;
    let per_phone = (600 / pace.gap_min.max(1)).clamp(pace.window_max, 30);
    let limits = crate::shell::ShellLimits {
        batch: pace.batch as usize,
        gap: Duration::from_secs(pace.gap_min as u64 * 60),
        window_max: per_phone as usize,
        global_max: grants as usize,
    };
    let now = std::time::Instant::now();
    let pause_secs = pace.gap_min as u64 * 60;
    if !state
        .shells
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .ready(&id, now, limits)
    {
        return Json(CrawlWork { urls: Vec::new(), pause_secs });
    }
    let hold = (pause_secs as i64).clamp(600, 1800);
    let engine = state.engine.clone();
    let batch = pace.batch as usize;
    let claimed = tokio::task::spawn_blocking(move || {
        let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.claim_urls(batch, hold).unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    let urls = state
        .shells
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take(&id, claimed.clone(), now, limits);
    if urls.len() < claimed.len() {
        let leftover: Vec<String> = claimed.into_iter().filter(|url| !urls.contains(url)).collect();
        let engine = state.engine.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
            let _ = guard.release_lease(&leftover);
        })
        .await;
    }
    Json(CrawlWork { urls, pause_secs })
}

async fn shell_pages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(pages): Json<Vec<PushedPage>>,
) -> StatusCode {
    let Some(id) = shell_header(&headers) else {
        return StatusCode::NOT_FOUND;
    };
    let now = std::time::Instant::now();
    let wish = headers
        .get("x-mds-wish")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("hive")
        .trim()
        .to_string();
    let batch_cap = {
        let guard = state.engine.lock().unwrap_or_else(|poison| poison.into_inner());
        let normal = guard.shell_pace().unwrap_or(mds_index::ShellPace {
            batch: 2,
            gap_min: 10,
            window_max: 4,
        });
        let ceiling = guard.shell_ceiling().unwrap_or(mds_index::ShellCeiling {
            max_batch: 8,
            min_gap: 1,
        });
        pace_for_wish(&wish, normal, ceiling).batch as usize
    };
    let batch: Vec<PushedPage> = {
        let mut shells = state.shells.lock().unwrap_or_else(|poison| poison.into_inner());
        pages
            .into_iter()
            .take(batch_cap)
            .filter(|page| shells.accepts(&id, &page.url, now))
            .filter_map(|page| {
                let host = host_of_url(&page.url)?;
                let text: String = page.text.chars().take(20_000).collect();
                if text.trim().chars().count() < 40 {
                    return None;
                }
                Some(PushedPage {
                    url: page.url,
                    title: page.title,
                    host: bare_host(&host),
                    text,
                    links: page.links.into_iter().take(20).collect(),
                    simhash: page.simhash,
                    anchors: page.anchors.into_iter().take(20).collect(),
                })
            })
            .collect()
    };
    if batch.is_empty() {
        return StatusCode::NO_CONTENT;
    }
    let engine = state.engine.clone();
    let stored = tokio::task::spawn_blocking(move || {
        let mut guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        let stored = guard.accept_remote_pages(batch)?;
        let _ = guard.note_shell_pages(stored as i64);
        Ok::<usize, crate::NodeError>(stored)
    })
    .await;
    if stored.ok().and_then(|item| item.ok()).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

async fn peer_search(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, StatusCode> {
    if !peer.ip().is_loopback() || !hit_limit(&state, &format!("peer:{}", peer.ip()), 30, 60) {
        return Err(StatusCode::NOT_FOUND);
    }
    if query_lane(&req.text) == Lane::Hidden {
        return Ok(Json(SearchResponse { hits: Vec::new() }));
    }
    let readers = state.readers.clone();
    let lsh = state.lsh.clone();
    let hits = tokio::task::spawn_blocking(move || {
        let vector = req.vector.dequantize();
        readers.call(|index| hits_for(index, &lsh, &req.text, &vector, 0, 0))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(SearchResponse {
        hits: hits.into_iter().map(SearchHit::from).collect(),
    }))
}

async fn peer_update(State(state): State<AppState>) -> Result<Json<ReleaseManifest>, StatusCode> {
    match mds_update::load_newest(&state.releases) {
        Ok(Some((manifest, _))) => Ok(Json(manifest)),
        _ => Err(StatusCode::NOT_FOUND),
    }
}

async fn peer_notice(State(state): State<AppState>) -> Result<Json<String>, StatusCode> {
    mds_update::load_notice(&state.releases)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn peer_notice_from(State(state): State<AppState>, Json(version): Json<String>) -> StatusCode {
    let already = mds_update::load_notice(&state.releases);
    if !mds_update::accept_notice(mds_update::VERSION, already.as_deref(), &version) {
        return StatusCode::NO_CONTENT;
    }
    if mds_update::save_notice(&state.releases, &version).is_err() {
        return StatusCode::BAD_REQUEST;
    }
    let peers = state.peers.clone();
    tokio::spawn(async move { announce(&peers, &version).await });
    StatusCode::NO_CONTENT
}

async fn peer_blob(State(state): State<AppState>) -> Result<Vec<u8>, StatusCode> {
    match mds_update::load_newest(&state.releases) {
        Ok(Some((_, bytes))) => Ok(bytes),
        _ => Err(StatusCode::NOT_FOUND),
    }
}

async fn peer_model(State(state): State<AppState>) -> Result<Json<mds_update::ModelManifest>, StatusCode> {
    let dir = state.models.join("swarm");
    if !dir.join("manifest.json").is_file() {
        return Err(StatusCode::NOT_FOUND);
    }
    match read_model_manifest(&dir) {
        Ok(manifest) => Ok(Json(manifest)),
        Err(err) => {
            eprintln!("MDS: модель для раздачи не подписана: {err}");
            Err(StatusCode::NOT_FOUND)
        }
    }
}

async fn peer_model_file(State(state): State<AppState>, Path(name): Path<String>) -> Response {
    let dir = state.models.join("swarm");
    let Ok(manifest) = read_model_manifest(&dir) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !manifest.files.iter().any(|file| file.name == name) {
        return StatusCode::NOT_FOUND.into_response();
    }
    send_file(dir.join(name), false).await
}

async fn download_android(State(state): State<AppState>) -> Response {
    let path = state.downloads.join("pchela-android.apk");
    if !path.is_file() {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("Оболочка для Android на этом улье ещё не лежит"))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    let mut response = send_file(path, true).await;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.android.package-archive"),
    );
    response
}

async fn download_linux(State(state): State<AppState>) -> Response {
    send_archive(&state, "pchela-linux-x64.tar.gz").await
}

async fn download_windows(State(state): State<AppState>) -> Response {
    send_archive(&state, "pchela-windows-x64.zip").await
}

async fn send_archive(state: &AppState, name: &str) -> Response {
    let path = state.downloads.join(name);
    if !path.is_file() {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("Архив на этом улье ещё не лежит"))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    send_file(path, true).await
}

async fn send_file(path: PathBuf, attachment: bool) -> Response {
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let len = file.metadata().await.ok().map(|meta| meta.len());
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream");
    if let Some(len) = len {
        builder = builder.header(header::CONTENT_LENGTH, len.to_string());
    }
    if attachment {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("pchela.bin");
        builder = builder.header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        );
    }
    builder
        .body(Body::from_stream(ReaderStream::new(file)))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn peer_status(State(state): State<AppState>) -> Result<Json<PeerStatus>, StatusCode> {
    let engine = state.engine.clone();
    let report = tokio::task::spawn_blocking(move || {
        let mut guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        let (buckets, holds_all) = guard.bucket_report().unwrap_or_default();
        let (searches, avg_ms) = guard.load_snapshot();
        PeerStatus {
            pages: guard.len().unwrap_or(0) as u64,
            queue: guard.queue_len().unwrap_or(0) as u64,
            buckets,
            holds_all,
            searches_per_minute: searches,
            avg_ms,
            db_bytes: guard.db_bytes(),
            region: state.region.clone().unwrap_or_default(),
            role: "full".into(),
            version: mds_update::VERSION.into(),
        }
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(report))
}

async fn peer_join(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(hello): Json<PeerHello>,
) -> StatusCode {
    let Some(bee) = bee_header(&headers) else {
        return StatusCode::NOT_FOUND;
    };
    let Some(declared) = crate::lite::site_host(&hello.site) else {
        return StatusCode::NOT_FOUND;
    };
    let host = bare_host(&declared);
    if state.bees.lock().unwrap_or_else(|poison| poison.into_inner()).is_refused(&host) {
        eprintln!("MDS: пчела сайта {host} остановлена");
        return StatusCode::NOT_FOUND;
    }
    let Some(card) = crate::bees::read_card(&declared).await else {
        eprintln!("MDS: нет карточки {}", crate::bees::card_url(&declared));
        return StatusCode::NOT_FOUND;
    };
    if !crate::bees::card_names_bee(&card, &bee) {
        eprintln!("MDS: карточка {host} не называет эту пчелу");
        return StatusCode::NOT_FOUND;
    }
    state
        .bees
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .note_verified(&bee, &host);
    let engine = state.engine.clone();
    let saved = host.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.remember_volunteer(&saved)
    })
    .await;
    state
        .volunteers
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(host.clone());
    state.swarm.lock().unwrap_or_else(|poison| poison.into_inner()).upsert(mds_dht::PeerCard {
        base: String::new(),
        label: host,
        buckets: hello.buckets,
        holds_all: false,
        latency_ms: 0,
        alive: true,
        reachable: false,
        pages: hello.pages,
        queue: hello.queue,
    });
    StatusCode::NO_CONTENT
}

async fn peer_pages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(pages): Json<Vec<PushedPage>>,
) -> Result<StatusCode, StatusCode> {
    let Some(bee) = bee_header(&headers) else {
        return Err(StatusCode::NOT_FOUND);
    };
    let Some(site) = state.bees.lock().unwrap_or_else(|poison| poison.into_inner()).host_of(&bee) else {
        return Err(StatusCode::NOT_FOUND);
    };
    let batch: Vec<PushedPage> = pages
        .into_iter()
        .take(40)
        .filter(|page| {
            state
                .bees
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .accepts(&bee, &page.url)
        })
        .filter_map(|page| {
            let host = host_of_url(&page.url)?;
            Some(PushedPage {
                url: page.url,
                title: page.title,
                host: bare_host(&host),
                text: page.text.chars().take(80_000).collect(),
                links: page.links.into_iter().take(20).collect(),
                simhash: page.simhash,
                anchors: page.anchors.into_iter().take(20).collect(),
            })
        })
        .collect();
    if batch.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }
    let engine = state.engine.clone();
    tokio::task::spawn_blocking(move || {
        let mut guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        let stored = guard.accept_remote_pages(batch)?;
        let _ = guard.note_bee_pages(&site, stored as i64);
        Ok::<usize, crate::NodeError>(stored)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn peer_work(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CrawlWork>, StatusCode> {
    let Some(bee) = bee_header(&headers) else {
        return Err(StatusCode::NOT_FOUND);
    };
    if state.bees.lock().unwrap_or_else(|poison| poison.into_inner()).host_of(&bee).is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let engine = state.engine.clone();
    let (urls, pause_secs) = tokio::task::spawn_blocking(move || {
        let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        let pace = guard.bee_pace().unwrap_or(mds_index::BeePace { batch: 4, gap_secs: 600 });
        let hold = pace.gap_secs.clamp(600, 1800);
        guard.claim_urls(pace.batch as usize, hold).map(|urls| (urls, pace.gap_secs as u64))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .bees
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .assign(&bee, urls.clone());
    Ok(Json(CrawlWork { urls, pause_secs }))
}

#[derive(Deserialize)]
struct FreshPage {
    url: String,
    simhash: u64,
}

#[derive(Deserialize)]
struct FreshAsk {
    pages: Vec<FreshPage>,
}

#[derive(Serialize)]
struct FreshAnswer {
    want: Vec<String>,
}

async fn peer_fresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(ask): Json<FreshAsk>,
) -> Result<Json<FreshAnswer>, StatusCode> {
    let Some(bee) = bee_header(&headers) else {
        return Err(StatusCode::NOT_FOUND);
    };
    if state.bees.lock().unwrap_or_else(|poison| poison.into_inner()).host_of(&bee).is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let pairs: Vec<(String, u64)> = ask
        .pages
        .into_iter()
        .take(40)
        .map(|page| (page.url, page.simhash))
        .collect();
    let engine = state.engine.clone();
    let want = tokio::task::spawn_blocking(move || {
        let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.fresh_urls(&pairs)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(FreshAnswer { want }))
}

fn bee_header(headers: &HeaderMap) -> Option<String> {
    let id = headers.get("x-mds-bee").and_then(|value| value.to_str().ok())?.trim();
    if crate::bees::bee_id_ok(id) {
        Some(id.to_string())
    } else {
        None
    }
}

async fn operator_act(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    request: Request,
) -> Response {
    let base = cabinet_base(uri.path());
    let headers = request.headers().clone();
    let Some(csrf) = cabinet_csrf(&state, &headers) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), 8_000).await else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let text = String::from_utf8_lossy(&bytes);
    if form_field(&text, "csrf") != csrf {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if form_field(&text, "action") == "issue_mcp" {
        let Some(door) = &state.door else {
            return StatusCode::NOT_FOUND.into_response();
        };
        return match door.issue_mcp() {
            Ok(secret) => Html(format!(
                "<!doctype html><meta charset=\"utf-8\"><title>Ключ MCP</title><body><main><p>Ключ показан один раз. Адрес MCP: {base}/mcp. Снаружи это https://admin.beesearch.ru/mcp.</p><pre>{}</pre><p><a href=\"{base}/operator\">Назад</a></p></main></body>",
                secret
            ))
            .into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
    }
    if form_field(&text, "action") == "revoke_mcp" {
        let Some(door) = &state.door else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let _ = door.revoke_mcp();
        return see_other(&format!("{base}/operator"));
    }
    let action = form_field(&text, "action");
    if action == "shell_pace" {
        let batch = form_field(&text, "batch").parse().unwrap_or(2);
        let gap = form_field(&text, "gap").parse().unwrap_or(10);
        let window = form_field(&text, "window").parse().unwrap_or(4);
        let engine = state.engine.clone();
        let _ = tokio::task::spawn_blocking(move || {
            engine.lock().unwrap_or_else(|poison| poison.into_inner()).set_shell_pace(batch, gap, window)
        })
        .await;
        let max_batch = form_field(&text, "max_batch").parse().unwrap_or(8);
        let min_gap = form_field(&text, "min_gap").parse().unwrap_or(1);
        let grants = form_field(&text, "grants").parse().unwrap_or(60);
        let engine = state.engine.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
            let _ = guard.set_shell_ceiling(max_batch, min_gap);
            let _ = guard.set_shell_grants(grants);
        })
        .await;
        return cabinet_note(&state, base, "Темп телефонов сохранён. Обычный режим берёт первые три числа. Быстрый режим и режим сервера не выходят за потолок. Общий потолок — на эту машину.");
    }
    if action == "refresh_pace" {
        let batch = form_field(&text, "batch").parse().unwrap_or(1);
        let gap = form_field(&text, "gap").parse().unwrap_or(120);
        let engine = state.engine.clone();
        let _ = tokio::task::spawn_blocking(move || {
            engine.lock().unwrap_or_else(|poison| poison.into_inner()).set_refresh_pace(batch, gap)
        })
        .await;
        return cabinet_note(&state, base, "Скорость переобхода сохранена. Уже лежащие обычные страницы проверяются отдельно от новых адресов.");
    }
    if action == "crawl_pace" {
        let batch = form_field(&text, "batch").parse().unwrap_or(4);
        let gap = form_field(&text, "gap").parse().unwrap_or(10);
        let engine = state.engine.clone();
        let _ = tokio::task::spawn_blocking(move || {
            engine.lock().unwrap_or_else(|poison| poison.into_inner()).set_crawl_pace(batch, gap)
        })
        .await;
        return cabinet_note(&state, base, "Скорость улья сохранена. Следующий заход обхода уже с этими числами.");
    }
    if action == "bees_page" {
        let on = form_field(&text, "on") != "0";
        let engine = state.engine.clone();
        let saved = tokio::task::spawn_blocking(move || {
            engine.lock().unwrap_or_else(|poison| poison.into_inner()).set_bees_page(on)
        })
        .await;
        if saved.ok().and_then(|result| result.ok()).is_none() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        let line = if on {
            "Раздел «Всё о пчёлах» включён. Ссылка снова на поиске."
        } else {
            "Раздел «Всё о пчёлах» выключен. Ссылки нет, адрес /bees не открывается."
        };
        return cabinet_note(&state, base, line);
    }
    if action == "bee_pace" {
        let batch = form_field(&text, "batch").parse().unwrap_or(4);
        let gap = form_field(&text, "gap").parse().unwrap_or(600);
        let engine = state.engine.clone();
        let _ = tokio::task::spawn_blocking(move || {
            engine.lock().unwrap_or_else(|poison| poison.into_inner()).set_bee_pace(batch, gap)
        })
        .await;
        *state.note.lock().unwrap_or_else(|poison| poison.into_inner()) =
            "Скорость пчёл сохранена. Улей отдаёт чужие адреса этим темпом.".into();
        return see_other(&format!("{base}/operator?sheet=bees"));
    }
    if action == "enqueue" {
        let url = form_field(&text, "url");
        if url.len() > 500 || url.is_empty() {
            return cabinet_note(&state, base, "Адрес слишком длинный.");
        }
        let check = mds_crawl::check_site(&url).await;
        if !check.queue.is_empty() {
            let engine = state.engine.clone();
            let queue = check.queue.clone();
            let _ = tokio::task::spawn_blocking(move || {
                engine.lock().unwrap_or_else(|poison| poison.into_inner()).enqueue_next(&queue)
            })
            .await;
        }
        let line = check.lines.first().map(|line| line.text.clone()).unwrap_or_else(|| "Адрес проверен.".into());
        return cabinet_note_sheet(&state, base, &line, "index");
    }
    if action == "mute" || action == "unmute" {
        let host = host_field(&form_field(&text, "host"));
        if host.is_empty() || !host.contains('.') {
            return cabinet_note_sheet(&state, base, "Нужен адрес сайта, например example.ru.", "index");
        }
        let engine = state.engine.clone();
        let saved = host.clone();
        let hiding = action == "mute";
        let changed = tokio::task::spawn_blocking(move || {
            let mut guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
            if hiding {
                guard.mute_host(&saved)
            } else {
                guard.unmute_host(&saved)
            }
        })
        .await;
        if changed.ok().and_then(|result| result.ok()).is_none() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        let line = if hiding {
            format!("Сайт {host} скрыт из выдачи этой ноды. Страницы в базе остались. Другие ульи этот список не видят.")
        } else {
            format!("Сайт {host} снова в выдаче этой ноды.")
        };
        return cabinet_note_sheet(&state, base, &line, "index");
    }
    if action == "forget_node" {
        let node = node_base(&form_field(&text, "base"));
        if let Some(node) = node {
            let engine = state.engine.clone();
            let saved = node.clone();
            let _ = tokio::task::spawn_blocking(move || {
                engine.lock().unwrap_or_else(|poison| poison.into_inner()).forget_node(&saved)
            })
            .await;
            state.dials.lock().unwrap_or_else(|poison| poison.into_inner()).retain(|item| item != &node);
            return cabinet_note(&state, base, "Нода убрана из перечня.");
        }
        return cabinet_note(&state, base, "Адрес ноды не похож на http://хост:порт.");
    }
    if action == "handshake" {
        let Some(node) = node_base(&form_field(&text, "base")) else {
            return cabinet_note(&state, base, "Адрес ноды не похож на http://хост:порт.");
        };
        let engine = state.engine.clone();
        let saved = node.clone();
        let _ = tokio::task::spawn_blocking(move || {
            engine.lock().unwrap_or_else(|poison| poison.into_inner()).remember_node(&saved)
        })
        .await;
        {
            let mut dials = state.dials.lock().unwrap_or_else(|poison| poison.into_inner());
            if !dials.iter().any(|item| item == &node) {
                dials.push(node.clone());
            }
        }
        let started = std::time::Instant::now();
        let response = state.client.get(format!("{node}/peer/status")).send().await;
        let elapsed = started.elapsed().as_millis() as u64;
        let alive = response.as_ref().map(|item| item.status().is_success()).unwrap_or(false);
        let engine = state.engine.clone();
        let saved = node.clone();
        let _ = tokio::task::spawn_blocking(move || {
            engine.lock().unwrap_or_else(|poison| poison.into_inner()).touch_node(&saved, alive, elapsed)
        })
        .await;
        let line = if alive {
            format!("Нода ответила за {elapsed} мс.")
        } else {
            "Нода не ответила. Адрес сохранён, улей спросит её ещё раз.".into()
        };
        return cabinet_note(&state, base, &line);
    }
    let host = bare_host(&form_field(&text, "host"));
    if host.is_empty() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let engine = state.engine.clone();
    let saved = host.clone();
    let action_for_db = action.clone();
    let changed = tokio::task::spawn_blocking(move || {
        let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        if action_for_db == "restore" {
            guard.restore_host(&saved)
        } else {
            guard.refuse_host(&saved)?;
            guard.forget_volunteer(&saved)
        }
    })
    .await;
    if changed.ok().and_then(|result| result.ok()).is_none() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let mut book = state.bees.lock().unwrap_or_else(|poison| poison.into_inner());
    if action == "restore" {
        book.restore(&host);
    } else {
        book.refuse(&host);
        state
            .volunteers
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&host);
    }
    drop(book);
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, format!("{base}/operator"))
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::NO_CONTENT.into_response())
}

fn form_field(body: &str, key: &str) -> String {
    body.split('&')
        .find_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            if name == key {
                Some(form_decode(value).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

fn form_decode(input: &str) -> String {
    let mut bytes = Vec::new();
    let raw = input.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'+' => {
                bytes.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < raw.len() => {
                let hex = &input[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    bytes.push(byte);
                    index += 3;
                } else {
                    bytes.push(b'%');
                    index += 1;
                }
            }
            byte => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn node_base(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_end_matches('/');
    if raw.len() > 200 || raw.contains('@') || raw.contains('\\') || raw.contains(' ') {
        return None;
    }
    let rest = raw.strip_prefix("https://").or_else(|| raw.strip_prefix("http://"))?;
    if rest.is_empty() || rest.starts_with('/') {
        return None;
    }
    Some(raw.to_string())
}

fn cabinet_note(state: &AppState, base: &str, note: &str) -> Response {
    cabinet_note_sheet(state, base, note, "node")
}

fn cabinet_note_sheet(state: &AppState, base: &str, note: &str, sheet: &str) -> Response {
    *state.note.lock().unwrap_or_else(|poison| poison.into_inner()) = note.to_string();
    see_other(&format!("{base}/operator?sheet={sheet}"))
}

fn host_field(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(host) = host_of_url(raw) {
        return bare_host(&host);
    }
    bare_host(raw.split(['/', '?', '#']).next().unwrap_or(""))
}

fn operator_on_this_machine(peer: SocketAddr, headers: &HeaderMap) -> bool {
    if !peer.ip().is_loopback() {
        return false;
    }
    let proxied = headers.contains_key("x-forwarded-for")
        || headers.contains_key("x-real-ip")
        || headers.contains_key("forwarded");
    !proxied
}

fn census_cached(state: &AppState) -> (usize, usize) {
    {
        let slot = state.census.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some((at, sites, pages)) = *slot {
            if at.elapsed() < Duration::from_secs(8) {
                return (sites, pages);
            }
        }
    }
    let counted = state.readers.call(|index| index.census().map_err(NodeError::from));
    let Ok((sites, pages)) = counted else {
        let slot = state.census.lock().unwrap_or_else(|poison| poison.into_inner());
        return slot.map(|(_, sites, pages)| (sites, pages)).unwrap_or((0, 0));
    };
    *state.census.lock().unwrap_or_else(|poison| poison.into_inner()) = Some((Instant::now(), sites, pages));
    (sites, pages)
}

fn painted(state: &AppState, html: String) -> Html<String> {
    let (sites, pages) = census_cached(state);
    let notice = mds_update::load_notice(&state.releases);
    Html(mds_web::stamp_notice(stamp_census(html, sites, pages), notice.as_deref()))
}

fn bees_open(state: &AppState) -> bool {
    state.engine.lock().unwrap_or_else(|poison| poison.into_inner()).bees_page()
}

async fn home(State(state): State<AppState>) -> Html<String> {
    painted(&state, render_results("", &[], &[], false, 1, None, state.region.as_deref(), "", "", None, None, None, bees_open(&state)))
}

async fn manifesto_page(State(state): State<AppState>) -> Html<String> {
    painted(&state, page_manifesto())
}

async fn how_page(State(state): State<AppState>) -> Html<String> {
    painted(&state, page_how())
}

async fn indexing_page(State(state): State<AppState>) -> Html<String> {
    painted(&state, page_indexing())
}

async fn android_page(State(state): State<AppState>) -> Html<String> {
    painted(&state, page_android())
}

async fn hive_page(State(state): State<AppState>) -> Html<String> {
    painted(&state, page_hive())
}

async fn computer_page(State(state): State<AppState>) -> Html<String> {
    painted(&state, page_computer())
}

async fn bees_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !bees_open(&state) {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(page_lost()))
            .unwrap_or_else(|_| StatusCode::NOT_FOUND.into_response());
    }
    let _ = headers;
    painted(&state, page_bees()).into_response()
}

async fn help_page(State(state): State<AppState>) -> Html<String> {
    painted(&state, page_help())
}

async fn for_sites_page(State(state): State<AppState>) -> Html<String> {
    painted(&state, page_for_sites())
}

async fn add_page(State(state): State<AppState>) -> Html<String> {
    painted(&state, page_add("", None, false))
}

#[derive(Deserialize)]
struct AddForm {
    url: String,
}

async fn add_site(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Form(form): Form<AddForm>,
) -> Html<String> {
    if form.url.len() > 500 || !hit_limit(&state, &format!("add:{}", peer.ip()), 20, 60) {
        let lines = [AddLine {
            ok: false,
            text: "Слишком часто или адрес слишком длинный.".into(),
        }];
        return painted(&state, page_add(&form.url, Some(&lines), false));
    }
    let check = mds_crawl::check_site(&form.url).await;
    if !check.queue.is_empty() {
        let engine = state.engine.clone();
        let queue = check.queue.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.enqueue_next(&queue)
        })
        .await;
    }
    let lines = check
        .lines
        .iter()
        .map(|line| AddLine {
            ok: line.ok,
            text: line.text.clone(),
        })
        .collect::<Vec<_>>();
    painted(&state, page_add(&form.url, Some(&lines), !check.queue.is_empty()))
}

async fn llms_page(State(state): State<AppState>) -> Html<String> {
    painted(&state, page_llms())
}

async fn llms_card() -> Response {
    text_file(hive_card(), "text/markdown; charset=utf-8")
}

async fn robots(headers: HeaderMap) -> Response {
    text_file(robots_txt(&public_origin(&headers)), "text/plain; charset=utf-8")
}

async fn sitemap(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let mut xml = sitemap_xml(&public_origin(&headers));
    if !bees_open(&state) {
        let origin = public_origin(&headers);
        xml = xml.replace(&format!("<url><loc>{origin}/bees</loc></url>\n"), "");
    }
    text_file(xml, "application/xml; charset=utf-8")
}

fn text_file(body: String, content_type: &'static str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn public_origin(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|host| {
            !host.is_empty()
                && host.chars().all(|ch| {
                    ch.is_ascii_alphanumeric() || matches!(ch, '.' | ':' | '-' | '[' | ']')
                })
        })
        .unwrap_or("127.0.0.1");
    let https = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("https"));
    let scheme = if https { "https" } else { "http" };
    format!("{scheme}://{host}")
}

async fn download_page(State(state): State<AppState>) -> Html<String> {
    let version = mds_update::load_newest(&state.releases)
        .ok()
        .and_then(|found| found.map(|(manifest, _)| manifest.version));
    painted(&state, page_download(version.as_deref()))
}

async fn download_bin(State(state): State<AppState>) -> Response {
    match mds_update::load_newest(&state.releases) {
        Ok(Some((manifest, bytes))) => {
            let filename = format!("mds-{}.exe", manifest.version);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename}\""),
                )
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("Подписанный файл ноды ещё не опубликован"))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

async fn download_zip(State(state): State<AppState>) -> Response {
    let Some((manifest, exe)) = mds_update::load_newest(&state.releases).ok().flatten() else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("Подписанный файл ноды ещё не опубликован"))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    let dll = onnx_dll();
    let urls = b"# \xd0\xbf\xd0\xbe \xd0\xbe\xd0\xb4\xd0\xbd\xd0\xbe\xd0\xbc\xd1\x83 \xd0\xb0\xd0\xb4\xd1\x80\xd0\xb5\xd1\x81\xd1\x83 \xd0\xb2 \xd1\x81\xd1\x82\xd1\x80\xd0\xbe\xd0\xba\xd0\xb5\nhttps://tehnoveles.ru/\n";
    let readme = if dll.is_some() {
        "Пчела для Windows.\r\nРядом лежат mds.exe, onnxruntime.dll и urls.txt.\r\nПолная нода:\r\nmds.exe node --listen 0.0.0.0:8080 --seeds urls.txt --follow-external --peers https://beesearch.ru\r\nЛёгкая пчела рядом с сайтом:\r\nmds.exe node --lite --site https://ваш-сайт --hive https://beesearch.ru\r\nИмя из журнала положите одной строкой в https://ваш-сайт/.well-known/pchela.txt\r\n"
    } else {
        "Пчела для Windows.\r\nВ этом архиве нет onnxruntime.dll. Положите сборку ONNX Runtime 1.20 рядом с mds.exe, в файл onnxruntime.dll.\r\n"
    };
    let mut files: Vec<(&str, &[u8])> = vec![
        ("mds.exe", exe.as_slice()),
        ("urls.txt", urls),
        ("README.txt", readme.as_bytes()),
    ];
    if let Some(bytes) = dll.as_ref() {
        files.insert(1, ("onnxruntime.dll", bytes.as_slice()));
    }
    let zip = mds_update::zip_stored(&files);
    let filename = format!("pchela-{}.zip", manifest.version);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .body(Body::from(zip))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn onnx_dll() -> Option<Vec<u8>> {
    let mut paths = vec![PathBuf::from("data/onnx/onnxruntime.dll")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("onnxruntime.dll"));
            paths.push(dir.join("onnx").join("onnxruntime.dll"));
        }
    }
    paths.into_iter().find(|path| path.is_file()).and_then(|path| std::fs::read(path).ok())
}

struct SearchPaint {
    query: String,
    open: Vec<Hit>,
    adult: Vec<Hit>,
    adult_open: bool,
    page: usize,
    error: Option<String>,
    region: Option<String>,
    tab: String,
    theme: String,
    lat: Option<f64>,
    lon: Option<f64>,
    radius: Option<f64>,
    offer: Option<(String, String)>,
    mark: Option<crate::visits::SearchMark>,
    adult_cookie: bool,
    refused: bool,
}

enum LocalReady {
    Ready(Vec<Hit>, Option<(QuantVector, u16)>),
    NeedModel,
}

async fn local_search(
    state: &AppState,
    query: &str,
    kind_mask: u32,
    theme_mask: u32,
) -> Result<(Vec<Hit>, Option<(QuantVector, u16)>), NodeError> {
    let started = Instant::now();
    let readers = state.readers.clone();
    let eng = state.engine.clone();
    let query_for_cache = query.to_string();
    let phase = tokio::task::spawn_blocking(move || {
        if let Some(ask) = site_ask(&query_for_cache) {
            let hits = readers.call(|index| site_hits(index, &ask.host, &ask.words, kind_mask, theme_mask))?;
            eng.lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .note_search(started.elapsed().as_millis());
            return Ok::<_, NodeError>(LocalReady::Ready(hits, None));
        }
        let mut guard = eng.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some((hits, vector)) = guard.cached_search(&query_for_cache, kind_mask, theme_mask) {
            let bucket = guard.semantic_bucket(&vector);
            guard.note_search(started.elapsed().as_millis());
            return Ok(LocalReady::Ready(hits, Some((vector, bucket))));
        }
        guard.set_search_first(true);
        Ok(LocalReady::NeedModel)
    })
    .await
    .map_err(|_| NodeError::Message("поиск прерван".into()))??;
    match phase {
        LocalReady::Ready(hits, ranked) => Ok((hits, ranked)),
        LocalReady::NeedModel => finish_fresh_search(state, query, kind_mask, theme_mask, started).await,
    }
}

async fn finish_fresh_search(
    state: &AppState,
    query: &str,
    kind_mask: u32,
    theme_mask: u32,
    started: Instant,
) -> Result<(Vec<Hit>, Option<(QuantVector, u16)>), NodeError> {
    let mut embedder = loop {
        let eng = state.engine.clone();
        let taken = tokio::task::spawn_blocking(move || eng.lock().unwrap_or_else(|poison| poison.into_inner()).take_model())
            .await
            .map_err(|_| NodeError::Message("поиск прерван".into()))?;
        if let Some(embedder) = taken {
            break embedder;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let query_for_model = query.to_string();
    let embedded = tokio::task::spawn_blocking(move || {
        let result = embedder.embed_query(&query_for_model);
        (embedder, result)
    })
    .await
    .map_err(|_| NodeError::Message("поиск прерван".into()))?;
    let (embedder, vector) = embedded;
    let vector = match vector {
        Ok(vector) => vector,
        Err(err) => {
            let eng = state.engine.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let mut guard = eng.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.give_model(embedder);
                guard.set_search_first(false);
            })
            .await;
            return Err(NodeError::Message(err.to_string()));
        }
    };
    let eng = state.engine.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let mut guard = eng.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.give_model(embedder);
        guard.set_search_first(false);
    })
    .await;
    let quant = quantize(&vector).map_err(|err| NodeError::Message(err.to_string()))?;
    let readers = state.readers.clone();
    let lsh = state.lsh.clone();
    let query_owned = query.to_string();
    let ranked = tokio::task::spawn_blocking(move || readers.call(|index| hits_for(index, &lsh, &query_owned, &vector, kind_mask, theme_mask)))
        .await
        .map_err(|_| NodeError::Message("поиск прерван".into()))??;
    let bucket = state.lsh.bucket(&quant.dequantize());
    let eng = state.engine.clone();
    let query_kept = query.to_string();
    let quant_kept = quant.clone();
    let hits_kept = ranked.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let mut guard = eng.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.keep_search(&query_kept, kind_mask, theme_mask, &hits_kept, quant_kept);
        guard.note_search(started.elapsed().as_millis());
    })
    .await;
    Ok((ranked, Some((quant, bucket))))
}

async fn spool_loop(engine: Arc<Mutex<Engine>>) {
    loop {
        let eng = engine.clone();
        let claimed = tokio::task::spawn_blocking(move || {
            let mut guard = eng.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.claim_passage_batch()
        })
        .await;
        let claim = match claimed {
            Ok(Ok(claim)) => claim,
            _ => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                continue;
            }
        };
        match claim {
            crate::engine::PassageClaim::Idle => tokio::time::sleep(Duration::from_millis(200)).await,
            crate::engine::PassageClaim::Yield => tokio::time::sleep(Duration::from_millis(20)).await,
            crate::engine::PassageClaim::Again => {}
            crate::engine::PassageClaim::Ready(mut batch) => {
                let embedded = batch.embed();
                let eng = engine.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let mut guard = eng.lock().unwrap_or_else(|poison| poison.into_inner());
                    match embedded {
                        Ok(vectors) => {
                            let _ = guard.finish_passage_batch(batch, vectors);
                        }
                        Err(_) => guard.cancel_passage_batch(batch),
                    }
                })
                .await;
            }
        }
    }
}

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
) -> Response {
    let paint = gather_search(&state, &headers, &params).await;
    let show_bees = bees_open(&state);
    let html = if paint.refused {
        render("", &[], 1, Some("Такой запрос эта нода не ищет."))
    } else if paint.query.is_empty() {
        render_results("", &[], &[], false, 1, None, paint.region.as_deref(), &paint.tab, &paint.theme, paint.lat, paint.lon, paint.radius, show_bees)
    } else if let Some(error) = &paint.error {
        render(&paint.query, &[], paint.page, Some(error))
    } else {
        let mut html = render_results(
            &paint.query,
            &paint.open,
            &paint.adult,
            paint.adult_open,
            paint.page,
            None,
            paint.region.as_deref(),
            &paint.tab,
            &paint.theme,
            paint.lat,
            paint.lon,
            paint.radius,
            show_bees,
        );
        if let Some((label, href)) = &paint.offer {
            html = offer_region(html, label, href);
        }
        html
    };
    let mut response = if let Some(mark) = paint.mark {
        stamp_search(painted(&state, html).into_response(), mark)
    } else {
        painted(&state, html).into_response()
    };
    if paint.adult_cookie {
        if let Ok(value) = HeaderValue::from_str("mds_adult=1; Path=/; Max-Age=2592000; SameSite=Lax; HttpOnly") {
            response.headers_mut().insert(header::SET_COOKIE, value);
        }
    }
    response
}

fn public_shelf(tab: &str, theme: &str) -> (String, String, u32, u32) {
    let mut tab = tab.trim().to_string();
    let mut theme = theme.trim().to_string();
    if theme.is_empty() && theme_mask(&tab) != 0 {
        theme = tab.clone();
        tab.clear();
    }
    let tab = match tab.as_str() {
        "news" => "news".to_string(),
        "gov" => "gov".to_string(),
        "jobs" | "job" => "jobs".to_string(),
        "services" | "service" => "services".to_string(),
        _ => String::new(),
    };
    if theme_mask(&theme) == 0 {
        theme.clear();
    }
    (tab.clone(), theme.clone(), tab_mask(&tab), theme_mask(&theme))
}

async fn gather_search(state: &AppState, headers: &HeaderMap, params: &SearchParams) -> SearchPaint {
    let query = params.q.clone().unwrap_or_default();
    let query = query.trim().to_string();
    let page = params.page.unwrap_or(1);
    let query_geo = geo_in(&query);
    let chosen = match params.region.as_deref() {
        None => state
            .region
            .clone()
            .or_else(|| query_geo.region.map(str::to_string)),
        Some("") => query_geo.region.map(str::to_string),
        Some(value) => canonical_region(value)
            .map(str::to_string)
            .or_else(|| state.region.clone())
            .or_else(|| query_geo.region.map(str::to_string)),
    };
    let (tab, theme, kind_mask, theme_bits) = public_shelf(
        params.tab.as_deref().unwrap_or(""),
        params.theme.as_deref().unwrap_or(""),
    );
    let lat = params.lat.filter(|value| value.is_finite());
    let lon = params.lon.filter(|value| value.is_finite());
    let radius = params.radius.filter(|value| value.is_finite() && *value > 0.0);
    if query.is_empty() {
        return SearchPaint {
            query,
            open: Vec::new(),
            adult: Vec::new(),
            adult_open: false,
            page,
            error: None,
            region: chosen,
            tab,
            theme,
            lat,
            lon,
            radius,
            offer: None,
            mark: None,
            adult_cookie: false,
            refused: false,
        };
    }
    if query_lane(&query) == Lane::Hidden {
        return SearchPaint {
            query,
            open: Vec::new(),
            adult: Vec::new(),
            adult_open: false,
            page,
            error: None,
            region: chosen,
            tab,
            theme,
            lat,
            lon,
            radius,
            offer: None,
            mark: Some(crate::visits::SearchMark {
                empty: false,
                failed: false,
                refused: true,
            }),
            adult_cookie: false,
            refused: true,
        };
    }
    let adult_open = params.adult.as_deref() == Some("1") || cookie_is(headers, "mds_adult", "1");
    let local = local_search(state, &query, kind_mask, theme_bits).await;
    match local {
        Ok((mut hits, ranked)) => {
            if let Some((vector, bucket)) = ranked {
                let targets = {
                    let swarm = state.swarm.lock().unwrap_or_else(|poison| poison.into_inner());
                    let asked = swarm.ask(bucket);
                    if asked.is_empty() {
                        state.peers.clone()
                    } else {
                        asked
                    }
                };
                if !targets.is_empty() {
                    let req = SearchRequest {
                        text: query.clone(),
                        vector,
                    };
                    let (remote, samples) = query_peers(&state.client, &targets, &req).await;
                    let mut swarm = state.swarm.lock().unwrap_or_else(|poison| poison.into_inner());
                    for (base, latency_ms, alive) in samples {
                        swarm.note_latency(&base, latency_ms, alive);
                    }
                    hits = merge_hits(hits, remote);
                }
            }
            let hosts = state
                .volunteers
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone();
            mark_volunteers(&mut hits, &hosts);
            let (open, adult) = split_lanes(hits);
            let mut open = open;
            open.retain(|hit| on_shelf(hit.kind, kind_mask, theme_bits));
            let around = match (params.lat, params.lon, params.radius) {
                (Some(lat), Some(lon), Some(r)) if valid_coords(lat, lon) && r.is_finite() && r > 0.0 => {
                    Some((lat, lon, r.clamp(0.1, 500.0)))
                }
                _ => None,
            };
            let open = prefer_geo_radius(open, chosen.as_deref(), query_geo.place, around);
            let logged = open.iter().cloned().chain(adult.iter().cloned()).collect::<Vec<_>>();
            let engine = state.engine.clone();
            let query_for_log = query.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.remember_query(&query_for_log, &logged)
            })
            .await;
            let offer = region_button(state, &query, &tab, &theme, lat, lon, radius).await;
            let empty = open.is_empty() && adult.is_empty();
            SearchPaint {
                query,
                open,
                adult,
                adult_open,
                page,
                error: None,
                region: chosen,
                tab,
                theme: theme.clone(),
                lat,
                lon,
                radius,
                offer,
                mark: Some(crate::visits::SearchMark {
                    empty,
                    failed: false,
                    refused: false,
                }),
                adult_cookie: params.adult.as_deref() == Some("1"),
                refused: false,
            }
        }
        Err(err) => SearchPaint {
            query,
            open: Vec::new(),
            adult: Vec::new(),
            adult_open,
            page,
            error: Some(err.to_string()),
            region: chosen,
            tab,
            theme,
            lat,
            lon,
            radius,
            offer: None,
            mark: Some(crate::visits::SearchMark {
                empty: false,
                failed: true,
                refused: false,
            }),
            adult_cookie: params.adult.as_deref() == Some("1"),
            refused: false,
        },
    }
}

async fn compose_cabinet(state: &AppState, csrf: &str, base: &str) -> Option<(OperatorReport, CabinetSheets)> {
    let hosts: Vec<String> = state
        .volunteers
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .iter()
        .cloned()
        .collect();
    let engine = state.engine.clone();
    let ledger = state.ledger.clone();
    let db_path = state.db_path.clone();
    let hosts_for_db = hosts.clone();
    let rows = tokio::task::spawn_blocking(move || {
        let avg_ms = {
            let mut writing = engine.lock().unwrap_or_else(|poison| poison.into_inner());
            writing.load_snapshot().1
        };
        let guard = ledger.lock().unwrap_or_else(|poison| poison.into_inner());
        let searches = guard.queries_last_minute().unwrap_or(0) as u64;
        let pace = guard.shell_pace().unwrap_or(mds_index::ShellPace { batch: 2, gap_min: 10, window_max: 4 });
        let span = |name: &str| guard.shell_span(name).unwrap_or((0, 0));
        let shells = (span("day"), span("week"), span("month"), span("all"), pace);
        let bees = hosts_for_db
            .iter()
            .map(|host| {
                let totals = guard.bee_totals(host).unwrap_or(mds_index::BeeTotals {
                    day: 0, week: 0, month: 0, all: 0, quiet: true,
                });
                (host.clone(), totals)
            })
            .collect::<Vec<_>>();
        let sites = guard.census().map(|(sites, _)| sites).unwrap_or(0) as u64;
        let crawl = guard.crawl_pace().unwrap_or(mds_index::CrawlPace { batch: 4, gap_secs: 10 });
        let bee_pace = guard.bee_pace().unwrap_or(mds_index::BeePace { batch: 4, gap_secs: 600 });
        let ceiling = guard.shell_ceiling().unwrap_or(mds_index::ShellCeiling { max_batch: 8, min_gap: 1 });
        let refresh = guard.refresh_pace().unwrap_or(mds_index::RefreshPace { batch: 1, gap_secs: 120 });
        let intake_days = guard
            .intake_days(30)
            .unwrap_or_default()
            .into_iter()
            .map(|day| (day.day, day.new_pages))
            .collect::<Vec<_>>();
        let load = guard.load_hours(24).unwrap_or_default();
        let activity = mds_web::ActivityBook {
            minutes: map_activity(guard.activity_span("minute", 60).unwrap_or_default()),
            hours: map_activity(guard.activity_span("hour", 48).unwrap_or_default()),
            days: map_activity(guard.activity_span("day", 30).unwrap_or_default()),
            weeks: map_activity(guard.activity_span("week", 16).unwrap_or_default()),
            months: map_activity(guard.activity_span("month", 12).unwrap_or_default()),
        };
        let picture = (
            sites,
            guard.intake_sum("new_pages", "day").unwrap_or(0),
            guard.intake_sum("new_pages", "yesterday").unwrap_or(0),
            guard.intake_sum("new_pages", "week").unwrap_or(0),
            guard.intake_sum("new_pages", "month").unwrap_or(0),
            guard.intake_sum("changed_pages", "day").unwrap_or(0),
            guard.intake_sum("same_pages", "day").unwrap_or(0),
            guard.intake_sum("new_hosts", "day").unwrap_or(0),
            crawl.batch,
            crawl.gap_secs,
            bee_pace.batch,
            bee_pace.gap_secs,
            ceiling.max_batch,
            ceiling.min_gap,
            intake_days,
            load.iter().map(|hour| (hour.hour.clone(), hour.cpu_pct)).collect::<Vec<_>>(),
            load.iter().map(|hour| (hour.hour.clone(), hour.ram_pct)).collect::<Vec<_>>(),
            guard.leased_count().unwrap_or(0).max(0) as u64,
            guard.refresh_due().unwrap_or(0),
            refresh.batch,
            refresh.gap_secs,
            guard.shell_grants().unwrap_or(60),
        );
        let report_head = (
            guard.len().unwrap_or(0) as u64,
            guard.queue_len().unwrap_or(0) as u64,
            std::fs::metadata(&db_path).map(|meta| meta.len()).unwrap_or(0),
            searches,
            avg_ms,
        );
        guard.recent_queries(50).map(|rows| (report_head, shells, bees, rows, picture, activity))
    })
    .await;
    let peers = {
        let swarm = state.swarm.lock().unwrap_or_else(|poison| poison.into_inner());
        swarm
            .cards()
            .iter()
            .map(|card| PeerLine {
                name: if card.base.is_empty() {
                    card.label.clone()
                } else {
                    card.base.clone()
                },
                latency_ms: card.latency_ms,
                alive: card.alive,
                buckets: if card.holds_all {
                    65_536
                } else {
                    card.buckets.len()
                },
                pages: card.pages,
            })
            .collect::<Vec<_>>()
    };
    let packed = match rows {
        Ok(Ok(((pages, queue, db_bytes, searches_per_minute, avg_ms), shells, bees, rows, picture, activity))) => {
            let view: Vec<(String, i64, i64, i64, i64)> = rows
                .into_iter()
                .map(|row| (row.text, row.results, row.words, row.meaning, row.adult))
                .collect();
            let (day, week, month, all, pace) = shells;
            let sheets = cabinet_view(&state, &csrf, base, &hosts);
            let note = std::mem::take(&mut *state.note.lock().unwrap_or_else(|poison| poison.into_inner()));
            let report = OperatorReport {
                    pages,
                    queue,
                    db_bytes,
                    searches_per_minute,
                    avg_ms,
                    peers,
                    shells: ShellLine {
                        day_apps: day.0,
                        day_pages: day.1,
                        week_apps: week.0,
                        week_pages: week.1,
                        month_apps: month.0,
                        month_pages: month.1,
                        all_apps: all.0,
                        all_pages: all.1,
                        batch: pace.batch,
                        gap_min: pace.gap_min,
                        window_max: pace.window_max,
                        max_batch: picture.12,
                        min_gap: picture.13,
                        grants: picture.21,
                    },
                    bees: bees
                        .into_iter()
                        .map(|(host, totals)| BeeLine {
                            host,
                            day: totals.day,
                            week: totals.week,
                            month: totals.month,
                            all: totals.all,
                            quiet: totals.quiet,
                        })
                        .collect(),
                    refused: state.bees.lock().unwrap_or_else(|poison| poison.into_inner()).refused(),
                    rows: view,
                    note,
                    csrf: csrf.to_string(),
                    base: base.to_string(),
                    sites: picture.0,
                    new_today: picture.1,
                    new_yesterday: picture.2,
                    new_week: picture.3,
                    new_month: picture.4,
                    changed_today: picture.5,
                    same_today: picture.6,
                    hosts_today: picture.7,
                    crawl_batch: picture.8,
                    crawl_gap: picture.9,
                    bee_batch: picture.10,
                    bee_gap: picture.11,
                    intake_days: picture.14,
                    cpu_hours: picture.15,
                    ram_hours: picture.16,
                    leased: picture.17,
                    refresh_due: picture.18,
                    refresh_batch: picture.19,
                    refresh_gap: picture.20,
                    span: String::new(),
                    activity,
                    bees_page: state.engine.lock().unwrap_or_else(|poison| poison.into_inner()).bees_page(),
                };
            Some((report, sheets))
        }
        _ => None,
    };
    packed
}

async fn operator(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let base = cabinet_base(uri.path());
    let Some(csrf) = cabinet_csrf(&state, &headers) else {
        return see_other(&format!("{base}/login"));
    };
    match compose_cabinet(&state, &csrf, base).await {
        Some((mut report, sheets)) => {
            report.span = query_value(&uri, "span");
            let sheet = query_sheet(&uri);
            let (stats, rest) = cabinet_sheets(&sheets, &sheet);
            painted(&state, page_operator(&report, &stats, &rest, &sheet)).into_response()
        }
        None => painted(&state, render("", &[], 1, Some("журнал не открылся"))).into_response(),
    }
}

fn cookie_is(headers: &HeaderMap, name: &str, value: &str) -> bool {
    let Some(raw) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    raw.split(';').any(|part| {
        part.trim()
            .split_once('=')
            .map(|(key, stored)| key == name && stored == value)
            .unwrap_or(false)
    })
}

fn split_lanes(hits: Vec<Hit>) -> (Vec<Hit>, Vec<Hit>) {
    let mut open = Vec::new();
    let mut adult = Vec::new();
    for hit in hits {
        let lane = page_lane(&hit.url, &hit.title, &hit.snippet);
        if lane == Lane::Hidden {
            continue;
        }
        if hit.adult || lane == Lane::Adult {
            adult.push(hit);
        } else {
            open.push(hit);
        }
    }
    (open, adult)
}

async fn query_peers(
    client: &reqwest::Client,
    peers: &[String],
    req: &SearchRequest,
) -> (Vec<Hit>, Vec<(String, u64, bool)>) {
    let mut set = tokio::task::JoinSet::new();
    for peer in peers {
        let client = client.clone();
        let req = req.clone();
        let peer = peer.clone();
        set.spawn(async move {
            let started = std::time::Instant::now();
            let hits = query_one(&client, &peer, &req).await;
            let alive = hits.is_some();
            (
                peer,
                started.elapsed().as_millis() as u64,
                alive,
                hits.unwrap_or_default(),
            )
        });
    }
    let mut out = Vec::new();
    let mut samples = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    loop {
        let next = tokio::time::timeout_at(deadline, set.join_next()).await;
        let Ok(joined) = next else {
            break;
        };
        let Some(joined) = joined else {
            break;
        };
        if let Ok((peer, latency_ms, alive, hits)) = joined {
            samples.push((peer, latency_ms, alive));
            out.extend(hits);
        }
    }
    (out, samples)
}

async fn query_one(client: &reqwest::Client, peer: &str, req: &SearchRequest) -> Option<Vec<Hit>> {
    let Ok(payload) = serde_json::to_vec(req) else {
        return None;
    };
    let base = peer.trim_end_matches('/');
    let url = format!("{base}/peer/search");
    let Ok(resp) = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload)
        .send()
        .await
    else {
        return None;
    };
    if !resp.status().is_success() {
        return None;
    }
    let Ok(bytes) = resp.bytes().await else {
        return None;
    };
    let Ok(body) = serde_json::from_slice::<SearchResponse>(&bytes) else {
        return None;
    };
    Some(
        body.hits
            .into_iter()
            .map(|hit| Hit {
                url: hit.url,
                title: hit.title,
                snippet: hit.snippet,
                score: hit.score,
                via_words: hit.via_words,
                via_meaning: hit.via_meaning,
                adult: hit.adult,
                volunteer: hit.volunteer,
                region: hit.region,
                place: hit.place,
                distance_km: hit.distance_km,
                lat: hit.lat,
                lon: hit.lon,
                kind: hit.kind,
                media_url: hit.media_url,
                feed_url: hit.feed_url,
                news_feed: hit.news_feed,
                price: hit.price,
            })
            .collect(),
    )
}

fn merge_hits(local: Vec<Hit>, remote: Vec<Hit>) -> Vec<Hit> {
    let mut by_url: HashMap<String, Hit> = HashMap::new();
    for hit in local.into_iter().chain(remote) {
        by_url
            .entry(hit.url.clone())
            .and_modify(|existing| {
                if hit.score > existing.score {
                    existing.score = hit.score;
                    existing.title = hit.title.clone();
                    existing.snippet = hit.snippet.clone();
                    existing.region = hit.region.clone();
                    existing.place = hit.place.clone();
                    existing.lat = hit.lat.or(existing.lat);
                    existing.lon = hit.lon.or(existing.lon);
                } else {
                    if existing.region.is_none() {
                        existing.region = hit.region.clone();
                    }
                    if existing.place.is_none() {
                        existing.place = hit.place.clone();
                    }
                    if existing.lat.is_none() {
                        existing.lat = hit.lat;
                    }
                    if existing.lon.is_none() {
                        existing.lon = hit.lon;
                    }
                }
                existing.via_words |= hit.via_words;
                existing.via_meaning |= hit.via_meaning;
            })
            .or_insert(hit);
    }
    let mut merged: Vec<Hit> = by_url.into_values().collect();
    merged.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    merged
}

fn mark_volunteers(hits: &mut Vec<Hit>, hosts: &HashSet<String>) {
    if hosts.is_empty() {
        return;
    }
    for hit in hits.iter_mut() {
        let Some(host) = mds_dht::host_of(&hit.url) else {
            continue;
        };
        if hosts.contains(&host) {
            hit.volunteer = true;
            hit.score = mds_rank::nudge_volunteer(hit.score);
        }
    }
    hits.sort_by(|left, right| right.score.partial_cmp(&left.score).unwrap_or(std::cmp::Ordering::Equal));
}

async fn poll_peers(dials: Arc<Mutex<Vec<String>>>, swarm: Arc<Mutex<Swarm>>, client: reqwest::Client, engine: Arc<Mutex<Engine>>) {
    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;
        let peers = dials.lock().unwrap_or_else(|poison| poison.into_inner()).clone();
        for peer in &peers {
            let base = peer.trim().trim_end_matches('/');
            let started = std::time::Instant::now();
            let response = client.get(format!("{base}/peer/status")).send().await;
            let elapsed = started.elapsed().as_millis() as u64;
            let Ok(response) = response else {
                let mut guard = swarm.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.note_latency(base, elapsed, false);
                let _ = engine.lock().unwrap_or_else(|poison| poison.into_inner()).touch_node(base, false, elapsed);
                continue;
            };
            if !response.status().is_success() {
                let mut guard = swarm.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.note_latency(base, elapsed, false);
                let _ = engine.lock().unwrap_or_else(|poison| poison.into_inner()).touch_node(base, false, elapsed);
                continue;
            }
            let Ok(bytes) = response.bytes().await else {
                continue;
            };
            let Ok(status) = serde_json::from_slice::<PeerStatus>(&bytes) else {
                continue;
            };
            let reachable = status.holds_all || !status.buckets.is_empty();
            let mut guard = swarm.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.upsert(mds_dht::PeerCard {
                base: base.to_string(),
                label: base.to_string(),
                buckets: status.buckets,
                holds_all: status.holds_all,
                latency_ms: elapsed,
                alive: true,
                reachable,
                pages: status.pages.min(u64::from(u32::MAX)) as u32,
                queue: status.queue.min(u64::from(u32::MAX)) as u32,
            });
            drop(guard);
            let _ = engine.lock().unwrap_or_else(|poison| poison.into_inner()).touch_node(base, true, elapsed);
        }
    }
}

async fn dial_hive(
    hive: String,
    key: String,
    label: String,
    engine: Arc<Mutex<Engine>>,
    client: reqwest::Client,
) {
    let hive = hive.trim().trim_end_matches('/').to_string();
    loop {
        tokio::time::sleep(Duration::from_secs(20)).await;
        let snapshot = {
            let engine = engine.clone();
            tokio::task::spawn_blocking(move || {
                let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
                let (buckets, holds_all) = guard.bucket_report().unwrap_or_default();
                let pages = guard.pages_for_hive(4).unwrap_or_default();
                (
                    guard.len().unwrap_or(0) as u32,
                    guard.queue_len().unwrap_or(0) as u32,
                    buckets,
                    holds_all,
                    pages,
                )
            })
            .await
        };
        let Ok((pages, queue, buckets, holds_all, batch)) = snapshot else {
            continue;
        };
        let hello = PeerHello {
            base: String::new(),
            label: label.clone(),
            buckets,
            holds_all,
            pages,
            queue,
            site: String::new(),
        };
        let hello_body = serde_json::to_vec(&hello).unwrap_or_default();
        let _ = client
            .post(format!("{hive}/peer/join"))
            .header("x-mds-swarm", &key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(hello_body)
            .send()
            .await;
        if batch.is_empty() {
            continue;
        }
        let pushed: Vec<PushedPage> = batch
            .into_iter()
            .map(|(url, title, host, text)| PushedPage {
                url,
                title,
                host,
                text,
                links: Vec::new(),
                simhash: 0,
                anchors: Vec::new(),
            })
            .collect();
        let pushed_body = serde_json::to_vec(&pushed).unwrap_or_default();
        let _ = client
            .post(format!("{hive}/peer/pages"))
            .header("x-mds-swarm", &key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(pushed_body)
            .send()
            .await;
    }
}

async fn load_loop(engine: Arc<Mutex<Engine>>) {
    let mut previous = cpu_ticks();
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let cpu = cpu_percent(&mut previous);
        let ram = ram_percent();
        if cpu.is_none() && ram.is_none() {
            continue;
        }
        let cpu = cpu.unwrap_or(0);
        let ram = ram.unwrap_or(0);
        let eng = engine.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let guard = eng.lock().unwrap_or_else(|p| p.into_inner());
            let _ = guard.note_load(cpu, ram);
        })
        .await;
    }
}

fn cpu_ticks() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    let line = text.lines().next()?;
    let mut all = 0_u64;
    let mut idle = 0_u64;
    for (index, part) in line.split_whitespace().skip(1).enumerate() {
        let value: u64 = part.parse().ok()?;
        all += value;
        if index == 3 || index == 4 {
            idle += value;
        }
    }
    Some((all, idle))
}

fn cpu_percent(previous: &mut Option<(u64, u64)>) -> Option<i64> {
    let now = cpu_ticks()?;
    let result = if let Some((all, idle)) = *previous {
        let spent = now.0.saturating_sub(all);
        let rested = now.1.saturating_sub(idle);
        if spent == 0 {
            None
        } else {
            Some(((spent.saturating_sub(rested)) * 100 / spent) as i64)
        }
    } else {
        None
    };
    *previous = Some(now);
    result
}

fn ram_percent() -> Option<i64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total = 0_i64;
    let mut available = 0_i64;
    for line in text.lines() {
        let number = |prefix: &str| -> Option<i64> {
            line.strip_prefix(prefix)?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        };
        if let Some(value) = number("MemTotal:") {
            total = value;
        }
        if let Some(value) = number("MemAvailable:") {
            available = value;
        }
    }
    if total <= 0 {
        return None;
    }
    Some(((total - available).clamp(0, total) * 100 / total).clamp(0, 100))
}

async fn refresh_due(engine: &Arc<Mutex<Engine>>, crawler: &mut mds_crawl::Crawler, last_refresh: &mut std::time::Instant) {
    let pace = {
        let eng = engine.clone();
        tokio::task::spawn_blocking(move || {
            let guard = eng.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.refresh_pace().unwrap_or(mds_index::RefreshPace { batch: 1, gap_secs: 120 })
        })
        .await
        .unwrap_or(mds_index::RefreshPace { batch: 1, gap_secs: 120 })
    };
    if last_refresh.elapsed() < Duration::from_secs(pace.gap_secs.max(60) as u64) {
        return;
    }
    *last_refresh = std::time::Instant::now();
    let jobs = {
        let eng = engine.clone();
        let limit = pace.batch.clamp(1, 4) as usize;
        tokio::task::spawn_blocking(move || {
            let guard = eng.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.claim_refresh(limit).unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    };
    for job in jobs {
        let hit = crawler.probe(&job.url, job.etag.as_deref(), job.modified.as_deref()).await;
        let url = job.url.clone();
        let eng = engine.clone();
        let status = hit.status;
        let etag = hit.etag;
        let modified = hit.modified;
        let page = hit.page;
        let gone = hit.gone;
        let skipped = hit.skipped;
        let failed = hit.failed;
        let _ = tokio::task::spawn_blocking(move || {
            let mut guard = eng.lock().unwrap_or_else(|poison| poison.into_inner());
            if gone {
                let _ = guard.drop_urls(&[url]);
                return;
            }
            if skipped {
                let _ = guard.refresh_after(&url, 14 * 86_400);
                return;
            }
            if failed {
                let _ = guard.refresh_retry(&url);
                return;
            }
            if let Some(page) = page {
                let _ = guard.store_refresh(page, etag, modified);
                return;
            }
            if status == 304 {
                let _ = guard.refresh_quiet(&url, etag.as_deref(), modified.as_deref());
            }
        })
        .await;
    }
}

async fn background_crawl_loop(
    engine: Arc<Mutex<Engine>>,
    policy: mds_crawl::CrawlPolicy,
    discover_ct: bool,
) {
    let mut step_policy = policy.clone();
    step_policy.max_pages = step_policy.max_pages.clamp(1, 6);
    let Ok(mut crawler) = mds_crawl::Crawler::new(step_policy) else {
        return;
    };
    let ct_queries = ["%.ru", "%.su", "%.com", "%.org"];
    let mut ct_round = 0_usize;
    let mut last_refresh = std::time::Instant::now();

    loop {
        let pace = {
            let eng = engine.clone();
            tokio::task::spawn_blocking(move || {
                let guard = eng.lock().unwrap_or_else(|p| p.into_inner());
                guard.crawl_pace().unwrap_or(mds_index::CrawlPace { batch: 4, gap_secs: 10 })
            })
            .await
            .unwrap_or(mds_index::CrawlPace { batch: 4, gap_secs: 10 })
        };
        tokio::time::sleep(Duration::from_secs(pace.gap_secs.max(2) as u64)).await;
        refresh_due(&engine, &mut crawler, &mut last_refresh).await;
        crawler.set_limit(pace.batch.clamp(1, 24) as usize);
        let batch = {
            let eng = engine.clone();
            let limit = pace.batch.clamp(1, 24) as usize;
            let hold = pace.gap_secs.saturating_mul(2).clamp(4, 1800);
            tokio::task::spawn_blocking(move || {
                let guard = eng.lock().unwrap_or_else(|p| p.into_inner());
                guard.claim_urls(limit, hold).unwrap_or_default()
            })
            .await
            .unwrap_or_default()
        };

        if batch.is_empty() {
            if discover_ct {
                let q = [ct_queries[ct_round % ct_queries.len()]];
                ct_round = ct_round.wrapping_add(1);
                let found = mds_crawl::discover_ct_domains(&q, 25).await;
                if !found.is_empty() {
                    let eng = engine.clone();
                    let count = found.len();
                    let _ = tokio::task::spawn_blocking(move || {
                        let guard = eng.lock().unwrap_or_else(|p| p.into_inner());
                        let _ = guard.enqueue_urls(&found);
                    })
                    .await;
                    eprintln!("MDS: из CT-логов добавлено {count} новых доменов в очередь");
                    continue;
                }
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
            continue;
        }

        let pages = match crawler.crawl(&batch).await {
            Ok(pages) => pages,
            Err(_) => {
                let failed = batch.clone();
                let eng = engine.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let guard = eng.lock().unwrap_or_else(|p| p.into_inner());
                    for url in &failed {
                        if let Some(host) = host_of_url(url) {
                            let _ = guard.note_host_quiet(&host);
                        }
                    }
                    let _ = guard.mark_seen(&failed);
                })
                .await;
                continue;
            }
        };
        let gone = crawler.take_gone();
        let mut visited: Vec<String> = pages.iter().map(|p| p.url.clone()).collect();
        visited.extend(batch);
        let mut discovered = Vec::new();
        for page in &pages {
            discovered.extend(page.links.iter().cloned());
        }
        let eng = engine.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let mut guard = eng.lock().unwrap_or_else(|p| p.into_inner());
            let _ = guard.record_crawl_step(&visited, &discovered, &gone, pages);
        })
        .await;
    }
}

fn admin_router(state: AppState) -> Router {
    let door = Router::new()
        .route("/login", get(login_page).post(login_act))
        .route("/logout", post(logout_act))
        .route("/operator", get(operator).post(operator_act))
        .route("/mcp", post(mcp_act));
    let prefixed = door.clone().route("/", get(admin_entry));
    Router::new()
        .merge(door)
        .nest("/admin", prefixed)
        .fallback(lost_page)
        .with_state(state)
}

fn cabinet_base(path: &str) -> &'static str {
    if path == "/admin" || path.starts_with("/admin/") {
        "/admin"
    } else {
        ""
    }
}

fn see_other(location: &str) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::NO_CONTENT.into_response())
}

fn session_cookie(token: &str, base: &str, max_age: u32) -> String {
    let path = if base.is_empty() { "/" } else { base };
    format!("mds_session={token}; HttpOnly; SameSite=Strict; Path={path}; Secure; Max-Age={max_age}")
}

async fn admin_entry() -> Response {
    see_other("/admin/login")
}

async fn login_page(OriginalUri(uri): OriginalUri) -> Html<String> {
    Html(crate::admin::login_page(false, cabinet_base(uri.path())))
}

#[derive(Deserialize)]
struct LoginForm {
    user: String,
    password: String,
}

async fn login_act(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    OriginalUri(uri): OriginalUri,
    Form(form): Form<LoginForm>,
) -> Response {
    let base = cabinet_base(uri.path());
    let Some(door) = &state.door else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !door.allow(&format!("login:{}", peer.ip()), 8, 60) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let Some((token, _)) = door.login(&form.user, &form.password) else {
        return Html(crate::admin::login_page(true, base)).into_response();
    };
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, format!("{base}/operator"))
        .header(header::SET_COOKIE, session_cookie(&token, base, 43200))
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::NO_CONTENT.into_response())
}

async fn logout_act(State(state): State<AppState>, OriginalUri(uri): OriginalUri, headers: HeaderMap) -> Response {
    let base = cabinet_base(uri.path());
    if let (Some(door), Some(token)) = (&state.door, crate::admin::cookie_token(&headers)) {
        door.logout(&token);
    }
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, format!("{base}/login"))
        .header(header::SET_COOKIE, session_cookie("", base, 0))
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::NO_CONTENT.into_response())
}

async fn mcp_act(State(state): State<AppState>, headers: HeaderMap, request: axum::extract::Request) -> Response {
    let Some(door) = &state.door else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("");
    if !door.allow(&format!("mcp:{bearer}"), 60, 60) || !door.mcp_ok(bearer) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), 64 * 1024).await else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let map = state.map.lock().unwrap_or_else(|poison| poison.into_inner()).clone();
    let engine = state.engine.clone();
    let answer = tokio::task::spawn_blocking(move || {
        let mut guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        crate::mcp::handle(&mut guard, map.as_ref(), &text, true)
    })
    .await;
    match answer {
        Ok((status, body)) => Response::builder()
            .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn query_sheet(uri: &axum::http::Uri) -> String {
    let sheet = query_value(uri, "sheet");
    if sheet.is_empty() { "node".into() } else { sheet }
}

fn query_value(uri: &axum::http::Uri, name: &str) -> String {
    uri.query()
        .and_then(|query| {
            query.split('&').find_map(|part| {
                let (key, value) = part.split_once('=')?;
                if key == name { Some(value.to_string()) } else { None }
            })
        })
        .unwrap_or_default()
}

fn map_activity(slots: Vec<mds_index::ActivitySlot>) -> Vec<mds_web::ActivityPoint> {
    slots
        .into_iter()
        .map(|slot| mds_web::ActivityPoint {
            label: slot.bucket,
            new_pages: slot.new_pages,
            new_hosts: slot.new_hosts,
            walked: slot.walked,
            shell_pages: slot.shell_pages,
            cpu: slot.cpu,
            ram: slot.ram,
        })
        .collect()
}

fn cabinet_csrf(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let door = state.door.as_ref()?;
    let token = crate::admin::cookie_token(headers)?;
    door.session(&token)
}

fn cabinet_view(state: &AppState, csrf: &str, base: &str, bees: &[String]) -> CabinetSheets {
    let guard = state.ledger.lock().unwrap_or_else(|poison| poison.into_inner());
    let day = guard.query_counts("day", 20).unwrap_or_default();
    let week = guard.query_counts("week", 20).unwrap_or_default();
    let month = guard.query_counts("month", 20).unwrap_or_default();
    let visits_day = guard.visit_span("day").unwrap_or_default();
    let visits_week = guard.visit_span("week").unwrap_or_default();
    let visits_month = guard.visit_span("month").unwrap_or_default();
    let days = guard.visit_days(30).unwrap_or_default();
    let pages = guard.list_index(None, None, 0, 20).unwrap_or_default();
    let kind_counts = guard.kind_counts().unwrap_or_default();
    let nodes = guard.known_nodes().unwrap_or_default();
    let guard_muted = guard.muted_hosts().unwrap_or_default();
    drop(guard);
    let map = state.map.lock().unwrap_or_else(|poison| poison.into_inner());
    let (map_version, map_places) = if let Some(map) = map.as_ref() {
        let places = map
            .places
            .iter()
            .map(|place| MapLine {
                country: place.place.country.clone(),
                region: place.place.region.clone(),
                door: place
                    .hives
                    .first()
                    .map(|door| door.base.clone())
                    .unwrap_or_else(|| "нет входа".into()),
            })
            .collect();
        (Some(map.version), places)
    } else {
        (None, Vec::new())
    };
    CabinetSheets {
        version: mds_update::VERSION.into(),
        region_label: state.region.clone().unwrap_or_else(|| "не задан".into()),
        model: state.model.clone(),
        days: days.into_iter().map(|day| (day.day, day.visits, day.searches)).collect(),
        visits_day: visit_numbers(visits_day),
        visits_week: visit_numbers(visits_week),
        visits_month: visit_numbers(visits_month),
        queries_day: day,
        queries_week: week,
        queries_month: month,
        index_urls: pages.into_iter().map(|page| page.url).collect(),
        kind_counts,
        muted: guard_muted,
        bee_hosts: bees.to_vec(),
        nodes: nodes
            .into_iter()
            .map(|node| NodeChip {
                base: node.base,
                alive: node.alive,
                latency_ms: node.latency_ms,
            })
            .collect(),
        map_version,
        map_places,
        csrf: csrf.to_string(),
        base: base.to_string(),
    }
}

fn visit_numbers(report: mds_index::VisitReport) -> VisitNumbers {
    VisitNumbers {
        visits: report.visits,
        views: report.views,
        searches: report.searches,
        bounces: report.bounces,
        dwell_seconds: report.dwell_seconds,
        timed_visits: report.timed_visits,
        empty_searches: report.empty_searches,
        search_errors: report.search_errors,
        refused: report.refused,
    }
}

fn hit_limit(state: &AppState, key: &str, limit: u32, window_secs: u64) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|span| span.as_secs())
        .unwrap_or(0);
    let mut hits = state.limits.lock().unwrap_or_else(|poison| poison.into_inner());
    let entry = hits.entry(key.to_string()).or_insert((0, now));
    if now.saturating_sub(entry.1) >= window_secs {
        *entry = (0, now);
    }
    if entry.0 >= limit {
        return false;
    }
    entry.0 += 1;
    true
}

fn model_generation(models: &std::path::Path) -> String {
    std::fs::read_to_string(models.join("swarm").join("stamp"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "legacy".into())
}

fn remember_map_neighbors(state: &AppState) {
    let map = state.map.lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(map) = map.as_ref() else {
        return;
    };
    let regional: Vec<String> = map
        .places
        .iter()
        .filter(|place| state.region.as_deref() == Some(place.place.region.as_str()))
        .flat_map(|place| place.hives.iter().map(|door| door.base.clone()))
        .collect();
    let shard_peers: Vec<String> = map
        .places
        .iter()
        .flat_map(|place| place.shards.iter())
        .flat_map(|shard| shard.holders.iter().cloned())
        .filter(|holder| holder.starts_with("http"))
        .collect();
    let distant: Vec<String> = map
        .places
        .iter()
        .filter(|place| state.region.as_deref() != Some(place.place.region.as_str()))
        .flat_map(|place| place.hives.iter().map(|door| door.base.clone()))
        .collect();
    let near = mds_dht::neighbor_set(&[], &regional, &shard_peers, &[]);
    let far = mds_dht::neighbor_set(&[], &[], &[], &distant);
    let mut swarm = state.swarm.lock().unwrap_or_else(|poison| poison.into_inner());
    for base in near {
        swarm.upsert(peer_card(base, true));
    }
    for base in far {
        swarm.upsert(peer_card(base, false));
    }
}

fn peer_card(base: String, reachable: bool) -> mds_dht::PeerCard {
    mds_dht::PeerCard {
        base: base.clone(),
        label: base,
        buckets: Vec::new(),
        holds_all: false,
        latency_ms: 0,
        alive: true,
        reachable,
        pages: 0,
        queue: 0,
    }
}

async fn region_button(
    state: &AppState,
    query: &str,
    tab: &str,
    theme: &str,
    lat: Option<f64>,
    lon: Option<f64>,
    radius: Option<f64>,
) -> Option<(String, String)> {
    let named = mds_types::region_in(query)?;
    if state.region.as_deref() == Some(named) {
        return None;
    }
    let doors = {
        let map = state.map.lock().unwrap_or_else(|poison| poison.into_inner());
        map.as_ref()?.hives_for(named).into_iter().map(str::to_string).collect::<Vec<_>>()
    };
    if doors.is_empty() {
        return None;
    }
    let probe = reqwest::Client::builder()
        .timeout(Duration::from_millis(400))
        .build()
        .ok()?;
    for base in doors {
        let status = format!("{}/peer/status", base.trim_end_matches('/'));
        if probe.get(status).send().await.ok()?.status().is_success() {
            let mut href = format!(
                "{}/search?q={}&page=1",
                base.trim_end_matches('/'),
                q_encode(query),
            );
            if !tab.is_empty() {
                href.push_str("&tab=");
                href.push_str(&q_encode(tab));
            }
            if !theme.is_empty() {
                href.push_str("&theme=");
                href.push_str(&q_encode(theme));
            }
            href.push_str("&region=");
            href.push_str(&q_encode(named));
            if let (Some(lat), Some(lon), Some(radius)) = (lat, lon, radius) {
                href.push_str(&format!("&lat={lat}&lon={lon}&radius={radius}"));
            }
            return Some((named.to_string(), href));
        }
    }
    None
}

fn q_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(*byte as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[derive(Deserialize)]
struct SignedPacket {
    envelope: mds_wire::WireEnvelope,
    body: String,
}

async fn peer_search_v1(State(state): State<AppState>, Json(packet): Json<SignedPacket>) -> Result<Json<mds_wire::SearchAnswerV1>, StatusCode> {
    let map = state.map.lock().unwrap_or_else(|poison| poison.into_inner()).clone();
    let Some(map) = map else {
        return Err(StatusCode::NOT_FOUND);
    };
    if !state
        .nonces
        .accept(&packet.envelope, packet.body.as_bytes(), &map, &state.model, 1)
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let query: mds_wire::SearchQueryV1 = serde_json::from_str(&packet.body).map_err(|_| StatusCode::BAD_REQUEST)?;
    if query.hop_limit == 0 || query_lane(&query.text) == Lane::Hidden {
        return Ok(Json(mds_wire::SearchAnswerV1 { hits: Vec::new(), compatible: true }));
    }
    if !mds_wire::search_compatible(&state.model, &packet.envelope.model_generation)
        || query.vector.len() != mds_types::EMBED_DIMS
    {
        return Ok(Json(mds_wire::SearchAnswerV1 { hits: Vec::new(), compatible: false }));
    }
    let readers = state.readers.clone();
    let lsh = state.lsh.clone();
    let req = SearchRequest {
        text: query.text.clone(),
        vector: mds_types::QuantVector {
            scale: query.scale,
            values: query.vector.clone(),
        },
    };
    let hits = tokio::task::spawn_blocking(move || {
        let vector = req.vector.dequantize();
        readers.call(|index| hits_for(index, &lsh, &req.text, &vector, 0, 0))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let hits = hits.into_iter().take(query.top_k as usize).map(SearchHit::from).collect();
    Ok(Json(mds_wire::SearchAnswerV1 { hits, compatible: true }))
}

#[derive(Deserialize)]
struct DeltaQuery {
    from: Option<u64>,
}

async fn peer_shard_manifest(State(state): State<AppState>) -> Json<mds_wire::ShardManifestV1> {
    let engine = state.engine.clone();
    let manifest = tokio::task::spawn_blocking(move || {
        engine
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .shard_manifest()
    })
    .await
    .unwrap_or_else(|_| mds_wire::ShardManifestV1 {
        shard: String::new(),
        generation: 1,
        revision: 0,
        pages: 0,
        root_sha256: String::new(),
    });
    Json(manifest)
}

async fn peer_shard_delta(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DeltaQuery>,
) -> Json<mds_wire::ShardBlockV1> {
    let from = query.from.unwrap_or(0);
    let caller = headers
        .get("x-mds-from")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .trim()
        .trim_end_matches('/')
        .to_string();
    let engine = state.engine.clone();
    let block = tokio::task::spawn_blocking(move || {
        let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        if !guard.shares_shards() || !guard.peer_allowed(&caller) {
            return guard.export_shard(from, 0).unwrap_or_else(|_| mds_wire::ShardBlockV1 {
                from_revision: from,
                to_revision: from,
                pages: Vec::new(),
                block_sha256: crate::engine::shard_hash(&[]),
            });
        }
        guard.export_shard(from, 32).unwrap_or_else(|_| mds_wire::ShardBlockV1 {
            from_revision: from,
            to_revision: from,
            pages: Vec::new(),
            block_sha256: crate::engine::shard_hash(&[]),
        })
    })
    .await
    .unwrap_or(mds_wire::ShardBlockV1 {
        from_revision: from,
        to_revision: from,
        pages: Vec::new(),
        block_sha256: String::new(),
    });
    Json(block)
}

async fn shard_sync_loop(
    engine: Arc<Mutex<Engine>>,
    client: reqwest::Client,
    peers: Vec<String>,
    self_base: String,
) {
    let mut cursor: HashMap<String, u64> = HashMap::new();
    loop {
        tokio::time::sleep(Duration::from_secs(120)).await;
        for peer in &peers {
            let from = cursor.get(peer).copied().unwrap_or(0);
            let url = format!("{}/peer/shard/delta?from={from}", peer.trim_end_matches('/'));
            let Ok(response) = client
                .get(url)
                .header("x-mds-from", &self_base)
                .send()
                .await
            else {
                continue;
            };
            if !response.status().is_success() {
                continue;
            }
            let Ok(bytes) = response.bytes().await else {
                continue;
            };
            let Ok(block) = serde_json::from_slice::<mds_wire::ShardBlockV1>(&bytes) else {
                continue;
            };
            if crate::engine::shard_hash(&block.pages) != block.block_sha256 {
                eprintln!("MDS: снимок соседа не сошёлся, пакет не принят");
                continue;
            }
            let next_revision = block.to_revision;
            let eng = engine.clone();
            let accepted = tokio::task::spawn_blocking(move || {
                let mut guard = eng.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.accept_shard_block(&block).unwrap_or(0)
            })
            .await
            .unwrap_or(0);
            if next_revision > from {
                cursor.insert(peer.clone(), next_revision);
            }
            if accepted > 0 {
                eprintln!("MDS: из снимка соседа принято страниц: {accepted}");
            }
        }
    }
}

async fn peer_shard_v1(State(state): State<AppState>, Json(packet): Json<SignedPacket>) -> StatusCode {
    let map = state.map.lock().unwrap_or_else(|poison| poison.into_inner()).clone();
    let Some(map) = map else {
        return StatusCode::NOT_FOUND;
    };
    if packet.envelope.kind != "shard_delta"
        || !state
            .nonces
            .accept(&packet.envelope, packet.body.as_bytes(), &map, &state.model, 1)
    {
        return StatusCode::NOT_FOUND;
    }
    let Some(packed) = crate::gate::pack_block(packet.body.as_bytes(), 1024 * 1024) else {
        return StatusCode::BAD_REQUEST;
    };
    let Some(plain) = crate::gate::unpack_block(&packed, 1024 * 1024) else {
        return StatusCode::BAD_REQUEST;
    };
    let Ok(delta) = serde_json::from_slice::<mds_wire::ShardDeltaV1>(&plain) else {
        return StatusCode::BAD_REQUEST;
    };
    let mut book = state.shards.lock().unwrap_or_else(|poison| poison.into_inner());
    if !book.known(&packet.envelope.node_id) {
        book.note(mds_dht::ShardCopy {
            node_id: packet.envelope.node_id.clone(),
            generation: delta.generation,
            revision: delta.from_revision,
            root_sha256: String::new(),
            state: mds_dht::CopyState::CatchingUp,
        });
    }
    let actual = mds_update::sha256_hex(&plain);
    if book.apply_delta(
        &packet.envelope.node_id,
        delta.generation,
        delta.from_revision,
        delta.to_revision,
        &delta.block_sha256,
        &actual,
    ) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CONFLICT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_journal_is_not_served_through_a_proxy() {
        let local: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        assert!(operator_on_this_machine(local, &HeaderMap::new()));
        let mut forwarded = HeaderMap::new();
        forwarded.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.8"));
        assert!(!operator_on_this_machine(local, &forwarded));
        let mut real = HeaderMap::new();
        real.insert("x-real-ip", HeaderValue::from_static("203.0.113.8"));
        assert!(!operator_on_this_machine(local, &real));
        let remote: SocketAddr = "203.0.113.8:443".parse().unwrap();
        assert!(!operator_on_this_machine(remote, &HeaderMap::new()));
    }

    #[test]
    fn empty_coordinates_are_not_a_broken_search() {
        assert_eq!(parse_blank_f64(Some("")), None);
        assert_eq!(parse_blank_f64(Some("  ")), None);
        assert_eq!(parse_blank_f64(None), None);
        assert_eq!(parse_blank_f64(Some("55.75")), Some(55.75));
        assert_eq!(parse_blank_f64(Some("тут")), None);
    }
}

