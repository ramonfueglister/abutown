use std::net::SocketAddr;

use anyhow::Context;
use sim_server::app::build_app_with_allowed_origins;

fn allowed_origins_from_env() -> Vec<String> {
    std::env::var("CORS_ALLOWED_ORIGINS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(str::to_string)
        .collect()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let _ = dotenvy::dotenv();
    let addr: SocketAddr = "127.0.0.1:8080".parse().context("parse listen address")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("bind e2e simulation server")?;

    tracing::info!(%addr, "starting e2e sim-server");
    axum::serve(
        listener,
        build_app_with_allowed_origins(&allowed_origins_from_env())?,
    )
    .await
    .context("run e2e simulation server")
}
