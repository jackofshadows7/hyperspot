//! Database connection options and configuration types.

use crate::config::{DbConnConfig, GlobalDatabaseConfig, PoolCfg};
use crate::DbHandle;
use anyhow::{Context, Result};
use thiserror::Error;

// Pool configuration moved to config.rs

/// Database connection options using typed sqlx ConnectOptions.
#[derive(Debug, Clone)]
pub enum DbConnectOptions {
    #[cfg(feature = "sqlite")]
    Sqlite(sqlx::sqlite::SqliteConnectOptions),
    #[cfg(feature = "pg")]
    Postgres(sqlx::postgres::PgConnectOptions),
    #[cfg(feature = "mysql")]
    MySql(sqlx::mysql::MySqlConnectOptions),
}

/// Errors that can occur during connection option building.
#[derive(Debug, Error)]
pub enum ConnectionOptionsError {
    #[error("Invalid SQLite PRAGMA parameter '{key}': {message}")]
    InvalidSqlitePragma { key: String, message: String },

    #[error("Unknown SQLite PRAGMA parameter: {0}")]
    UnknownSqlitePragma(String),

    #[error("Invalid connection parameter: {0}")]
    InvalidParameter(String),

    #[error("Feature not enabled: {0}")]
    FeatureDisabled(&'static str),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("URL parsing error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("Environment variable error: {0}")]
    EnvVar(#[from] std::env::VarError),
}

impl std::fmt::Display for DbConnectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "sqlite")]
            DbConnectOptions::Sqlite(opts) => {
                let filename = opts.get_filename().display().to_string();
                if filename.is_empty() {
                    write!(f, "sqlite://memory")
                } else {
                    write!(f, "sqlite://{}", filename)
                }
            }
            #[cfg(feature = "pg")]
            DbConnectOptions::Postgres(opts) => {
                write!(
                    f,
                    "postgresql://<redacted>@{}:{}/{}",
                    opts.get_host(),
                    opts.get_port(),
                    opts.get_database().unwrap_or("")
                )
            }
            #[cfg(feature = "mysql")]
            DbConnectOptions::MySql(_opts) => {
                write!(f, "mysql://<redacted>@...")
            }
            #[cfg(not(any(feature = "sqlite", feature = "pg", feature = "mysql")))]
            _ => {
                unreachable!("No database features enabled")
            }
        }
    }
}

impl DbConnectOptions {
    /// Connect to the database using the configured options.
    pub async fn connect(&self, pool: PoolCfg) -> anyhow::Result<crate::DbHandle> {
        match self {
            #[cfg(feature = "sqlite")]
            DbConnectOptions::Sqlite(opts) => {
                let mut pool_opts = sqlx::sqlite::SqlitePoolOptions::new();

                if let Some(max_conns) = pool.max_conns {
                    pool_opts = pool_opts.max_connections(max_conns);
                }
                if let Some(timeout) = pool.acquire_timeout {
                    pool_opts = pool_opts.acquire_timeout(timeout);
                }

                let sqlx_pool = pool_opts.connect_with(opts.clone()).await?;

                #[cfg(feature = "sea-orm")]
                let sea = sea_orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(sqlx_pool.clone());

                let filename = opts.get_filename().display().to_string();
                let handle = crate::DbHandle {
                    engine: crate::DbEngine::Sqlite,
                    pool: crate::DbPool::Sqlite(sqlx_pool),
                    dsn: format!("sqlite://{}", filename),
                    #[cfg(feature = "sea-orm")]
                    sea,
                };

                Ok(handle)
            }
            #[cfg(feature = "pg")]
            DbConnectOptions::Postgres(opts) => {
                let mut pool_opts = sqlx::postgres::PgPoolOptions::new();

                if let Some(max_conns) = pool.max_conns {
                    pool_opts = pool_opts.max_connections(max_conns);
                }
                if let Some(timeout) = pool.acquire_timeout {
                    pool_opts = pool_opts.acquire_timeout(timeout);
                }

                let sqlx_pool = pool_opts.connect_with(opts.clone()).await?;

                #[cfg(feature = "sea-orm")]
                let sea =
                    sea_orm::SqlxPostgresConnector::from_sqlx_postgres_pool(sqlx_pool.clone());

                let handle = crate::DbHandle {
                    engine: crate::DbEngine::Postgres,
                    pool: crate::DbPool::Postgres(sqlx_pool),
                    dsn: format!(
                        "postgresql://<redacted>@{}:{}/{}",
                        opts.get_host(),
                        opts.get_port(),
                        opts.get_database().unwrap_or("")
                    ),
                    #[cfg(feature = "sea-orm")]
                    sea,
                };

                Ok(handle)
            }
            #[cfg(feature = "mysql")]
            DbConnectOptions::MySql(opts) => {
                let mut pool_opts = sqlx::mysql::MySqlPoolOptions::new();

                if let Some(max_conns) = pool.max_conns {
                    pool_opts = pool_opts.max_connections(max_conns);
                }
                if let Some(timeout) = pool.acquire_timeout {
                    pool_opts = pool_opts.acquire_timeout(timeout);
                }

                let sqlx_pool = pool_opts.connect_with(opts.clone()).await?;

                #[cfg(feature = "sea-orm")]
                let sea = sea_orm::SqlxMySqlConnector::from_sqlx_mysql_pool(sqlx_pool.clone());

                let handle = crate::DbHandle {
                    engine: crate::DbEngine::MySql,
                    pool: crate::DbPool::MySql(sqlx_pool),
                    dsn: "mysql://<redacted>@...".to_string(),
                    #[cfg(feature = "sea-orm")]
                    sea,
                };

                Ok(handle)
            }
            #[cfg(not(any(feature = "sqlite", feature = "pg", feature = "mysql")))]
            _ => {
                unreachable!("No database features enabled")
            }
        }
    }
}

