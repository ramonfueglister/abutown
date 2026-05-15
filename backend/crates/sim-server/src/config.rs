#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub database_url: String,
    pub supabase_url: String,
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

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ServerConfigError {
    #[error("DATABASE_URL is required")]
    MissingDatabaseUrl,
    #[error("SUPABASE_URL is required")]
    MissingSupabaseUrl,
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
}
