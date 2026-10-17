use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Operable settings. Everything but `server_url` has a sensible default, so
/// `{ "server_url": "..." }` alone is a complete, valid config.
#[derive(Debug, Clone)]
pub struct Config {
    /// HTTP port on which the app is served.
    pub port: u16,
    /// Where finished files are written.
    pub downloads_dir: PathBuf,
    /// Job/session state files.
    pub data_dir: PathBuf,
    /// How many parallel jobs are downloaded at once.
    pub parallel_downloads: u32,
    /// Auto-delete completed download files + records this many hours after
    /// they were finished. 0 disables cleanup.
    pub cleanup_hours: u64,
    /// Compiled frontend assets. Overridable with JD_DIST env var.
    pub frontend_dist: String,
    /// The Jellyfin server everyone uses. REQUIRED — the app refuses to start
    /// without it, and the login form never asks for a URL.
    pub server_url: String,
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
            server_url: String::new(),
        }
    }
}

/// Loose deserialization target: every field optional; `Config::load` merges
/// them over `Config::default()`. Also ignores unknown keys that older/newer
/// configs might carry.
#[derive(Serialize, Deserialize, Default)]
struct ConfigPartial {
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    downloads_dir: Option<PathBuf>,
    #[serde(default)]
    data_dir: Option<PathBuf>,
    #[serde(default)]
    parallel_downloads: Option<u32>,
    #[serde(default)]
    cleanup_hours: Option<u64>,
    #[serde(default)]
    frontend_dist: Option<String>,
    #[serde(default)]
    server_url: Option<String>,
}

impl Config {
    /// Load config.json (creating a skeleton if missing). Exits the process when
    /// `server_url` is missing/blank — by design: no start without a Jellyfin to talk to.
    pub fn load() -> Self {
        let path = Path::new("config.json");
        let raw = if path.exists() {
            std::fs::read_to_string(path).unwrap_or_else(|e| {
                eprintln!("\nconfig.json exists but unreadable: {e}\n");
                std::process::exit(1);
            })
        } else {
            let base = Self::default();
            if let Ok(text) = serde_json::to_string_pretty(&json_skeleton(&base)) {
                let _ = std::fs::write(path, text);
            }
            eprintln!(
                "\nconfig.json was created — set \"server_url\" to your Jellyfin address\n(e.g. \"http://192.168.1.100:8096\") and start the app again.\n"
            );
            std::process::exit(1);
        };

        let partial: ConfigPartial = match serde_json::from_str(&raw) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "\nconfig.json could not be used: {e}\nAt minimum it needs:\n  {{ \"server_url\": \"http://<your-jellyfin>:8096\" }}\n"
                );
                std::process::exit(1);
            }
        };

        let base = Self::default();
        let mut cfg = Config {
            server_url: partial.server_url.unwrap_or_default(),
            ..base
        };
        if let Some(v) = partial.port {
            cfg.port = v;
        }
        if let Some(v) = partial.downloads_dir {
            cfg.downloads_dir = v;
        }
        if let Some(v) = partial.data_dir {
            cfg.data_dir = v;
        }
        if let Some(v) = partial.parallel_downloads {
            cfg.parallel_downloads = v;
        }
        if let Some(v) = partial.cleanup_hours {
            cfg.cleanup_hours = v;
        }
        if let Ok(env_dist) = std::env::var("JD_DIST") {
            if !env_dist.is_empty() {
                cfg.frontend_dist = env_dist;
            } else if let Some(v) = partial.frontend_dist {
                cfg.frontend_dist = v;
            }
        } else if let Some(v) = partial.frontend_dist {
            cfg.frontend_dist = v;
        }

        if cfg.server_url.trim().is_empty() {
            eprintln!(
                "\nconfig.json has no \"server_url\" — set it to your Jellyfin address\n(e.g. \"http://192.168.1.100:8096\") and start the app again.\n"
            );
            std::process::exit(1);
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

fn json_skeleton(base: &Config) -> ConfigPartial {
    ConfigPartial {
        port: Some(base.port),
        downloads_dir: Some(base.downloads_dir.clone()),
        data_dir: Some(base.data_dir.clone()),
        parallel_downloads: Some(base.parallel_downloads),
        cleanup_hours: Some(base.cleanup_hours),
        frontend_dist: Some(base.frontend_dist.clone()),
        server_url: None,
    }
}
