use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::json;
use shared::{ItemDetail, ItemSummary, LibraryView, MediaInfo};
use std::time::Duration;
use url::Url;

pub const AUTH_HEADER_BASE: &str = "MediaBrowser Client=\"Jellydown\", Device=\"Jellydown\", DeviceId=\"jellydown-desktop\", Version=\"0.1.0\"";

#[derive(Debug, Clone)]
pub struct Credentials {
    pub server_url: String,
    pub user_id: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
struct AuthResponse {
    #[serde(rename = "AccessToken")]
    access_token: String,
    #[serde(rename = "User")]
    user: JfUser,
}

#[derive(Debug, Deserialize)]
struct JfUser {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Debug, Deserialize)]
struct ItemsResponse {
    #[serde(rename = "Items", default)]
    items: Vec<JfItem>,
}

#[derive(Debug, Deserialize)]
struct JfItem {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Type")]
    item_type: String,
    #[serde(rename = "ProductionYear", default)]
    year: Option<i32>,
    #[serde(rename = "SeriesName", default)]
    series_name: Option<String>,
    #[serde(rename = "RunTimeTicks", default)]
    run_time_ticks: Option<i64>,
    #[serde(rename = "ParentIndexNumber", default)]
    parent_index_number: Option<i32>,
    #[serde(rename = "IndexNumber", default)]
    index_number: Option<i32>,
    #[serde(rename = "CollectionType", default)]
    collection_type: Option<String>,
}

pub fn normalize_server_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("Server URL is empty");
    }
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let parsed: Url = with_scheme.parse().context("Server URL is not valid")?;
    if parsed.host_str().is_none() {
        bail!("Server URL has no host");
    }
    Ok(with_scheme)
}

#[derive(Debug, Clone)]
pub struct JellyfinClient {
    http: reqwest::Client,
}

