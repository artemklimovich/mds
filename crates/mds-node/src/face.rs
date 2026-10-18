use std::path::PathBuf;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{OriginalUri, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use mds_types::Hit;
use mds_web::{
    cabinet_sheets, offer_region, page_add, page_android, page_computer, page_download, page_for_sites, page_help, page_hive, page_how, page_lost,
    page_indexing, page_llms, page_manifesto, page_operator, render, render_results, stamp_census, stamp_notice,
    CabinetSheets, OperatorReport,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::NodeError;

pub struct FaceConfig {
    pub listen: String,
    pub hive: String,
    pub admin: String,
    pub token_file: PathBuf,
}

#[derive(Clone)]
struct FaceState {
    hive: String,
    admin: String,
    token: String,
    client: reqwest::Client,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FaceMeta {
    pub sites: usize,
    pub pages: usize,
    pub notice: Option<String>,
    pub region: Option<String>,
    pub release_version: Option<String>,
    pub visit_cookie: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FaceSearch {
    pub query: String,
    pub open: Vec<Hit>,
    pub adult: Vec<Hit>,
    pub adult_open: bool,
    pub page: usize,
    pub error: Option<String>,
    pub region: Option<String>,
    pub tab: String,
    pub offer_label: Option<String>,
    pub offer_href: Option<String>,
    pub refused: bool,
    pub adult_cookie: bool,
    pub sites: usize,
    pub pages: usize,
    pub notice: Option<String>,
    pub visit_cookie: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FaceCabinet {
    pub login: bool,
    pub report: Option<OperatorReport>,
    pub sheets: Option<CabinetSheets>,
}

pub(crate) fn ensure_face_token(path: &std::path::Path) -> Result<String, NodeError> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let text = text.trim().to_string();
        if text.len() >= 32 {
            return Ok(text);
        }
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|err| NodeError::Message(err.to_string()))?;
    let text: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    std::fs::write(path, &text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(text)
}

pub fn token_matches(expected: &str, given: &str) -> bool {
    if expected.len() != given.len() || expected.len() < 32 {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in expected.bytes().zip(given.bytes()) {
        diff |= left ^ right;
    }
    diff == 0
}

pub async fn serve_face(config: FaceConfig) -> Result<(), NodeError> {
    let token = std::fs::read_to_string(&config.token_file)
        .map_err(|_| NodeError::Message("нет ключа лица: улей ещё не открылся".into()))?;
    let token = token.trim().to_string();
    if token.len() < 32 {
        return Err(NodeError::Message("ключ лица слишком короткий".into()));
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| NodeError::Message(err.to_string()))?;
    let state = FaceState {
        hive: config.hive.trim_end_matches('/').to_string(),
        admin: config.admin.trim_end_matches('/').to_string(),
        token,
        client,
    };
    let app = Router::new()
        .route("/", get(home_page))
        .route("/search", get(search_page))
        .route("/manifesto", get(manifesto_page))
        .route("/how", get(how_page))
        .route("/help", get(help_page))
        .route("/for-sites", get(for_sites_page))
        .route("/indexing", get(indexing_page))
        .route("/android", get(android_page))
        .route("/hive", get(hive_role_page))
        .route("/computer", get(computer_page))
        .route("/llms", get(llms_page))
        .route("/download", get(download_page))
        .route("/add", get(add_page).post(proxy_hive))
        .route("/admin", get(admin_entry))
        .route("/admin/", get(admin_entry))
        .route("/admin/login", get(login_page).post(proxy_admin))
        .route("/admin/operator", get(cabinet_page).post(proxy_admin))
        .route("/admin/logout", post(proxy_admin))
        .route("/admin/mcp", post(proxy_admin))
        .fallback(lost_page)
        .with_state(state);
    let listener = TcpListener::bind(&config.listen).await?;
    eprintln!("MDS: лицо на http://{}/", config.listen);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn lost_page() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(page_lost()))
        .unwrap_or_else(|_| StatusCode::NOT_FOUND.into_response())
}

fn stamp(html: String, meta: &FaceMeta) -> Html<String> {
    Html(stamp_notice(
        stamp_census(html, meta.sites, meta.pages),
        meta.notice.as_deref(),
    ))
}

fn mark_face(mut response: Response) -> Response {
    let _ = response
        .headers_mut()
        .insert("x-mds-node", header::HeaderValue::from_static("face"));
    response
}

fn with_visit(mut response: Response, cookie: &Option<String>) -> Response {
    if let Some(token) = cookie {
        if let Ok(value) = header::HeaderValue::from_str(&format!(
            "mds_visit={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=1800"
        )) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    mark_face(response)
}

async fn meta_for(state: &FaceState, headers: &HeaderMap, path: &str) -> Result<FaceMeta, StatusCode> {
    let response = state
        .client
        .get(format!("{}/internal/face/meta", state.hive))
        .header("x-mds-face", &state.token)
        .header(header::COOKIE, cookie_of(headers))
        .query(&[("path", path)])
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    if !response.status().is_success() {
        return Err(StatusCode::BAD_GATEWAY);
    }
    read_json(response).await
}

async fn read_json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, StatusCode> {
    let bytes = response.bytes().await.map_err(|_| StatusCode::BAD_GATEWAY)?;
    serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_GATEWAY)
}

fn cookie_of(headers: &HeaderMap) -> String {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string()
}

async fn home_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    let meta = meta_for(&state, &headers, "/").await?;
    let html = render_results("", &[], &[], false, 1, None, meta.region.as_deref(), "");
    Ok(with_visit(stamp(html, &meta).into_response(), &meta.visit_cookie))
}

async fn search_page(
    State(state): State<FaceState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let query = uri.query().unwrap_or("");
    let response = state
        .client
        .get(format!("{}/internal/face/search?{query}", state.hive))
        .header("x-mds-face", &state.token)
        .header(header::COOKIE, cookie_of(&headers))
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    if !response.status().is_success() {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let paint: FaceSearch = read_json(response).await?;
    let html = if paint.refused {
        render("", &[], 1, Some("Такой запрос эта нода не ищет."))
    } else if paint.query.is_empty() {
        render_results("", &[], &[], false, 1, None, paint.region.as_deref(), "")
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
        );
        if let (Some(label), Some(href)) = (&paint.offer_label, &paint.offer_href) {
            html = offer_region(html, label, href);
        }
        html
    };
    let meta = FaceMeta {
        sites: paint.sites,
        pages: paint.pages,
        notice: paint.notice.clone(),
        region: paint.region.clone(),
        release_version: None,
        visit_cookie: paint.visit_cookie.clone(),
    };
    let mut response = with_visit(stamp(html, &meta).into_response(), &paint.visit_cookie);
    if paint.adult_cookie {
        if let Ok(value) = header::HeaderValue::from_str("mds_adult=1; Path=/; Max-Age=2592000; SameSite=Lax; HttpOnly") {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    Ok(response)
}

async fn painted_page(
    state: &FaceState,
    headers: &HeaderMap,
    path: &str,
    body: String,
) -> Result<Response, StatusCode> {
    let meta = meta_for(state, headers, path).await?;
    Ok(with_visit(stamp(body, &meta).into_response(), &meta.visit_cookie))
}

async fn manifesto_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    painted_page(&state, &headers, "/manifesto", page_manifesto()).await
}

async fn how_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    painted_page(&state, &headers, "/how", page_how()).await
}

async fn help_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    painted_page(&state, &headers, "/help", page_help()).await
}

async fn for_sites_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    painted_page(&state, &headers, "/for-sites", page_for_sites()).await
}

async fn indexing_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    painted_page(&state, &headers, "/indexing", page_indexing()).await
}