/// SQLite PRAGMA whitelist and validation.
pub mod sqlite_pragma {
    use super::ConnectionOptionsError;
    use std::collections::HashMap;

    /// Whitelisted SQLite PRAGMA parameters.
    const ALLOWED_PRAGMAS: &[&str] = &["wal", "synchronous", "busy_timeout", "journal_mode"];

    /// Validate and apply SQLite PRAGMA parameters to connection options.
    pub fn apply_pragmas(
        mut opts: sqlx::sqlite::SqliteConnectOptions,
        params: &HashMap<String, String>,
    ) -> Result<sqlx::sqlite::SqliteConnectOptions, ConnectionOptionsError> {
        for (key, value) in params {
            let key_lower = key.to_lowercase();

            if !ALLOWED_PRAGMAS.contains(&key_lower.as_str()) {
                return Err(ConnectionOptionsError::UnknownSqlitePragma(key.clone()));
            }

            match key_lower.as_str() {
                "wal" => {
                    let journal_mode = validate_wal_pragma(value)?;
                    opts = opts.pragma("journal_mode", journal_mode);
                }
                "journal_mode" => {
                    let mode = validate_journal_mode_pragma(value)?;
                    opts = opts.pragma("journal_mode", mode);
                }
                "synchronous" => {
                    let sync_mode = validate_synchronous_pragma(value)?;
                    opts = opts.pragma("synchronous", sync_mode);
                }
                "busy_timeout" => {
                    let timeout = validate_busy_timeout_pragma(value)?;
                    opts = opts.pragma("busy_timeout", timeout.to_string());
                }
                _ => unreachable!("Checked against whitelist above"),
            }
        }

        Ok(opts)
    }

    /// Validate WAL PRAGMA value.
    fn validate_wal_pragma(value: &str) -> Result<&'static str, ConnectionOptionsError> {
        match value.to_lowercase().as_str() {
            "true" | "1" => Ok("WAL"),
            "false" | "0" => Ok("DELETE"),
            _ => Err(ConnectionOptionsError::InvalidSqlitePragma {
                key: "wal".to_string(),
                message: format!("must be true/false/1/0, got '{}'", value),
            }),
        }
    }

    /// Validate synchronous PRAGMA value.
    fn validate_synchronous_pragma(value: &str) -> Result<String, ConnectionOptionsError> {
        match value.to_uppercase().as_str() {
            "OFF" | "NORMAL" | "FULL" | "EXTRA" => Ok(value.to_uppercase()),
            _ => Err(ConnectionOptionsError::InvalidSqlitePragma {
                key: "synchronous".to_string(),
                message: format!("must be OFF/NORMAL/FULL/EXTRA, got '{}'", value),
            }),
        }
    }

    /// Validate busy_timeout PRAGMA value.
    fn validate_busy_timeout_pragma(value: &str) -> Result<i64, ConnectionOptionsError> {
        let timeout =
            value
                .parse::<i64>()
                .map_err(|_| ConnectionOptionsError::InvalidSqlitePragma {
                    key: "busy_timeout".to_string(),
                    message: format!("must be a non-negative integer, got '{}'", value),
                })?;

        if timeout < 0 {
            return Err(ConnectionOptionsError::InvalidSqlitePragma {
                key: "busy_timeout".to_string(),
                message: format!("must be non-negative, got '{}'", timeout),
            });
        }

        Ok(timeout)
    }

    /// Validate journal_mode PRAGMA value.
    fn validate_journal_mode_pragma(value: &str) -> Result<String, ConnectionOptionsError> {
        match value.to_uppercase().as_str() {
            "DELETE" | "WAL" | "MEMORY" | "TRUNCATE" | "PERSIST" | "OFF" => {
                Ok(value.to_uppercase())
            }
            _ => Err(ConnectionOptionsError::InvalidSqlitePragma {
                key: "journal_mode".to_string(),
                message: format!(
                    "must be DELETE/WAL/MEMORY/TRUNCATE/PERSIST/OFF, got '{}'",
                    value
                ),
            }),
        }
    }
}

