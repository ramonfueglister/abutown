use std::net::SocketAddr;

use anyhow::Context;
use sim_server::{app::build_app_from_config, config::ServerConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let _ = dotenvy::dotenv();
    let config = ServerConfig::from_env().context("load server config")?;
    let addr: SocketAddr = "127.0.0.1:8080".parse().context("parse listen address")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("bind simulation server")?;

    tracing::info!(%addr, "starting sim-server");
    axum::serve(listener, build_app_from_config(&config).await?)
        .await
        .context("run simulation server")
}