async fn android_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    painted_page(&state, &headers, "/android", page_android()).await
}

async fn hive_role_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    painted_page(&state, &headers, "/hive", page_hive()).await
}

async fn computer_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    painted_page(&state, &headers, "/computer", page_computer()).await
}

async fn llms_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    painted_page(&state, &headers, "/llms", page_llms()).await
}

async fn download_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    let meta = meta_for(&state, &headers, "/download").await?;
    let html = page_download(meta.release_version.as_deref());
    Ok(with_visit(stamp(html, &meta).into_response(), &meta.visit_cookie))
}

async fn add_page(State(state): State<FaceState>, headers: HeaderMap) -> Result<Response, StatusCode> {
    painted_page(&state, &headers, "/add", page_add("", None, false)).await
}

async fn admin_entry() -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/admin/login")
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::NO_CONTENT.into_response())
}

async fn login_page() -> Html<String> {
    Html(crate::admin::login_page(false, "/admin"))
}

async fn cabinet_page(State(state): State<FaceState>, headers: HeaderMap, request: Request) -> Result<Response, StatusCode> {
    let response = state
        .client
        .get(format!("{}/internal/face/cabinet", state.hive))
        .header("x-mds-face", &state.token)
        .header(header::COOKIE, cookie_of(&headers))
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    if !response.status().is_success() {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let pack: FaceCabinet = read_json(response).await?;
    if pack.login {
        return Ok(Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(header::LOCATION, "/admin/login")
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::NO_CONTENT.into_response()));
    }
    let Some(mut report) = pack.report else {
        return Err(StatusCode::BAD_GATEWAY);
    };
    let Some(sheets) = pack.sheets else {
        return Err(StatusCode::BAD_GATEWAY);
    };
    let query = request.uri().query();
    let sheet = query
        .and_then(|query| {
            query.split('&').find_map(|part| {
                let (key, value) = part.split_once('=')?;
                if key == "sheet" { Some(value.to_string()) } else { None }
            })
        })
        .unwrap_or_else(|| "node".into());
    report.span = query
        .and_then(|query| {
            query.split('&').find_map(|part| {
                let (key, value) = part.split_once('=')?;
                if key == "span" { Some(value.to_string()) } else { None }
            })
        })
        .unwrap_or_default();
    let (stats, rest) = cabinet_sheets(&sheets, &sheet);
    let html = stamp_census(page_operator(&report, &stats, &rest, &sheet), report.sites as usize, report.pages as usize);
    Ok(mark_face(Html(html).into_response()))
}

