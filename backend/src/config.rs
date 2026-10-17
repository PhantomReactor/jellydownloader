use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub port: u16,
    /// Where finished files are written.
    pub downloads_dir: PathBuf,
    /// Job/session state files.
    pub data_dir: PathBuf,
    /// How many jobs may run at once.
    pub parallel_downloads: u32,
    /// Auto-delete completed download files + records this many hours after
    /// they were finished. 0 disables cleanup. Also trims list entries whose
    /// file has disappeared.
    #[serde(default = "default_cleanup_hours")]
    pub cleanup_hours: u64,
    /// Compiled frontend assets. Overridable with JD_DIST env var.
    pub frontend_dist: String,
}

fn default_cleanup_hours() -> u64 {
    24
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 3001,
            downloads_dir: PathBuf::from("downloads"),
            data_dir: PathBuf::from("data"),
            parallel_downloads: 3,
            cleanup_hours: 24,
            frontend_dist: "../frontend/dist".to_string(),
        }
    }
}

impl Config {
    /// Load config.json from the working directory, creating it with defaults if missing.
    pub fn load() -> Self {
        let path = Path::new("config.json");
        let cfg = if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(raw) => serde_json::from_str(&raw)
                    .unwrap_or_else(|e| {
                        tracing::warn!("config.json invalid ({e}), using defaults");
                        Self::default()
                    }),
                Err(_) => Self::default(),
            }
        } else {
            let cfg = Self::default();
            if let Ok(raw) = serde_json::to_string_pretty(&cfg) {
                let _ = std::fs::write(path, raw);
            }
            cfg
        };
        if let Ok(env_dist) = std::env::var("JD_DIST") {
            if !env_dist.is_empty() {
                return Config { frontend_dist: env_dist, ..cfg };
            }
        }
        cfg
    }

    /// Locate the frontend dist directory, trying sensible candidates.
    pub fn resolve_dist(&self) -> Option<PathBuf> {
        let candidates = [
            PathBuf::from(&self.frontend_dist),
            PathBuf::from("frontend/dist"),
            PathBuf::from("dist"),
        ];
        for c in candidates {
            if c.join("index.html").exists() {
                return Some(c);
            }
        }
        None
    }
}
