use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use mds_dht::Swarm;
use mds_types::Hit;
use mds_update::ReleaseManifest;
use mds_rank::{page_lane, query_lane, Lane};
use mds_web::{
    page_download, page_for_sites, page_help, page_how, page_llms, page_manifesto, page_operator, render,
    render_results, OperatorReport, PeerLine,
};
use mds_wire::{CrawlWork, PeerHello, PeerStatus, PushedPage, SearchHit, SearchRequest, SearchResponse};
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::updates::watch;
use crate::{Engine, NodeError};

#[derive(Clone)]
struct AppState {
    engine: Arc<Mutex<Engine>>,
    releases: PathBuf,
    peers: Vec<String>,
    client: reqwest::Client,
    swarm: Arc<Mutex<Swarm>>,
    swarm_key: String,
    volunteers: Arc<Mutex<HashSet<String>>>,
}

pub struct ServeConfig {
    pub listen: String,
    pub peers: Vec<String>,
    pub releases: PathBuf,
    pub background_policy: Option<mds_crawl::CrawlPolicy>,
    pub discover_ct: bool,
    pub hive: Option<String>,
    pub swarm_key: String,
    pub label: String,
}

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
    page: Option<usize>,
    adult: Option<String>,
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
    let state = AppState {
        engine,
        releases: config.releases,
        peers: config.peers,
        client,
        swarm,
        swarm_key: config.swarm_key,
        volunteers,
    };
    let app = Router::new()
        .route("/", get(home))
        .route("/search", get(search))
        .route("/manifesto", get(manifesto_page))
        .route("/how", get(how_page))
        .route("/help", get(help_page))
        .route("/for-sites", get(for_sites_page))
        .route("/llms", get(llms_page))
        .route("/download", get(download_page))
        .route("/download/mds.exe", get(download_bin))
        .route("/download/pchela.zip", get(download_zip))
        .route("/operator", get(operator))
        .route("/peer/search", post(peer_search))
        .route("/peer/status", get(peer_status))
        .route("/peer/join", post(peer_join))
        .route("/peer/pages", post(peer_pages))
        .route("/peer/work", get(peer_work))
        .route("/peer/update", get(peer_update))
        .route("/peer/update.bin", get(peer_blob))
        .with_state(state);
    let listener = TcpListener::bind(&config.listen).await?;
    eprintln!("MDS: поиск на http://{}/", config.listen);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

async fn peer_search(
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, StatusCode> {
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

async fn peer_blob(State(state): State<AppState>) -> Result<Vec<u8>, StatusCode> {
    match mds_update::load_newest(&state.releases) {
        Ok(Some((_, bytes))) => Ok(bytes),
        _ => Err(StatusCode::NOT_FOUND),
    }
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
        }
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(report))
}

async fn peer_join(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(hello): Json<PeerHello>,
) -> StatusCode {
    if !swarm_allowed(&state.swarm_key, &headers, peer) {
        return StatusCode::NOT_FOUND;
    }
    let reachable = hello.base.starts_with("http://") || hello.base.starts_with("https://");
    state.swarm.lock().unwrap_or_else(|poison| poison.into_inner()).upsert(mds_dht::PeerCard {
        base: hello.base,
        label: hello.label,
        buckets: hello.buckets,
        holds_all: hello.holds_all,
        latency_ms: 0,
        alive: true,
        reachable,
        pages: hello.pages,
        queue: hello.queue,
    });
    if let Some(host) = crate::lite::site_host(&hello.site) {
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
            .insert(host);
    }
    StatusCode::NO_CONTENT
}

