use std::net::SocketAddr;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub database_url: String,
    pub supabase_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMode {
    Persistent(ServerConfig),
    InMemory,
}

impl ServerMode {
    pub fn from_env() -> Result<Self, ServerConfigError> {
        Self::from_pairs(std::env::vars())
    }

    pub fn from_pairs<I, K, V>(pairs: I) -> Result<Self, ServerConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let pairs: Vec<(String, String)> = pairs
            .into_iter()
            .map(|(key, value)| (key.as_ref().to_string(), value.into()))
            .collect();
        let mode = pairs
            .iter()
            .find_map(|(key, value)| (key == "ABUTOWN_SERVER_MODE").then_some(value.as_str()))
            .unwrap_or("persistent");

        match mode {
            "persistent" | "postgres" => Ok(Self::Persistent(ServerConfig::from_pairs(pairs)?)),
            "memory" | "in-memory" | "in_memory" => Ok(Self::InMemory),
            other => Err(ServerConfigError::UnknownServerMode(other.to_string())),
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, ServerConfigError> {
        Self::from_pairs(std::env::vars())
    }

    pub fn from_pairs<I, K, V>(pairs: I) -> Result<Self, ServerConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let mut database_url = None;
        let mut supabase_url = None;

        for (key, value) in pairs {
            match key.as_ref() {
                "DATABASE_URL" => database_url = Some(value.into()),
                "SUPABASE_URL" => supabase_url = Some(value.into()),
                _ => {}
            }
        }

        Ok(Self {
            database_url: database_url.ok_or(ServerConfigError::MissingDatabaseUrl)?,
            supabase_url: supabase_url.ok_or(ServerConfigError::MissingSupabaseUrl)?,
        })
    }
}

pub fn listen_addr_from_env() -> Result<SocketAddr, ServerConfigError> {
    listen_addr_from_pairs(std::env::vars())
}

pub fn listen_addr_from_pairs<I, K, V>(pairs: I) -> Result<SocketAddr, ServerConfigError>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: Into<String>,
{
    let bind_addr = pairs
        .into_iter()
        .find_map(|(key, value)| (key.as_ref() == "ABUTOWN_BIND_ADDR").then_some(value.into()))
        .unwrap_or_else(|| DEFAULT_BIND_ADDR.to_string());

    bind_addr
        .parse()
        .map_err(|_| ServerConfigError::InvalidBindAddr(bind_addr))
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ServerConfigError {
    #[error("DATABASE_URL is required")]
    MissingDatabaseUrl,
    #[error("SUPABASE_URL is required")]
    MissingSupabaseUrl,
    #[error("ABUTOWN_SERVER_MODE must be persistent or memory, got {0}")]
    UnknownServerMode(String),
    #[error("ABUTOWN_BIND_ADDR must be a socket address, got {0}")]
    InvalidBindAddr(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reads_required_supabase_database_values() {
        let config = ServerConfig::from_pairs([
            ("DATABASE_URL", "postgres://primary"),
            ("SUPABASE_URL", "https://project.supabase.co"),
        ])
        .unwrap();

        assert_eq!(config.database_url, "postgres://primary");
        assert_eq!(config.supabase_url, "https://project.supabase.co");
    }

    #[test]
    fn config_rejects_missing_database_url() {
        let error = ServerConfig::from_pairs([("SUPABASE_URL", "https://project.supabase.co")])
            .unwrap_err();

        assert_eq!(error, ServerConfigError::MissingDatabaseUrl);
    }

    #[test]
    fn config_rejects_missing_supabase_url() {
        let error = ServerConfig::from_pairs([("DATABASE_URL", "postgres://primary")]).unwrap_err();

        assert_eq!(error, ServerConfigError::MissingSupabaseUrl);
    }

    #[test]
    fn server_mode_memory_does_not_require_database_values() {
        let mode = ServerMode::from_pairs([("ABUTOWN_SERVER_MODE", "memory")]).unwrap();

        assert_eq!(mode, ServerMode::InMemory);
    }

    #[test]
    fn server_mode_defaults_to_persistent_config() {
        let mode = ServerMode::from_pairs([
            ("DATABASE_URL", "postgres://primary"),
            ("SUPABASE_URL", "https://project.supabase.co"),
        ])
        .unwrap();

        assert_eq!(
            mode,
            ServerMode::Persistent(ServerConfig {
                database_url: "postgres://primary".to_string(),
                supabase_url: "https://project.supabase.co".to_string(),
            })
        );
    }

    #[test]
    fn server_mode_rejects_unknown_value() {
        let error = ServerMode::from_pairs([("ABUTOWN_SERVER_MODE", "sqlite")]).unwrap_err();

        assert_eq!(
            error,
            ServerConfigError::UnknownServerMode("sqlite".to_string())
        );
    }

    #[test]
    fn listen_addr_defaults_to_loopback_8080() {
        let addr = listen_addr_from_pairs(std::iter::empty::<(&str, &str)>()).unwrap();

        assert_eq!(addr.to_string(), "127.0.0.1:8080");
    }

    #[test]
    fn listen_addr_reads_explicit_env_value() {
        let addr = listen_addr_from_pairs([("ABUTOWN_BIND_ADDR", "127.0.0.1:18080")]).unwrap();

        assert_eq!(addr.to_string(), "127.0.0.1:18080");
    }

    #[test]
    fn listen_addr_rejects_invalid_env_value() {
        let error = listen_addr_from_pairs([("ABUTOWN_BIND_ADDR", "not-an-address")]).unwrap_err();

        assert_eq!(
            error,
            ServerConfigError::InvalidBindAddr("not-an-address".to_string())
        );
    }
}
