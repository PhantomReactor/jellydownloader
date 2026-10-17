use gloo_net::http::Request;
use serde::de::DeserializeOwned;
use serde::Serialize;
use shared::{ApiError, CreateDownloadRequest, DownloadJob, ItemDetail, ItemSummary, LibraryView, LoginRequest, User};

#[derive(Clone, serde::Deserialize)]
pub struct DownloadList {
    pub jobs: Vec<DownloadJob>,
    /// Completed auto-save jobs owned by this session, not yet delivered.
    pub save_ready: Vec<String>,
}

async fn error_from(resp: gloo_net::http::Response) -> String {
    resp.json::<ApiError>()
        .await
        .map(|e| e.message)
        .unwrap_or_else(|_| format!("Request failed ({})", resp.status()))
}

pub async fn get<T: DeserializeOwned>(path: String) -> Result<T, String> {
    let resp = Request::get(&path).send().await.map_err(|e| e.to_string())?;
    if resp.ok() {
        resp.json::<T>().await.map_err(|e| e.to_string())
    } else {
        Err(error_from(resp).await)
    }
}

pub async fn post<T: DeserializeOwned>(path: &str, body: &impl Serialize) -> Result<T, String> {
    let resp = Request::post(path)
        .json(body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.ok() {
        resp.json::<T>().await.map_err(|e| e.to_string())
    } else {
        Err(error_from(resp).await)
    }
}

pub async fn post_empty<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    let resp = Request::post(path).send().await.map_err(|e| e.to_string())?;
    if resp.ok() {
        resp.json::<T>().await.map_err(|e| e.to_string())
    } else {
        Err(error_from(resp).await)
    }
}

pub async fn delete_req<T: DeserializeOwned>(path: String) -> Result<T, String> {
    let resp = Request::delete(&path).send().await.map_err(|e| e.to_string())?;
    if resp.ok() {
        resp.json::<T>().await.map_err(|e| e.to_string())
    } else {
        Err(error_from(resp).await)
    }
}

pub async fn login(username: String, password: String) -> Result<User, String> {
    post::<User>("/api/login", &LoginRequest { username, password }).await
}

pub async fn me() -> Result<User, String> {
    get("/api/me".to_string()).await
}

pub async fn logout() -> Result<serde_json::Value, String> {
    post_empty("/api/logout").await
}

pub async fn libraries() -> Result<Vec<LibraryView>, String> {
    get("/api/libraries".to_string()).await
}

pub async fn items(library: Option<String>, search: Option<String>) -> Result<Vec<ItemSummary>, String> {
    let mut path = String::from("/api/items?");
    if let Some(l) = library {
        path.push_str(&format!("library={l}&"));
    }
    if let Some(s) = search {
        path.push_str(&format!("search={}", js_sys::encode_uri_component(&s)));
    }
    get(path).await
}

pub async fn item_detail(id: String) -> Result<ItemDetail, String> {
    get(format!("/api/items/{id}")).await
}

pub async fn episodes(series_id: String) -> Result<Vec<ItemSummary>, String> {
    get(format!("/api/series/{series_id}/episodes")).await
}

#[derive(Clone, serde::Deserialize)]
pub struct CreateResult {
    pub job: DownloadJob,
    /// "completed" = identical download already finished; "active" = already queued.
    pub existing: Option<String>,
}

pub async fn create_download(item_id: String, quality_id: String, auto_save: bool) -> Result<CreateResult, String> {
    post("/api/downloads", &CreateDownloadRequest { item_id, quality_id, auto_save }).await
}

pub async fn jobs() -> Result<DownloadList, String> {
    get("/api/downloads".to_string()).await
}

pub async fn job_action(id: String, action: &'static str) -> Result<serde_json::Value, String> {
    post_empty(&format!("/api/downloads/{id}/{action}")).await
}

pub async fn remove_job(id: String, delete_file: bool) -> Result<serde_json::Value, String> {
    delete_req(format!("/api/downloads/{id}?delete_file={delete_file}")).await
}