impl JellyfinClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build http client");
        Self { http }
    }

    /// Simple GET builder with auth header.
    fn req(&self, server_url: &str, path: &str, token: Option<&str>) -> reqwest::RequestBuilder {
        let mut auth = AUTH_HEADER_BASE.to_string();
        if let Some(t) = token {
            auth = format!("{auth}, Token=\"{t}\"");
        }
        let full = format!("{server_url}{path}");
        self.http
            .get(full)
            .header("Authorization", auth)
    }

    pub async fn authenticate(
        &self,
        server_url: &str,
        username: &str,
        password: &str,
    ) -> Result<(Credentials, String)> {
        let resp = self
            .http
            .post(format!("{server_url}/Users/AuthenticateByName"))
            .header("Authorization", AUTH_HEADER_BASE)
            .json(&json!({ "Username": username, "Pw": password }))
            .send()
            .await
            .context("Could not reach the Jellyfin server")?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            bail!("Wrong username or password");
        }
        if !status.is_success() {
            bail!("Jellyfin returned HTTP {status}");
        }
        let body: AuthResponse = resp.json().await.context("Unexpected login response")?;
        let creds = Credentials {
            server_url: server_url.to_string(),
            user_id: body.user.id.clone(),
            token: body.access_token,
        };
        Ok((creds, body.user.name))
    }

    pub async fn fetch_image(
        &self,
        c: &Credentials,
        item_id: &str,
        kind: &str,
        width: u32,
    ) -> Result<reqwest::Response> {
        let path = if kind == "backdrop" { "Backdrop" } else { "Primary" };
        let url = format!(
            "/Items/{item_id}/Images/{path}?api_key={}&fillWidth={width}&quality=90",
            c.token
        );
        Ok(self.req(&c.server_url, &url, Some(&c.token)).send().await?)
    }

    pub async fn user_views(&self, c: &Credentials) -> Result<Vec<LibraryView>> {
        let resp: ItemsResponse = self
            .req(&c.server_url, &format!("/Users/{}/Views", c.user_id), Some(&c.token))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .items
            .into_iter()
            .map(|i| LibraryView {
                id: i.id,
                name: i.name,
                collection_type: i.collection_type.unwrap_or_default(),
            })
            .collect())
    }

    pub async fn items(
        &self,
        c: &Credentials,
        parent_id: Option<&str>,
        search: Option<&str>,
        include_types: &str,
        limit: u32,
    ) -> Result<Vec<ItemSummary>> {
        let mut url = format!(
            "/Users/{}/Items?Recursive=true&SortBy=SortName&SortOrder=Ascending&Limit={limit}&Fields=RunTimeTicks&ImageTypeLimit=1&ExcludeLocationTypes=Virtual&IncludeItemTypes={include_types}",
            c.user_id
        );
        if let Some(p) = parent_id {
            url.push_str(&format!("&ParentId={p}"));
        }
        if let Some(s) = search {
            if !s.is_empty() {
                url.push_str(&format!("&SearchTerm={}", urlquery(s)));
            }
        }
        let resp: ItemsResponse = self
            .req(&c.server_url, &url, Some(&c.token))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .items
            .into_iter()
            .map(|i| ItemSummary {
                image_url: Some(Self::image_url(&i.id, 400)),
                id: i.id,
                name: i.name,
                item_type: i.item_type,
                year: i.year,
                series_name: i.series_name,
                season: i.parent_index_number,
                episode: i.index_number,
                run_time_ticks: i.run_time_ticks,
            })
            .collect())
    }

    pub async fn episodes(&self, c: &Credentials, series_id: &str) -> Result<Vec<ItemSummary>> {
        let url = format!(
            "/Shows/{series_id}/Episodes?userId={}&Fields=RunTimeTicks&ImageTypeLimit=1",
            c.user_id
        );
        let resp: ItemsResponse = self
            .req(&c.server_url, &url, Some(&c.token))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .items
            .into_iter()
            .map(|i| ItemSummary {
                image_url: Some(Self::image_url(&i.id, 400)),
                id: i.id,
                name: i.name,
                item_type: i.item_type,
                year: i.year,
                series_name: i.series_name,
                season: i.parent_index_number,
                episode: i.index_number,
                run_time_ticks: i.run_time_ticks,
            })
            .collect())
    }

    pub async fn item_detail(&self, c: &Credentials, item_id: &str) -> Result<ItemDetail> {
        let url = format!(
            "/Users/{}/Items/{item_id}?Fields=MediaSources,Path,Overview",
            c.user_id
        );
        let resp = self
            .req(&c.server_url, &url, Some(&c.token))
            .send()
            .await?
            .error_for_status()?;
        let body: serde_json::Value = resp.json().await?;
        let item: JfItem = serde_json::from_value(body.clone()).map_err(|e| anyhow!("bad item: {e}"))?;

        let media = extract_media_info(&body);
        let backdrop = body
            .get("BackdropImageTags")
            .and_then(|b| b.get(0))
            .and_then(|t| t.as_str())
            .map(|_| Self::backdrop_url(item_id));

        Ok(ItemDetail {
            image_url: Some(Self::image_url(&item.id, 500)),
            backdrop_url: backdrop,
            id: item.id,
            name: item.name,
            item_type: item.item_type,
            year: item.year,
            overview: body.get("Overview").and_then(|o| o.as_str()).map(String::from),
            series_name: item.series_name,
            season: item.parent_index_number,
            episode: item.index_number,
            parent_id: body.get("SeriesId").and_then(|s| s.as_str()).map(String::from),
            run_time_ticks: item.run_time_ticks,
            media,
        })
    }

    /// App-relative URL for the item's primary image. The backend proxies the
    /// actual bytes from Jellyfin so any origin (internal or external) works.
    fn image_url(item_id: &str, width: u32) -> String {
        format!("/api/image/{item_id}?w={width}")
    }

    fn backdrop_url(item_id: &str) -> String {
        format!("/api/image/{item_id}?type=backdrop&w=1200")
    }

    /// URL for the original file (range-capable static stream).
    pub fn static_stream_url(c: &Credentials, item_id: &str, is_audio: bool) -> String {
        let base = if is_audio { "Audio" } else { "Videos" };
        format!(
            "{}/{}/{item_id}/stream?static=true&api_key={}",
            c.server_url, base, c.token
        )
    }

    /// URL for the transcoding master playlist with a target quality.
    /// The video codec matches the source (e.g. HEVC -> HEVC) and the bitrate
    /// scales with the resolution change, so a downscale is smaller than the original.
    pub fn hls_master_url(
        c: &Credentials,
        item_id: &str,
        preset: &shared::QualityPreset,
        video_codec: &str,
        video_bitrate: u64,
    ) -> String {
        format!(
            "{}/Videos/{item_id}/master.m3u8?api_key={}&VideoCodec={video_codec}&AudioCodec=aac&VideoBitrate={video_bitrate}&AudioBitrate={}&MaxWidth={}&MaxHeight={}",
            c.server_url,
            c.token,
            preset.audio_bitrate.unwrap_or(128_000),
            preset.max_width.unwrap_or(1920),
            preset.max_height.unwrap_or(1080),
        )
    }
}

fn extract_media_info(body: &serde_json::Value) -> Option<MediaInfo> {
    let sources = body.get("MediaSources")?.as_array()?;
    let src = sources.first()?;
    let mut video_codec = None;
    let mut width = None;
    let mut height = None;
    let mut video_bitrate = None;
    let mut audio_codec = None;
    let mut audio_channels = None;
    if let Some(streams) = src.get("MediaStreams").and_then(|s| s.as_array()) {
        for s in streams {
            let t = s.get("Type").and_then(|t| t.as_str()).unwrap_or("");
            match t {
                "Video" => {
                    if video_codec.is_none() {
                        video_codec = s.get("Codec").and_then(|c| c.as_str()).map(String::from);
                        width = s.get("Width").and_then(|w| w.as_u64()).map(|w| w as u32);
                        height = s.get("Height").and_then(|h| h.as_u64()).map(|h| h as u32);
                        video_bitrate = s.get("BitRate").and_then(|b| b.as_u64());
                    }
                }
                "Audio" => {
                    if audio_codec.is_none() {
                        audio_codec = s.get("Codec").and_then(|c| c.as_str()).map(String::from);
                        audio_channels = s.get("Channels").and_then(|c| c.as_u64()).map(|c| c as u32);
                    }
                }
                _ => {}
            }
        }
    }
    Some(MediaInfo {
        container: src.get("Container").and_then(|c| c.as_str()).map(String::from),
        video_codec,
        width,
        height,
        video_bitrate,
        audio_codec,
        audio_channels,
        size: src.get("Size").and_then(|s| s.as_u64()),
    })
}

fn urlquery(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
