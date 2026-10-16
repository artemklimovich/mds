use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{ConnectInfo, Form, Path, Query, Request, State};
use axum::middleware::Next;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use mds_dht::Swarm;
use mds_types::{bare_host, canonical_region, host_of_url, in_general, tab_mask, Hit};
use mds_update::{read_model_manifest, ReleaseManifest};
use mds_rank::{page_lane, prefer_region, query_lane, Lane};
use mds_web::{
    hive_card, offer_region, page_add, page_download, page_for_sites, page_help, page_how, page_indexing, page_llms, page_manifesto, page_operator,
    AddLine,
    render, render_results, robots_txt, sitemap_xml, stamp_census, OperatorReport, PeerLine,
};
use mds_wire::{CrawlWork, PeerHello, PeerStatus, PushedPage, SearchHit, SearchRequest, SearchResponse};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio_util::io::ReaderStream;

use crate::updates::{announce, watch};
use crate::{Engine, NodeError};

#[derive(Clone)]
struct AppState {
    engine: Arc<Mutex<Engine>>,
    releases: PathBuf,
    models: PathBuf,
    downloads: PathBuf,
    peers: Vec<String>,
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
}

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
    page: Option<usize>,
    adult: Option<String>,
    region: Option<String>,
    tab: Option<String>,
}