/// Build a database handle from configuration.
/// This is the unified entry point for building database handles from configuration.
pub async fn build_db_handle(
    mut cfg: DbConnConfig,
    _global: Option<&GlobalDatabaseConfig>,
) -> Result<DbHandle> {
    // Expand environment variables in DSN and password
    if let Some(dsn) = &cfg.dsn {
        cfg.dsn = Some(expand_env_vars(dsn)?);
    }
    if let Some(password) = &cfg.password {
        cfg.password = Some(resolve_password(password)?);
    }

    // Expand environment variables in params
    if let Some(ref mut params) = cfg.params {
        for (_, value) in params.iter_mut() {
            if value.contains("${") {
                *value = expand_env_vars(value)?;
            }
        }
    }

    // Determine database type and build connection options
    let is_sqlite = cfg.file.is_some()
        || cfg.path.is_some()
        || cfg
            .dsn
            .as_ref()
            .is_some_and(|dsn| dsn.starts_with("sqlite"))
        || (cfg.server.is_none() && cfg.dsn.is_none());

    let connect_options = if is_sqlite {
        build_sqlite_options(&cfg)?
    } else {
        build_server_options(&cfg)?
    };

    // Build pool configuration
    let pool_cfg = cfg.pool.unwrap_or_default();

    // Log connection attempt (without credentials)
    let log_dsn = redact_credentials_in_dsn(cfg.dsn.as_deref());
    tracing::debug!(
        dsn = log_dsn,
        is_sqlite = is_sqlite,
        "Building database connection"
    );

    // Connect to database
    let handle = connect_options
        .connect(pool_cfg)
        .await
        .context("Failed to connect to database")?;

    Ok(handle)
}

/// Build SQLite connection options from configuration.
fn build_sqlite_options(cfg: &DbConnConfig) -> Result<DbConnectOptions, ConnectionOptionsError> {
    #[cfg(feature = "sqlite")]
    {
        let db_path = if let Some(dsn) = &cfg.dsn {
            parse_sqlite_path_from_dsn(dsn)?
        } else if let Some(path) = &cfg.path {
            path.clone()
        } else if let Some(_file) = &cfg.file {
            // This should not happen as manager.rs should have resolved file to path
            return Err(ConnectionOptionsError::InvalidParameter(
                "File path should have been resolved to absolute path".to_string(),
            ));
        } else {
            return Err(ConnectionOptionsError::InvalidParameter(
                "SQLite connection requires either DSN, path, or file".to_string(),
            ));
        };

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);

        // Apply PRAGMA parameters with whitelist validation
        if let Some(params) = &cfg.params {
            opts = sqlite_pragma::apply_pragmas(opts, params)?;
        }

        Ok(DbConnectOptions::Sqlite(opts))
    }
    #[cfg(not(feature = "sqlite"))]
    {
        Err(ConnectionOptionsError::FeatureDisabled(
            "SQLite feature not enabled",
        ))
    }
}

