use crate::sessions::{clear_cookie, current_session, session_cookie, Session};
use crate::{error::ApiErr, AppState};
use axum::extract::{Path, Query, State};
use axum::http::header::SET_COOKIE;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use tokio::io::AsyncSeekExt;
use serde::Deserialize;
use serde_json::json;
use shared::{
    CreateDownloadRequest, LoginRequest, User,
};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/config", get(app_config))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/qualities", get(qualities))
        .route("/api/libraries", get(libraries))
        .route("/api/items", get(items))
        .route("/api/items/{id}", get(item_detail))
        .route("/api/series/{id}/episodes", get(episodes))
        .route("/api/image/{id}", get(image))
        .route("/api/downloads", post(create_download).get(list_downloads))
        .route("/api/downloads/{id}/{action}", post(download_action))
        .route("/api/downloads/{id}", delete(remove_download))
        .route("/api/downloads/{id}/file", get(download_file))
}

async fn health() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}

/// Which Jellyfin server the operator pinned in config.json (if any).
async fn app_config(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({ "server_url": st.config.server_url }))
}

async fn login(State(st): State<Arc<AppState>>, headers: HeaderMap, Json(body): Json<LoginRequest>) -> Result<Response, ApiErr> {
    // An operator-pinned server beats whatever the client typed.
    let url_raw = st
        .config
        .server_url
        .clone()
        .unwrap_or(body.server_url);
    let server_url = crate::jellyfin::normalize_server_url(&url_raw)
        .map_err(|e| ApiErr::bad(e.to_string()))?;
    let (creds, name) = st
        .jf
        .authenticate(&server_url, &body.username, &body.password)
        .await
        .map_err(|e| ApiErr::msg(axum::http::StatusCode::UNAUTHORIZED, e.to_string()))?;
    let session = Session {
        id: uuid::Uuid::new_v4().to_string(),
        user: User {
            id: creds.user_id.clone(),
            name,
            server_url,
        },
        token: creds.token.clone(),
    };
    st.sessions.insert(session.clone()).await;
    // Behind a TLS reverse proxy, mark the cookie Secure.
    let secure = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|p| p.eq_ignore_ascii_case("https"))
        .unwrap_or(false);
    let mut headers = HeaderMap::new();
    headers.insert(SET_COOKIE, session_cookie(&session.id, secure));
    Ok((headers, Json(session.user)).into_response())
}