async fn peer_pages(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(pages): Json<Vec<PushedPage>>,
) -> Result<StatusCode, StatusCode> {
    if !swarm_allowed(&state.swarm_key, &headers, peer) {
        return Err(StatusCode::NOT_FOUND);
    }
    let batch: Vec<(String, String, String, String)> = pages
        .into_iter()
        .take(8)
        .map(|page| (page.url, page.title, page.host, page.text))
        .collect();
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<CrawlWork>, StatusCode> {
    if !swarm_allowed(&state.swarm_key, &headers, peer) {
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
    Ok(Json(CrawlWork { urls }))
}

fn swarm_allowed(key: &str, headers: &HeaderMap, peer: SocketAddr) -> bool {
    if key.is_empty() {
        return peer.ip().is_loopback();
    }
    let got = headers
        .get("x-mds-swarm")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    token_eq(key, got)
}

fn token_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut diff = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(a ^ b);
    }
    diff == 0
}

async fn home() -> Html<String> {
    Html(render("", &[], 1, None))
}

async fn manifesto_page() -> Html<String> {
    Html(page_manifesto())
}

async fn how_page() -> Html<String> {
    Html(page_how())
}

async fn help_page() -> Html<String> {
    Html(page_help())
}

async fn for_sites_page() -> Html<String> {
    Html(page_for_sites())
}

async fn llms_page() -> Html<String> {
    Html(page_llms())
}

async fn download_page(State(state): State<AppState>) -> Html<String> {
    let version = mds_update::load_newest(&state.releases)
        .ok()
        .and_then(|found| found.map(|(manifest, _)| manifest.version));
    Html(page_download(version.as_deref()))
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
        "Пчела для Windows.\r\nРядом лежат mds.exe, onnxruntime.dll и urls.txt.\r\nЗапуск публичного улья:\r\nmds.exe node --listen 0.0.0.0:8080 --seeds urls.txt --follow-external --swarm-key секрет\r\nДомашняя пчела за роутером, без белого IP:\r\nmds.exe node --seeds urls.txt --follow-external --hive http://адрес-улья:8080 --swarm-key секрет\r\n"
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
    if query.is_empty() {
        return Html(render("", &[], 1, None)).into_response();
    }
    if query_lane(&query) == Lane::Hidden {
        return Html(render("", &[], 1, Some("Такой запрос эта нода не ищет."))).into_response();
    }
    let adult_open = params.adult.as_deref() == Some("1") || cookie_is(&headers, "mds_adult", "1");
    let engine = state.engine.clone();
    let query_for_search = query.clone();
    let local = tokio::task::spawn_blocking(move || {
        let mut guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
        let started = std::time::Instant::now();
        let (hits, vector) = guard.search_with_vector(&query_for_search)?;
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
            let logged = open.iter().cloned().chain(adult.iter().cloned()).collect::<Vec<_>>();
            let engine = state.engine.clone();
            let query_for_log = query.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let guard = engine.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.remember_query(&query_for_log, &logged)
            })
            .await;
            Html(render_results(&query, &open, &adult, adult_open, page, None)).into_response()
        }
        Ok(Err(err)) => Html(render(&query, &[], page, Some(&err.to_string()))).into_response(),
        Err(_) => Html(render(&query, &[], page, Some("поиск прерван"))).into_response(),
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

async fn operator(State(state): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>) -> Response {
    if !peer.ip().is_loopback() {
        return StatusCode::NOT_FOUND.into_response();
    }
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
            Html(page_operator(&OperatorReport {
                pages,
                queue,
                db_bytes,
                searches_per_minute,
                avg_ms,
                peers,
                rows: view,
            }))
            .into_response()
        }
        _ => Html(render("", &[], 1, Some("журнал не открылся"))).into_response(),
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
        let mut visited: Vec<String> = pages.iter().map(|p| p.url.clone()).collect();
        visited.extend(batch);
        let mut discovered = Vec::new();
        for page in &pages {
            discovered.extend(page.links.iter().cloned());
        }
        let eng = engine.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let mut guard = eng.lock().unwrap_or_else(|p| p.into_inner());
            let _ = guard.record_crawl_step(&visited, &discovered, pages);
        })
        .await;
    }
}

