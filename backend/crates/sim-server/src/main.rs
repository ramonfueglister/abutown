use std::net::SocketAddr;

use anyhow::Context;
use sim_server::{app::build_app_from_config, config::ServerConfig};

/// Resolve the TCP listen address from host + port. Host must be a numeric IP
/// (`SocketAddr` does not resolve hostnames): `127.0.0.1` for dev (loopback only),
/// `0.0.0.0` in a container (all interfaces). Returns an error for a non-IP host
/// rather than silently failing.
fn resolve_listen_addr(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
    format!("{host}:{port}")
        .parse()
        .with_context(|| format!("parse listen address {host}:{port}"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let _ = dotenvy::dotenv();
    let config = ServerConfig::from_env().context("load server config")?;
    let port: u16 = match std::env::var("LISTEN_PORT") {
        Err(_) => 8080,
        Ok(v) => v.parse().context("LISTEN_PORT must be a valid u16")?,
    };
    let host = std::env::var("LISTEN_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let addr = resolve_listen_addr(&host, port)?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("bind simulation server")?;

    tracing::info!(%addr, "starting sim-server");
    axum::serve(listener, build_app_from_config(&config).await?)
        .await
        .context("run simulation server")
}

#[cfg(test)]
mod tests {
    use super::resolve_listen_addr;

    #[test]
    fn defaults_to_loopback() {
        let addr = resolve_listen_addr("127.0.0.1", 8080).expect("valid loopback addr");
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert_eq!(addr.port(), 8080);
        assert!(addr.ip().is_loopback());
    }

    #[test]
    fn binds_all_interfaces_when_overridden() {
        let addr = resolve_listen_addr("0.0.0.0", 8080).expect("valid wildcard addr");
        assert!(
            addr.ip().is_unspecified(),
            "0.0.0.0 must be the unspecified (all-interfaces) addr"
        );
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn rejects_non_ip_host() {
        // SocketAddr parsing is numeric-only — a hostname like "localhost" must
        // error, not silently bind nowhere.
        assert!(resolve_listen_addr("not-an-ip", 8080).is_err());
    }
}