async fn logout(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Result<Response, ApiErr> {
    let session = current_session(&st.sessions, &headers).await?;
    st.sessions.remove(&session.id).await;
    let mut out = HeaderMap::new();
    out.insert(SET_COOKIE, clear_cookie());
    Ok((out, Json(json!({"ok": true}))).into_response())
}

async fn me(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Result<Json<User>, ApiErr> {
    let session = current_session(&st.sessions, &headers).await?;
    Ok(Json(session.user))
}

async fn qualities() -> impl IntoResponse {
    Json(json!(shared::quality_presets()))
}

async fn libraries(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiErr> {
    let session = current_session(&st.sessions, &headers).await?;
    let creds = st.sessions.credentials(&session);
    let views = st.jf.user_views(&creds).await.map_err(|e| ApiErr::bad(format!("{e:#}")))?;
    Ok(Json(json!(views)))
}

#[derive(Deserialize)]
struct ItemsQuery {
    library: Option<String>,
    search: Option<String>,
    #[serde(rename = "types")]
    types: Option<String>,
}

async fn items(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ItemsQuery>,
) -> Result<Json<serde_json::Value>, ApiErr> {
    let session = current_session(&st.sessions, &headers).await?;
    let creds = st.sessions.credentials(&session);
    let default_types = if q.search.as_deref().is_some_and(|s| !s.is_empty()) {
        "Movie,Series,Episode"
    } else {
        "Movie,Series"
    };
    let list = st
        .jf
        .items(
            &creds,
            q.library.as_deref(),
            q.search.as_deref(),
            q.types.as_deref().unwrap_or(default_types),
            200,
        )
        .await
        .map_err(|e| ApiErr::bad(format!("{e:#}")))?;
    Ok(Json(json!(list)))
}

async fn item_detail(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiErr> {
    let session = current_session(&st.sessions, &headers).await?;
    let creds = st.sessions.credentials(&session);
    let detail = st.jf.item_detail(&creds, &id).await.map_err(|e| ApiErr::bad(format!("{e:#}")))?;
    Ok(Json(json!(detail)))
}

async fn episodes(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiErr> {
    let session = current_session(&st.sessions, &headers).await?;
    let creds = st.sessions.credentials(&session);
    let list = st.jf.episodes(&creds, &id).await.map_err(|e| ApiErr::bad(format!("{e:#}")))?;
    Ok(Json(json!(list)))
}

async fn create_download(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateDownloadRequest>,
) -> Result<Json<serde_json::Value>, ApiErr> {
    let session = current_session(&st.sessions, &headers).await?;
    let creds = st.sessions.credentials(&session);
    let preset = shared::preset_by_id(&body.quality_id)
        .ok_or_else(|| ApiErr::bad("Unknown quality preset"))?;
    let detail = st
        .jf
        .item_detail(&creds, &body.item_id)
        .await
        .map_err(|e| ApiErr::bad(format!("{e:#}")))?;
    let outcome = st
        .jobs
        .create(detail, preset, creds, body.auto_save, Some(session.id))
        .map_err(|e| ApiErr::bad(format!("{e:#}")))?;
    let (job, existing) = match outcome {
        crate::jobs::CreateOutcome::Created(j) => (j, None),
        crate::jobs::CreateOutcome::Completed(j) => (j, Some("completed")),
        crate::jobs::CreateOutcome::Active(j) => (j, Some("active")),
    };
    Ok(Json(json!({ "job": job, "existing": existing })))
}

async fn list_downloads(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Json<serde_json::Value> {
    let session = current_session(&st.sessions, &headers).await.ok();
    let save_ready = session
        .map(|s| st.jobs.list_save_ready(&s.id))
        .unwrap_or_default();
    Json(json!({ "jobs": st.jobs.list(), "save_ready": save_ready }))
}

#[derive(Deserialize)]
struct ImageQuery {
    #[serde(rename = "type")]
    kind: Option<String>,
    /// Max edge (approx fillWidth for Jellyfin).
    w: Option<u32>,
}

/// Proxies item images from Jellyfin so any origin works (posters do not
/// need to be reachable by the user's browser).
async fn image(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<ImageQuery>,
) -> Result<Response, ApiErr> {
    let session = current_session(&st.sessions, &headers).await?;
    let creds = st.sessions.credentials(&session);
    let kind = q.kind.as_deref().unwrap_or("primary");
    let is_backdrop = kind == "backdrop";
    let width = q.w.unwrap_or(if is_backdrop { 1200 } else { 400 });
    let resp = st
        .jf
        .fetch_image(&creds, &id, kind, width)
        .await
        .map_err(|e| ApiErr::msg(axum::http::StatusCode::NOT_FOUND, format!("{e:#}")))?;

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "image/jpeg".to_string());

    if !resp.status().is_success() {
        // No artwork: pass the upstream status straight through.
        let status = axum::http::StatusCode::from_u16(resp.status().as_u16())
            .unwrap_or(axum::http::StatusCode::NOT_FOUND);
        return Ok((status, "not found").into_response());
    }

    let cache_control = if is_backdrop { "private, max-age=86400" } else { "private, max-age=86400" };
    let body = axum::body::Body::from_stream(resp.bytes_stream());
    Ok(([("content-type", content_type), ("cache-control", cache_control.to_string())], body).into_response())
}

/// Streams a finished download to the user's browser ("save to device").
/// Supports `Range` (open-ended `bytes=N-`), so browser download
/// managers can natively pause/resume the transfer.
async fn download_file(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiErr> {
    current_session(&st.sessions, &headers).await?;
    let Some(entry) = st.jobs.get_entry(&id) else {
        return Err(ApiErr::not_found("Download not found"));
    };
    if entry.job.state != shared::JobState::Completed {
        return Err(ApiErr::msg(axum::http::StatusCode::CONFLICT, "Download is not finished"));
    }
    let path = entry.downloads_dir.join(&entry.final_name);
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|_| ApiErr::not_found("File is missing on the server"))?;
    let total = meta.len();
    if total == 0 {
        return Err(ApiErr::not_found("File is empty"));
    }

    // Parse `bytes=N-`, `bytes=N-M`, `bytes=-S`. Anything else serves the whole file.
    let range = headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_range(v, total));
    let (start, partial) = match range {
        Some((s, _)) => (s, true),
        None => (0, false),
    };
    if start >= total {
        return Err(ApiErr::msg(
            axum::http::StatusCode::RANGE_NOT_SATISFIABLE,
            "Requested range is beyond file size",
        ));
    }

    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| ApiErr::not_found("File is missing on the server"))?;
    let mut reader = tokio::io::BufReader::new(file);
    reader
        .seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|_| ApiErr::not_found("Could not seek in file"))?;
    let served = total - start;

    let filename = entry.final_name.clone();
    let disposition = format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        filename.replace('"', "_"),
        urlencode(&filename)
    );

    let mut builder = Response::builder()
        .header("content-type", "application/octet-stream")
        .header("accept-ranges", "bytes")
        .header("content-disposition", disposition)
        .header("cache-control", "private, no-store")
        .header("content-length", served.to_string());
    if partial {
        builder = builder
            .status(axum::http::StatusCode::PARTIAL_CONTENT)
            .header("content-range", format!("bytes {start}-{}/{total}", total - 1));
    }

    let stream = tokio_util::io::ReaderStream::with_capacity(reader, 256 * 1024);
    Ok(builder.body(axum::body::Body::from_stream(stream)).unwrap())
}

/// `bytes=N-` (start..EOF), `bytes=N-M`, `bytes=-S` (suffix).
fn parse_range(v: &str, total: u64) -> Option<(u64, u64)> {
    let spec = v.strip_prefix("bytes=")?;
    if let Some(suffix) = spec.strip_prefix('-') {
        let n: u64 = suffix.trim().parse().ok()?;
        if n == 0 {
            return None;
        }
        let start = total.saturating_sub(n.min(total));
        return Some((start, total - 1));
    }
    let (first, second) = spec.split_once('-')?;
    let start: u64 = first.trim().parse().ok()?;
    if start >= total {
        return None;
    }
    if second.trim().is_empty() {
        return Some((start, total - 1));
    }
    let end: u64 = second.trim().parse().ok()?;
    Some((start, end.min(total - 1)))
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}


#[derive(Deserialize)]
struct RemoveQuery {
    #[serde(default)]
    delete_file: bool,
}

async fn download_action(
    State(st): State<Arc<AppState>>,
    Path((id, action)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiErr> {
    let res = match action.as_str() {
        "pause" => st.jobs.pause(&id),
        "resume" | "start" => st.jobs.start(&id),
        "stop" => st.jobs.stop(&id),
        "restart" => st.jobs.restart(&id),
        other => return Err(ApiErr::bad(format!("Unknown action '{other}'"))),
    };
    res.map_err(|e| ApiErr::bad(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

async fn remove_download(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<RemoveQuery>,
) -> Result<Json<serde_json::Value>, ApiErr> {
    st.jobs
        .remove(&id, q.delete_file)
        .map_err(|e| ApiErr::bad(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}
