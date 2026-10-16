use crate::error::ApiErr;
use crate::jellyfin::Credentials;
use axum::http::{header, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use shared::{ApiError, User};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::RwLock;

pub const COOKIE_NAME: &str = "jd_session";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub user: User,
    pub token: String,
}

pub struct SessionStore {
    map: RwLock<HashMap<String, Session>>,
    path: PathBuf,
}

#[derive(Serialize, Deserialize, Default)]
struct SessionFile {
    sessions: Vec<Session>,
}

impl SessionStore {
    pub fn new(data_dir: &std::path::Path) -> Self {
        let _ = std::fs::create_dir_all(data_dir);
        let path = data_dir.join("sessions.json");
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<SessionFile>(&raw).ok())
            .map(|file| {
                file.sessions
                    .into_iter()
                    .map(|s| (s.id.clone(), s))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        Self { map: RwLock::new(map), path }
    }

    async fn persist(&self, map: &HashMap<String, Session>) {
        let file = SessionFile { sessions: map.values().cloned().collect() };
        if let Ok(raw) = serde_json::to_string_pretty(&file) {
            let tmp = self.path.with_extension("tmp");
            if std::fs::write(&tmp, raw).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }

    pub async fn insert(&self, session: Session) {
        let mut map = self.map.write().await;
        map.insert(session.id.clone(), session);
        self.persist(&map).await;
    }

    pub async fn get(&self, id: &str) -> Option<Session> {
        self.map.read().await.get(id).cloned()
    }

    pub async fn remove(&self, id: &str) {
        let mut map = self.map.write().await;
        map.remove(id);
        self.persist(&map).await;
    }

    pub fn credentials(&self, session: &Session) -> Credentials {
        Credentials {
            server_url: session.user.server_url.clone(),
            user_id: session.user.id.clone(),
            token: session.token.clone(),
        }
    }
}

/// Extract the current session from request headers (cookie).
pub async fn current_session(store: &SessionStore, headers: &HeaderMap) -> Result<Session, ApiErr> {
    let cookie_raw = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mut token_value = None;
    for part in cookie_raw.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(COOKIE_NAME) {
            let rest = rest.strip_prefix('=').unwrap_or(rest);
            token_value = Some(rest.to_string());
        }
    }
    let sid = token_value.ok_or_else(|| {
        ApiErr { status: axum::http::StatusCode::UNAUTHORIZED, err: ApiError::new("Not logged in".to_string()) }
    })?;
    store
        .get(&sid)
        .await
        .ok_or_else(|| ApiErr { status: axum::http::StatusCode::UNAUTHORIZED, err: ApiError::new("Session expired".to_string()) })
}

pub fn session_cookie(session_id: &str, secure: bool) -> HeaderValue {
    let mut cookie = format!("{COOKIE_NAME}={session_id}; HttpOnly; Path=/; SameSite=Lax; Max-Age=31536000");
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie).expect("cookie header")
}

pub fn clear_cookie() -> HeaderValue {
    HeaderValue::from_str(&format!("{COOKIE_NAME}=deleted; HttpOnly; Path=/; SameSite=Lax; Max-Age=0"))
        .expect("cookie header")
}
