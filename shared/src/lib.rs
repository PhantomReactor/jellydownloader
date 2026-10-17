use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityKind {
    Original,
    Transcode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityPreset {
    pub id: String,
    pub label: String,
    pub kind: QualityKind,
    /// Target video bitrate in bits/sec for transcodes.
    pub video_bitrate: Option<u64>,
    /// Target audio bitrate in bits/sec for transcodes.
    pub audio_bitrate: Option<u64>,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    /// Estimated bitrate used for size hints (bits/sec), if any.
    pub estimate_bps: Option<u64>,
}

pub fn quality_presets() -> Vec<QualityPreset> {
    vec![
        QualityPreset {
            id: "original".to_string(),
            label: "Original".to_string(),
            kind: QualityKind::Original,
            video_bitrate: None,
            audio_bitrate: None,
            max_width: None,
            max_height: None,
            estimate_bps: None,
        },
        QualityPreset {
            id: "1080p".to_string(),
            label: "1080p".to_string(),
            kind: QualityKind::Transcode,
            video_bitrate: Some(8_000_000),
            audio_bitrate: Some(128_000),
            max_width: Some(1920),
            max_height: Some(1080),
            estimate_bps: Some(8_128_000),
        },
        QualityPreset {
            id: "720p".to_string(),
            label: "720p".to_string(),
            kind: QualityKind::Transcode,
            video_bitrate: Some(4_000_000),
            audio_bitrate: Some(128_000),
            max_width: Some(1280),
            max_height: Some(720),
            estimate_bps: Some(4_128_000),
        },
        QualityPreset {
            id: "480p".to_string(),
            label: "480p".to_string(),
            kind: QualityKind::Transcode,
            video_bitrate: Some(1_500_000),
            audio_bitrate: Some(128_000),
            max_width: Some(854),
            max_height: Some(480),
            estimate_bps: Some(1_628_000),
        },
    ]
}

pub fn preset_by_id(id: &str) -> Option<QualityPreset> {
    quality_presets().into_iter().find(|q| q.id == id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub server_url: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub server_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryView {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub collection_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemSummary {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub year: Option<i32>,
    pub series_name: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub run_time_ticks: Option<i64>,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    /// Jellyfin MediaSource id (needed for stream/transcode endpoints).
    pub source_id: Option<String>,
    pub container: Option<String>,
    pub video_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub video_bitrate: Option<u64>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<u32>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemDetail {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub series_name: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub parent_id: Option<String>,
    pub run_time_ticks: Option<i64>,
    pub image_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub media: Option<MediaInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Paused,
    Stopped,
    Remuxing,
    Completed,
    Failed,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Paused => "paused",
            JobState::Stopped => "stopped",
            JobState::Remuxing => "remuxing",
            JobState::Completed => "completed",
            JobState::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadJob {
    pub id: String,
    pub item_id: String,
    pub item_name: String,
    pub series_name: Option<String>,
    pub year: Option<i32>,
    pub image_url: Option<String>,
    pub quality_id: String,
    pub quality_label: String,
    /// Progress 0..=1 when computable (original: bytes ratio; transcode: duration ratio).
    pub progress: Option<f64>,
    pub state: JobState,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub segments_done: u32,
    pub segments_total: Option<u32>,
    pub speed_bps: f64,
    pub eta_seconds: Option<u64>,
    pub error: Option<String>,
    pub file_path: Option<String>,
    pub created_at: u64,
    /// Browser should offer the file for saving when the job completes.
    pub auto_save: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateDownloadRequest {
    pub item_id: String,
    pub quality_id: String,
    /// When set, the user's browser starts saving the file the moment the job finishes.
    #[serde(default)]
    pub auto_save: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub message: String,
}

impl ApiError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self { message: msg.into() }
    }
}