async fn proxy_hive(State(state): State<FaceState>, request: Request) -> Result<Response, StatusCode> {
    proxy_to(&state, &state.hive, request).await
}

async fn proxy_admin(State(state): State<FaceState>, request: Request) -> Result<Response, StatusCode> {
    proxy_to(&state, &state.admin, request).await
}

async fn proxy_to(state: &FaceState, upstream: &str, request: Request) -> Result<Response, StatusCode> {
    let path = request.uri().path().to_string();
    if path.starts_with("/peer") || path.starts_with("/internal") || path.starts_with("/shell") {
        return Err(StatusCode::NOT_FOUND);
    }
    let query = request.uri().query().map(|value| format!("?{value}")).unwrap_or_default();
    let method = request.method().clone();
    let headers = request.headers().clone();
    let bytes = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let response = state
        .client
        .request(method, format!("{upstream}{path}{query}"))
        .header(header::COOKIE, cookie_of(&headers))
        .header(
            header::CONTENT_TYPE,
            headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("application/x-www-form-urlencoded"),
        )
        .body(bytes)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let status = StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    if let Some(location) = response.headers().get(header::LOCATION) {
        builder = builder.header(header::LOCATION, location);
    }
    let set_cookies: Vec<_> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .cloned()
        .collect();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| header::HeaderValue::from_static("text/html; charset=utf-8"));
    builder = builder.header(header::CONTENT_TYPE, content_type);
    let body = response.bytes().await.map_err(|_| StatusCode::BAD_GATEWAY)?;
    let mut out = builder.body(Body::from(body)).map_err(|_| StatusCode::BAD_GATEWAY)?;
    for cookie in set_cookies {
        out.headers_mut().append(header::SET_COOKIE, cookie);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_token_rejects_a_different_value() {
        let token = "0123456789abcdef0123456789abcdef";
        assert!(token_matches(token, token));
        assert!(!token_matches(token, "0123456789abcdef0123456789abcdee"));
        assert!(!token_matches(token, "short"));
        assert!(!token_matches("short", "short"));
    }
}
