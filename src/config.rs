use anyhow::Context;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// Address the API server binds to, e.g. "127.0.0.1:8080"
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
    pub baikal: BaikalConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BaikalConfig {
    /// Base URL of the Baikal instance, e.g. "http://localhost"
    pub base_url: String,
    /// Username used to authenticate against Baikal's CardDAV endpoint.
    pub username: String,
    /// Password used to authenticate against Baikal's CardDAV endpoint.
    pub password: String,
    /// Path (relative to base_url) of the addressbook collection to manage,
    /// e.g. "/dav.php/addressbooks/alan/default/"
    pub addressbook_path: String,
}

fn default_bind_addr() -> String {
    "127.0.0.1:8080".to_string()
}

/// Default config location: ~/.config/davserv/config.toml
pub fn default_config_path() -> anyhow::Result<PathBuf> {
    let base = dirs::config_dir().context("could not determine config directory")?;
    Ok(base.join("davserv").join("config.toml"))
}

const TEMPLATE: &str = include_str!("../config.example.toml");

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config file {}: {e}", path.display()))?;
        let cfg: Config = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("failed to parse config file {}: {e}", path.display()))?;
        Ok(cfg)
    }

    /// Loads from `path` if given, otherwise from the default location. If
    /// the default location doesn't exist yet, writes a template there and
    /// returns an error asking the user to fill it in, rather than running
    /// against a guessed Baikal host/credentials.
    pub fn load_default_or(explicit_path: Option<PathBuf>) -> anyhow::Result<Self> {
        if let Some(path) = explicit_path {
            return Self::load(&path);
        }

        let path = default_config_path()?;
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            std::fs::write(&path, TEMPLATE)
                .with_context(|| format!("failed to write default config to {}", path.display()))?;
            anyhow::bail!(
                "no config found, created a template at {} - edit it and re-run",
                path.display()
            );
        }

        Self::load(&path)
    }
}