/// Build server-based connection options from configuration.
fn build_server_options(cfg: &DbConnConfig) -> Result<DbConnectOptions, ConnectionOptionsError> {
    // Determine the database type from DSN or default to PostgreSQL
    let scheme = if let Some(dsn) = &cfg.dsn {
        let parsed = url::Url::parse(dsn)?;
        parsed.scheme().to_string()
    } else {
        "postgresql".to_string()
    };

    match scheme.as_str() {
        "postgresql" | "postgres" => {
            #[cfg(feature = "pg")]
            {
                let mut opts = if let Some(dsn) = &cfg.dsn {
                    dsn.parse::<sqlx::postgres::PgConnectOptions>()
                        .map_err(|e| ConnectionOptionsError::InvalidParameter(e.to_string()))?
                } else {
                    sqlx::postgres::PgConnectOptions::new()
                };

                // Override with individual fields
                if let Some(host) = &cfg.host {
                    opts = opts.host(host);
                }
                if let Some(port) = cfg.port {
                    opts = opts.port(port);
                }
                if let Some(user) = &cfg.user {
                    opts = opts.username(user);
                }
                if let Some(password) = &cfg.password {
                    opts = opts.password(password);
                }
                if let Some(dbname) = &cfg.dbname {
                    opts = opts.database(dbname);
                } else if cfg.dsn.is_none() {
                    return Err(ConnectionOptionsError::InvalidParameter(
                        "dbname is required for PostgreSQL connections".to_string(),
                    ));
                }

                // Apply additional parameters
                if let Some(params) = &cfg.params {
                    for (key, value) in params {
                        opts = opts.options([(key.as_str(), value.as_str())]);
                    }
                }

                Ok(DbConnectOptions::Postgres(opts))
            }
            #[cfg(not(feature = "pg"))]
            {
                Err(ConnectionOptionsError::FeatureDisabled(
                    "PostgreSQL feature not enabled",
                ))
            }
        }
        "mysql" => {
            #[cfg(feature = "mysql")]
            {
                let mut opts = if let Some(dsn) = &cfg.dsn {
                    dsn.parse::<sqlx::mysql::MySqlConnectOptions>()
                        .map_err(|e| ConnectionOptionsError::InvalidParameter(e.to_string()))?
                } else {
                    sqlx::mysql::MySqlConnectOptions::new()
                };

                // Override with individual fields
                if let Some(host) = &cfg.host {
                    opts = opts.host(host);
                }
                if let Some(port) = cfg.port {
                    opts = opts.port(port);
                }
                if let Some(user) = &cfg.user {
                    opts = opts.username(user);
                }
                if let Some(password) = &cfg.password {
                    opts = opts.password(password);
                }
                if let Some(dbname) = &cfg.dbname {
                    opts = opts.database(dbname);
                } else if cfg.dsn.is_none() {
                    return Err(ConnectionOptionsError::InvalidParameter(
                        "dbname is required for MySQL connections".to_string(),
                    ));
                }

                Ok(DbConnectOptions::MySql(opts))
            }
            #[cfg(not(feature = "mysql"))]
            {
                Err(ConnectionOptionsError::FeatureDisabled(
                    "MySQL feature not enabled",
                ))
            }
        }
        _ => Err(ConnectionOptionsError::InvalidParameter(format!(
            "Unsupported database scheme: {}",
            scheme
        ))),
    }
}

/// Parse SQLite path from DSN.
fn parse_sqlite_path_from_dsn(dsn: &str) -> Result<std::path::PathBuf, ConnectionOptionsError> {
    if dsn.starts_with("sqlite:") {
        let path_part = dsn.strip_prefix("sqlite:").unwrap();
        let path_part = if path_part.starts_with("//") {
            path_part.strip_prefix("//").unwrap()
        } else {
            path_part
        };

        // Remove query parameters
        let path_part = if let Some(pos) = path_part.find('?') {
            &path_part[..pos]
        } else {
            path_part
        };

        Ok(std::path::PathBuf::from(path_part))
    } else {
        Err(ConnectionOptionsError::InvalidParameter(format!(
            "Invalid SQLite DSN: {}",
            dsn
        )))
    }
}

/// Expand environment variables in a string.
fn expand_env_vars(input: &str) -> Result<String> {
    let re = regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();
    let mut result = input.to_string();

    for caps in re.captures_iter(input) {
        let full_match = &caps[0];
        let var_name = &caps[1];
        let value = std::env::var(var_name)
            .with_context(|| format!("Environment variable '{}' not found", var_name))?;
        result = result.replace(full_match, &value);
    }

    Ok(result)
}

/// Resolve password from environment variable if it starts with ${VAR}.
fn resolve_password(password: &str) -> Result<String> {
    if password.starts_with("${") && password.ends_with('}') {
        let var_name = &password[2..password.len() - 1];
        std::env::var(var_name)
            .with_context(|| format!("Environment variable '{}' not found for password", var_name))
    } else {
        Ok(password.to_string())
    }
}

/// Redact credentials from DSN for logging.
pub fn redact_credentials_in_dsn(dsn: Option<&str>) -> String {
    match dsn {
        Some(dsn) if dsn.contains('@') => {
            if let Ok(mut parsed) = url::Url::parse(dsn) {
                if parsed.password().is_some() {
                    let _ = parsed.set_password(Some("***"));
                }
                parsed.to_string()
            } else {
                "***".to_string()
            }
        }
        Some(dsn) => dsn.to_string(),
        None => "none".to_string(),
    }
}
