mod api;
mod config;
mod dav;
mod vcard;

use std::path::PathBuf;
use std::sync::Arc;

use api::AppState;
use config::Config;
use dav::CardDavClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.toml"));

    let cfg = Config::load(&config_path)?;
    let dav = CardDavClient::new(&cfg.baikal)?;

    let state = AppState { dav: Arc::new(dav) };
    let app = api::router(state);

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await?;
    tracing::info!("listening on {}", cfg.bind_addr);
    axum::serve(listener, app).await?;

    Ok(())
}
