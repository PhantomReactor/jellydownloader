mod api;
mod config;
mod engine;
mod error;
mod jellyfin;
mod jobs;
mod sessions;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use jobs::JobManager;
use jellyfin::JellyfinClient;
use sessions::SessionStore;
use std::sync::Arc;
use tracing::info;

pub struct AppState {
    pub config: Arc<config::Config>,
    pub jf: Arc<JellyfinClient>,
    pub sessions: SessionStore,
    pub jobs: Arc<JobManager>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut config = config::Config::load();
    config.server_url =
        jellyfin::normalize_server_url(&config.server_url).expect("config.json: invalid server_url");
    let config = Arc::new(config);
    std::fs::create_dir_all(&config.downloads_dir)?;
    std::fs::create_dir_all(&config.data_dir)?;

    // Warn early if ffmpeg is missing (only needed for transcoded downloads).
    match tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
    {
        Ok(s) if s.success() => info!("ffmpeg found"),
        _ => {
            tracing::warn!("ffmpeg not found on PATH — transcoded downloads will fail. Original quality still works.");
        }
    }

    let jf = Arc::new(JellyfinClient::new());
    let sessions = SessionStore::new(&config.data_dir);
    let jobs = JobManager::new(config.clone(), Arc::clone(&jf));
    let state = Arc::new(AppState { config: config.clone(), jf, sessions, jobs });

    let app = api::router()
        .fallback(static_handler)
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("jellydownloader listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn static_handler(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let dist = state.config.resolve_dist();
    let path = req.uri().path().trim_start_matches('/');

    // API routes never reach here (router handles them), but guard anyway.
    if path.starts_with("api/") {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "application/json")
            .body(Body::from("{\"message\":\"not found\"}"))
            .unwrap();
    }

    if !path.is_empty() {
        if let Some(dist) = &dist {
            let file_path = dist.join(path);
            if file_path.is_file() {
                if let Ok(bytes) = tokio::fs::read(&file_path).await {
                    let mime = mime_of(path);
                    // Trunk emits content-hashed names for wasm/js/css/svg — those
                    // are safely immortal; anything else is short-lived.
                    let cache_control = if path.ends_with(".html") {
                        "no-cache"
                    } else if path
                        .rsplit_once('-')
                        .and_then(|(_, tail)| tail.split('.').next())
                        .map(trailing_hash_ok)
                        .unwrap_or(false)
                    {
                        "public, max-age=31536000, immutable"
                    } else {
                        "public, max-age=300"
                    };
                    return Response::builder()
                        .header("content-type", mime)
                        .header("cache-control", cache_control)
                        .body(Body::from(bytes))
                        .unwrap();
                }
            }
        }
    }

    // Fall back to index.html (SPA) or explain that the frontend isn't built yet.
    match dist.as_ref().and_then(|d| std::fs::read(d.join("index.html")).ok()) {
        Some(bytes) => Response::builder()
            .header("content-type", "text/html; charset=utf-8")
            // Never cache the shell — hashed assets carry their own versioning.
            .header("cache-control", "no-cache")
            .body(Body::from(bytes))
            .unwrap(),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "text/html; charset=utf-8")
            .body(Body::from(
                "<h1>Frontend not built</h1><p>Run <code>trunk build</code> inside <code>frontend/</code> first.</p>",
            ))
            .unwrap(),
    }
}

/// `frontend-a86d74555457ad8d_bg.wasm` → hash is the final hex-only `-` chunk.
fn trailing_hash_ok(tail: &str) -> bool {
    let stem = tail.split('.').next().unwrap_or(tail).split('_').next().unwrap_or("");
    (8..=24).contains(&stem.len())
        && stem.chars().all(|c| c.is_ascii_hexdigit())
        && stem.chars().any(|c| c.is_ascii_alphabetic())
}

fn mime_of(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css",
        "js" => "text/javascript",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
