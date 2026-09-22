use serde::Deserialize;
use std::path::Path;

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

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config file {}: {e}", path.display()))?;
        let cfg: Config = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("failed to parse config file {}: {e}", path.display()))?;
        Ok(cfg)
    }
}
