//! Database configuration types.
//!
//! This module contains the canonical definitions of all database configuration
//! structures used throughout the system. These types are deserialized directly
//! from Figment configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Global database configuration with server-based DBs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalDatabaseConfig {
    /// Server-based DBs (postgres/mysql/sqlite/etc.), keyed by server name.
    #[serde(default)]
    pub servers: HashMap<String, DbConnConfig>,
    /// Optional dev-only flag to auto-provision DB/schema when missing.
    #[serde(default)]
    pub auto_provision: Option<bool>,
}

/// Reusable DB connection config for both global servers and modules.
/// DSN must be a FULL, valid DSN if provided (dsn crate compliant).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DbConnConfig {
    // DSN-style (full, valid). Optional: can be absent and rely on fields.
    pub dsn: Option<String>,

    // Field-based style; any of these override DSN parts when present:
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<String>, // literal password or ${VAR} for env expansion
    pub dbname: Option<String>,   // MUST be present in final for server-based DBs
    #[serde(default)]
    pub params: Option<HashMap<String, String>>,

    // SQLite file-based helpers (module-level only; ignored for global):
    pub file: Option<String>,  // relative name under home_dir/module
    pub path: Option<PathBuf>, // absolute path

    // Connection pool overrides:
    #[serde(default)]
    pub pool: Option<PoolCfg>,

    // Module-level only: reference to a global server by name.
    // If absent, this module config must be fully self-sufficient (dsn or fields).
    pub server: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PoolCfg {
    pub max_conns: Option<u32>,
    #[serde(with = "humantime_serde", default)]
    pub acquire_timeout: Option<Duration>,
    // add other pool knobs already supported by options.rs
}