pub async fn serve(engine: Engine, config: ServeConfig) -> Result<(), NodeError> {
    let releases = config.releases.clone();
    let peers = config.peers.clone();
    tokio::spawn(watch(peers, releases));
    let engine = Arc::new(Mutex::new(engine));
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
            alive: true,
            reachable: true,
            pages: 0,
            queue: 0,
        });
    }
    let swarm = Arc::new(Mutex::new(swarm));
    if !config.peers.is_empty() {
        tokio::spawn(poll_peers(config.peers.clone(), swarm.clone(), client.clone()));
    }
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
        releases: config.releases,
        models: config.models,
        downloads: config.downloads,
        peers: config.peers,
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
    };
    if !config.alone {
        remember_map_neighbors(&state);
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
        .route("/manifesto", get(manifesto_page))
        .route("/how", get(how_page))
        .route("/help", get(help_page))
        .route("/for-sites", get(for_sites_page))
        .route("/indexing", get(indexing_page))
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
        .route("/peer/status", get(peer_status))
        .route("/peer/join", post(peer_join))
        .route("/peer/pages", post(peer_pages))
        .route("/peer/work", get(peer_work))
        .route("/peer/update", get(peer_update))
        .route("/peer/update.bin", get(peer_blob))
        .route("/peer/model", get(peer_model))
        .route("/peer/model/{name}", get(peer_model_file))
        .route("/peer/notice", get(peer_notice).post(peer_notice_from))
        .route("/wire", get(wire))
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

fn person_page(path: &str) -> bool {
    matches!(
        path,
        "/" | "/search" | "/manifesto" | "/how" | "/help" | "/for-sites" | "/indexing" | "/add" | "/llms" | "/download"
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
    if !person_page(&path) {
        return response;
    }
    let (token, fresh) = visit_token(cookie.as_deref());
    let mark = response.extensions().get::<crate::visits::SearchMark>().copied();
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
    if fresh {
        if let Ok(value) = HeaderValue::from_str(&format!(
            "mds_visit={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=1800"
        )) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

async fn wire() -> Json<Vec<crate::wire_feed::WireLine>> {
    Json(crate::wire_feed::headlines().await)
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
    let engine = state.engine.clone();
    let hits = tokio::task::spawn_blocking(move || {
        let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.search_wire(&req)
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
    if state.bees.lock().unwrap_or_else(|poison| poison.into_inner()).host_of(&bee).is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let batch: Vec<(String, String, String, String)> = pages
        .into_iter()
        .take(8)
        .filter(|page| {
            state
                .bees
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .accepts(&bee, &page.url)
        })
        .filter_map(|page| {
            let host = host_of_url(&page.url)?;
            Some((page.url, page.title, bare_host(&host), page.text))
        })
        .collect();
    if batch.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }
    let engine = state.engine.clone();
    tokio::task::spawn_blocking(move || {
        let mut guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.accept_remote_pages(batch)
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
    let urls = tokio::task::spawn_blocking(move || {
        let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.pending_urls(4)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .bees
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .assign(&bee, urls.clone());
    Ok(Json(CrawlWork { urls }))
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
    request: axum::extract::Request,
) -> Response {
    let headers = request.headers().clone();
    let Some(csrf) = cabinet_csrf(&state, &headers) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), 2_000).await else {
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
                "<!doctype html><meta charset=\"utf-8\"><title>Ключ MCP</title><body><main><p>Ключ показан один раз. Адрес: https://admin.&lt;домен этой ноды&gt;/mcp, порт 443.</p><pre>{}</pre><p><a href=\"/operator\">Назад</a></p></main></body>",
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
        return Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(header::LOCATION, "/operator")
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::NO_CONTENT.into_response());
    }
    let host = bare_host(&form_field(&text, "host"));
    let action = form_field(&text, "action");
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
        .header(header::LOCATION, "/operator")
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::NO_CONTENT.into_response())
}

fn form_field(body: &str, key: &str) -> String {
    body.split('&')
        .find_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            if name == key {
                Some(value.replace('+', " ").trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

#[cfg(test)]
fn operator_on_this_machine(peer: SocketAddr, headers: &HeaderMap) -> bool {
    if !peer.ip().is_loopback() {
        return false;
    }
    let proxied = headers.contains_key("x-forwarded-for")
        || headers.contains_key("x-real-ip")
        || headers.contains_key("forwarded");
    !proxied
}

fn painted(state: &AppState, html: String) -> Html<String> {
    let (sites, pages) = state
        .engine
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .census()
        .unwrap_or((0, 0));
    let notice = mds_update::load_notice(&state.releases);
    Html(mds_web::stamp_notice(stamp_census(html, sites, pages), notice.as_deref()))
}

async fn home(State(state): State<AppState>) -> Html<String> {
    painted(&state, render_results("", &[], &[], false, 1, None, state.region.as_deref(), ""))
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

async fn sitemap(headers: HeaderMap) -> Response {
    text_file(sitemap_xml(&public_origin(&headers)), "application/xml; charset=utf-8")
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
        "Пчела для Windows.\r\nРядом лежат mds.exe, onnxruntime.dll и urls.txt.\r\nПолная нода:\r\nmds.exe node --listen 0.0.0.0:8080 --seeds urls.txt --follow-external --peers https://45.158.46.52\r\nЛёгкая пчела рядом с сайтом:\r\nmds.exe node --lite --site https://ваш-сайт --hive https://45.158.46.52\r\nИмя из журнала положите одной строкой в https://ваш-сайт/.well-known/pchela.txt\r\n"
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

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
) -> Response {
    let query = params.q.unwrap_or_default();
    let query = query.trim().to_string();
    let page = params.page.unwrap_or(1);
    let chosen = match params.region.as_deref() {
        None => state.region.clone(),
        Some("") => None,
        Some(value) => canonical_region(value).map(str::to_string).or_else(|| state.region.clone()),
    };
    if query.is_empty() {
        return painted(&state, render_results("", &[], &[], false, 1, None, chosen.as_deref(), "")).into_response();
    }
    if query_lane(&query) == Lane::Hidden {
        return stamp_search(
            painted(&state, render("", &[], 1, Some("Такой запрос эта нода не ищет."))).into_response(),
            crate::visits::SearchMark {
                empty: false,
                failed: false,
                refused: true,
            },
        );
    }
    let adult_open = params.adult.as_deref() == Some("1") || cookie_is(&headers, "mds_adult", "1");
    let tab = params.tab.clone().unwrap_or_default();
    let kind_mask = tab_mask(tab.trim());
    let engine = state.engine.clone();
    let query_for_search = query.clone();
    let local = tokio::task::spawn_blocking(move || {
        let mut guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        let started = std::time::Instant::now();
        let (hits, vector) = guard.search_with_vector(&query_for_search, kind_mask)?;
        let bucket = guard.semantic_bucket(&vector);
        guard.note_search(started.elapsed().as_millis());
        Ok::<_, crate::NodeError>((hits, vector, bucket))
    })
    .await;
    let response = match local {
        Ok(Ok((mut hits, vector, bucket))) => {
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
            let hosts = state
                .volunteers
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone();
            mark_volunteers(&mut hits, &hosts);
            let (open, adult) = split_lanes(hits);
            let mut open = open;
            if kind_mask == 0 {
                open.retain(|hit| in_general(hit.kind));
            } else {
                open.retain(|hit| hit.kind & kind_mask != 0);
            }
            let open = prefer_region(open, chosen.as_deref());
            let logged = open.iter().cloned().chain(adult.iter().cloned()).collect::<Vec<_>>();
            let engine = state.engine.clone();
            let query_for_log = query.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.remember_query(&query_for_log, &logged)
            })
            .await;
            let mut html = render_results(&query, &open, &adult, adult_open, page, None, chosen.as_deref(), &tab);
            if let Some((label, href)) = region_button(&state, &query, &tab).await {
                html = offer_region(html, &label, &href);
            }
            stamp_search(
                painted(&state, html).into_response(),
                crate::visits::SearchMark {
                    empty: open.is_empty() && adult.is_empty(),
                    failed: false,
                    refused: false,
                },
            )
        }
        Ok(Err(err)) => stamp_search(
            painted(&state, render(&query, &[], page, Some(&err.to_string()))).into_response(),
            crate::visits::SearchMark {
                empty: false,
                failed: true,
                refused: false,
            },
        ),
        Err(_) => stamp_search(
            painted(&state, render(&query, &[], page, Some("поиск прерван"))).into_response(),
            crate::visits::SearchMark {
                empty: false,
                failed: true,
                refused: false,
            },
        ),
    };
    if params.adult.as_deref() == Some("1") {
        let mut response = response;
        if let Ok(value) = HeaderValue::from_str("mds_adult=1; Path=/; Max-Age=2592000; SameSite=Lax; HttpOnly") {
            response.headers_mut().insert(header::SET_COOKIE, value);
        }
        response
    } else {
        response
    }
}

async fn operator(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let Some(csrf) = cabinet_csrf(&state, &headers) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let engine = state.engine.clone();
    let rows = tokio::task::spawn_blocking(move || {
        let mut guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        let (searches, avg_ms) = guard.load_snapshot();
        let report_head = (
            guard.len().unwrap_or(0) as u64,
            guard.queue_len().unwrap_or(0) as u64,
            guard.db_bytes(),
            searches,
            avg_ms,
        );
        guard.recent_queries(50).map(|rows| (report_head, rows))
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
    match rows {
        Ok(Ok(((pages, queue, db_bytes, searches_per_minute, avg_ms), rows))) => {
            let view: Vec<(String, i64, i64, i64, i64)> = rows
                .into_iter()
                .map(|row| (row.text, row.results, row.words, row.meaning, row.adult))
                .collect();
            painted(
                &state,
                page_operator(&OperatorReport {
                    pages,
                    queue,
                    db_bytes,
                    searches_per_minute,
                    avg_ms,
                    peers,
                    volunteers: state
                        .volunteers
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .iter()
                        .cloned()
                        .collect(),
                    refused: state.bees.lock().unwrap_or_else(|poison| poison.into_inner()).refused(),
                    rows: view,
                    csrf: csrf.clone(),
                }),
            )
            .0
            .replacen("</main>", &format!("{}</main>", cabinet_tail(&state, &csrf)), 1)
            .into_response()
        }
        _ => painted(&state, render("", &[], 1, Some("журнал не открылся"))).into_response(),
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
    while let Some(joined) = set.join_next().await {
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
                kind: hit.kind,
                media_url: hit.media_url,
                feed_url: hit.feed_url,
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
                } else if existing.region.is_none() {
                    existing.region = hit.region.clone();
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

async fn poll_peers(peers: Vec<String>, swarm: Arc<Mutex<Swarm>>, client: reqwest::Client) {
    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;
        for peer in &peers {
            let base = peer.trim().trim_end_matches('/');
            let started = std::time::Instant::now();
            let response = client.get(format!("{base}/peer/status")).send().await;
            let elapsed = started.elapsed().as_millis() as u64;
            let Ok(response) = response else {
                let mut guard = swarm.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.note_latency(base, elapsed, false);
                continue;
            };
            if !response.status().is_success() {
                let mut guard = swarm.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.note_latency(base, elapsed, false);
                continue;
            }
            let Ok(bytes) = response.bytes().await else {
                continue;
            };
            let Ok(status) = serde_json::from_slice::<PeerStatus>(&bytes) else {
                continue;
            };
            let mut guard = swarm.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.upsert(mds_dht::PeerCard {
                base: base.to_string(),
                label: base.to_string(),
                buckets: status.buckets,
                holds_all: status.holds_all,
                latency_ms: elapsed,
                alive: true,
                reachable: true,
                pages: status.pages.min(u64::from(u32::MAX)) as u32,
                queue: status.queue.min(u64::from(u32::MAX)) as u32,
            });
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

    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let batch = {
            let eng = engine.clone();
            tokio::task::spawn_blocking(move || {
                let guard = eng.lock().unwrap_or_else(|p| p.into_inner());
                guard.pending_urls(4).unwrap_or_default()
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
            Err(_) => continue,
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
    Router::new()
        .route("/login", get(login_page).post(login_act))
        .route("/logout", post(logout_act))
        .route("/operator", get(operator).post(operator_act))
        .route("/mcp", post(mcp_act))
        .with_state(state)
}

async fn login_page() -> Html<String> {
    Html(crate::admin::login_page(false))
}

#[derive(Deserialize)]
struct LoginForm {
    user: String,
    password: String,
}

async fn login_act(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Form(form): Form<LoginForm>,
) -> Response {
    let Some(door) = &state.door else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !door.allow(&format!("login:{}", peer.ip()), 8, 60) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let Some((token, _)) = door.login(&form.user, &form.password) else {
        return Html(crate::admin::login_page(true)).into_response();
    };
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/operator")
        .header(
            header::SET_COOKIE,
            format!("mds_session={token}; HttpOnly; SameSite=Strict; Path=/; Secure; Max-Age=43200"),
        )
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::NO_CONTENT.into_response())
}

async fn logout_act(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let (Some(door), Some(token)) = (&state.door, crate::admin::cookie_token(&headers)) {
        door.logout(&token);
    }
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/login")
        .header(
            header::SET_COOKIE,
            "mds_session=; HttpOnly; SameSite=Strict; Path=/; Secure; Max-Age=0",
        )
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

fn cabinet_csrf(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let door = state.door.as_ref()?;
    let token = crate::admin::cookie_token(headers)?;
    door.session(&token)
}

fn cabinet_tail(state: &AppState, csrf: &str) -> String {
    let guard = state.engine.lock().unwrap_or_else(|poison| poison.into_inner());
    let day = guard.query_counts("day").unwrap_or_default();
    let week = guard.query_counts("week").unwrap_or_default();
    let month = guard.query_counts("month").unwrap_or_default();
    let visits_day = guard.visit_span("day").unwrap_or_default();
    let visits_week = guard.visit_span("week").unwrap_or_default();
    let visits_month = guard.visit_span("month").unwrap_or_default();
    let pages = guard.list_index(None, None, 0).unwrap_or_default();
    drop(guard);
    let map = state.map.lock().unwrap_or_else(|poison| poison.into_inner());
    let mut body = format!(
        "<h2>Эта нода</h2><ul class=\"load\"><li>Версия: {}</li><li>Край: {}</li><li>Поколение модели: {}</li></ul>",
        mds_update::VERSION,
        state.region.as_deref().unwrap_or("не задан"),
        state.model
    );
    body.push_str("<h2>Карта сети</h2>");
    if let Some(map) = map.as_ref() {
        body.push_str(&format!("<p>Версия карты {}.</p><ul class=\"check\">", map.version));
        for place in &map.places {
            let door = place.hives.first().map(|door| door.base.as_str()).unwrap_or("нет входа");
            body.push_str(&format!(
                "<li>{} / {} — {}</li>",
                esc_text(&place.place.country),
                esc_text(&place.place.region),
                esc_text(door)
            ));
        }
        body.push_str("</ul>");
    } else {
        body.push_str("<p>Подписанной карты нет, поэтому кнопка другого края не показывается.</p>");
    }
    body.push_str("<h2>Заходы</h2><p>Это браузеры, не имена людей. Адрес не пишется и к запросу не привязывается. Время — от первого запроса визита до последнего; одна открытая страница времени не даёт. Отказ — ушёл, не поискав и не открыв вторую страницу.</p>");
    for (title, report) in [("За день", visits_day), ("За неделю", visits_week), ("За месяц", visits_month)] {
        body.push_str(&format!("<h3>{title}</h3>{}", visit_lines(&report)));
    }
    body.push_str("<h2>Запросы</h2><p>Одинаковый текст склеен. Число — сколько раз его спросили за этот срок. Буквы ё и е не склеиваются.</p>");
    for (title, rows) in [("За день", day), ("За неделю", week), ("За месяц", month)] {
        body.push_str(&format!("<h3>{title}</h3><ul class=\"check\">"));
        for (query, count) in rows {
            body.push_str(&format!("<li>{} — {}</li>", esc_text(&query), count));
        }
        body.push_str("</ul>");
    }
    body.push_str("<h2>Свой индекс</h2><ul class=\"check\">");
    for page in pages {
        body.push_str(&format!("<li>{}</li>", esc_text(&page.url)));
    }
    body.push_str("</ul>");
    body.push_str(&format!(
        "<h2>MCP</h2><p>Адрес https://admin.&lt;домен этой ноды&gt;/mcp, порт 443. Ключ отдельно от пароля кабинета.</p>
<form method=\"post\" action=\"/operator\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><button name=\"action\" value=\"issue_mcp\">Выдать ключ</button><button name=\"action\" value=\"revoke_mcp\">Отозвать</button></form>"
    ));
    body
}

fn visit_lines(report: &mds_index::VisitReport) -> String {
    let time = if report.timed_visits > 0 {
        format!("{} с", report.dwell_seconds / report.timed_visits)
    } else {
        "ещё не измерено".into()
    };
    let bounces = if report.visits > 0 {
        format!("{} ({}%)", report.bounces, report.bounces * 100 / report.visits)
    } else {
        "0".into()
    };
    format!(
        "<ul class=\"load\"><li>Заходы: {}</li><li>Просмотры: {}</li><li>Среднее время: {time}</li><li>Отказы: {bounces}</li><li>Поиски: {}</li><li>Пустая выдача: {}</li><li>Ошибки поиска: {}</li><li>Запросы, которые нода не ищет: {}</li></ul>",
        report.visits, report.views, report.searches, report.empty_searches, report.search_errors, report.refused
    )
}

fn esc_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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

async fn region_button(state: &AppState, query: &str, tab: &str) -> Option<(String, String)> {
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
            let href = format!(
                "{}/search?q={}&tab={}&region={}",
                base.trim_end_matches('/'),
                q_encode(query),
                q_encode(tab),
                q_encode(named)
            );
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
    let engine = state.engine.clone();
    let req = SearchRequest {
        text: query.text.clone(),
        vector: mds_types::QuantVector {
            scale: query.scale,
            values: query.vector.clone(),
        },
    };
    let hits = tokio::task::spawn_blocking(move || {
        let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.search_wire(&req)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let hits = hits.into_iter().take(query.top_k as usize).map(SearchHit::from).collect();
    Ok(Json(mds_wire::SearchAnswerV1 { hits, compatible: true }))
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
}

