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
    /// The Jellyfin server everyone uses. REQUIRED — the app refuses to start
    /// without it, and the login form never asks for a URL.
    pub server_url: String,
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
            server_url: String::new(),
        }
    }
}

impl Config {
    /// Load config.json (creating a skeleton if missing). Exits the process when
    /// `server_url` is missing/blank — by design: no start without a Jellyfin to talk to.
    pub fn load() -> Self {
        let path = Path::new("config.json");
        if !path.exists() {
            let cfg = Self::default();
            if let Ok(raw) = serde_json::to_string_pretty(&cfg) {
                let _ = std::fs::write(path, raw);
            }
            eprintln!(
                "\nconfig.json was created — set \"server_url\" to your Jellyfin address\n(e.g. \"http://192.168.1.100:8096\") and start the app again.\n"
            );
            std::process::exit(1);
        }
        let mut cfg: Config = match std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
        {
            Some(c) => c,
            None => {
                eprintln!(
                    "\nconfig.json could not be parsed (invalid JSON) or is missing required fields.\nAt minimum it needs:
  {{ \"server_url\": \"http://<your-jellyfin>:8096\" }}\n"
                );
                std::process::exit(1);
            }
        };
        if cfg.server_url.trim().is_empty() {
            eprintln!(
                "\nconfig.json has no \"server_url\" — set it to your Jellyfin address\n(e.g. \"http://192.168.1.100:8096\") and start the app again.\n"
            );
            std::process::exit(1);
        }
        if let Ok(env_dist) = std::env::var("JD_DIST") {
            if !env_dist.is_empty() {
                cfg.frontend_dist = env_dist;
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
