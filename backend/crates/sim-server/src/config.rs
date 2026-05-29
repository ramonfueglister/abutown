#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub database_url: String,
    pub supabase_url: String,
    pub cors_allowed_origins: Vec<String>,
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
        let mut cors_allowed_origins = Vec::new();

        for (key, value) in pairs {
            match key.as_ref() {
                "DATABASE_URL" => database_url = Some(value.into()),
                "SUPABASE_URL" => supabase_url = Some(value.into()),
                "CORS_ALLOWED_ORIGINS" => {
                    cors_allowed_origins = value
                        .into()
                        .split(',')
                        .map(str::trim)
                        .filter(|origin| !origin.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                _ => {}
            }
        }

        Ok(Self {
            database_url: database_url.ok_or(ServerConfigError::MissingDatabaseUrl)?,
            supabase_url: supabase_url.ok_or(ServerConfigError::MissingSupabaseUrl)?,
            cors_allowed_origins,
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

    #[test]
    fn config_parses_comma_separated_cors_origins() {
        let config = ServerConfig::from_pairs([
            ("DATABASE_URL", "postgres://primary"),
            ("SUPABASE_URL", "https://project.supabase.co"),
            (
                "CORS_ALLOWED_ORIGINS",
                "http://127.0.0.1:5173,https://app.example.com",
            ),
        ])
        .unwrap();

        assert_eq!(
            config.cors_allowed_origins,
            vec![
                "http://127.0.0.1:5173".to_string(),
                "https://app.example.com".to_string(),
            ]
        );
    }

    #[test]
    fn config_defaults_to_no_cors_origins_when_unset() {
        let config = ServerConfig::from_pairs([
            ("DATABASE_URL", "postgres://primary"),
            ("SUPABASE_URL", "https://project.supabase.co"),
        ])
        .unwrap();

        assert!(config.cors_allowed_origins.is_empty());
    }

    #[test]
    fn config_ignores_blank_cors_entries() {
        let config = ServerConfig::from_pairs([
            ("DATABASE_URL", "postgres://primary"),
            ("SUPABASE_URL", "https://project.supabase.co"),
            ("CORS_ALLOWED_ORIGINS", " http://a , ,http://b "),
        ])
        .unwrap();

        assert_eq!(
            config.cors_allowed_origins,
            vec!["http://a".to_string(), "http://b".to_string()]
        );
    }
}
