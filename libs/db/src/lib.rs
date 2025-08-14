#![cfg_attr(
    not(any(feature = "pg", feature = "mysql", feature = "sqlite")),
    allow(
        unused_imports,
        unused_variables,
        dead_code,
        unreachable_code,
        unused_lifetimes
    )
)]
//! Database abstraction crate providing a database-agnostic `DbHandle`.
//!
//! This crate provides a unified interface for working with different databases
//! (SQLite, PostgreSQL, MySQL) through SQLx, with optional SeaORM integration.
//!
//! # Features
//! - `pg`, `mysql`, `sqlite`: enable SQLx backends
//! - `sea-orm`: add SeaORM integration for type-safe operations
//!
//! # Example
//! ```rust,no_run
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     use db::{DbHandle, ConnectOpts};
//!
//!     let db = DbHandle::connect("postgres://user:pass@localhost/app", ConnectOpts::default()).await?;
//!
//!     // sqlx
//!     #[cfg(feature="pg")]
//!     {
//!         let pool = db.sqlx_postgres().unwrap();
//!         sqlx::query("select 1").execute(pool).await?;
//!
//!         db.with_pg_tx(|tx| async move {
//!             sqlx::query("select 2").execute(&mut *tx).await?;
//!             Ok(())
//!         }).await?;
//!     }
//!
//!     // sea-orm (if enabled)
//!     #[cfg(feature="sea-orm")]
//!     {
//!         use sea_orm::{ConnectionTrait, Statement, DatabaseBackend};
//!         db.sea().execute(Statement::from_string(DatabaseBackend::Postgres, "SELECT 3")).await?;
//!     }
//!
//!     db.close().await;
//!     Ok(())
//! }
//! ```

use anyhow::{bail, Context, Result};
use std::time::Duration;

#[cfg(feature = "mysql")]
use sqlx::{mysql::MySqlPoolOptions, MySql, MySqlPool};
#[cfg(feature = "pg")]
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres};
#[cfg(feature = "sqlite")]
use sqlx::{sqlite::SqlitePoolOptions, Sqlite, SqlitePool};

#[cfg(feature = "sea-orm")]
use sea_orm::DatabaseConnection;
#[cfg(all(feature = "sea-orm", feature = "mysql"))]
use sea_orm::SqlxMySqlConnector;
#[cfg(all(feature = "sea-orm", feature = "pg"))]
use sea_orm::SqlxPostgresConnector;
#[cfg(all(feature = "sea-orm", feature = "sqlite"))]
use sea_orm::SqlxSqliteConnector;

// Re-export key types for public API
pub use advisory_locks::{DbLockGuard, LockConfig};

// Advisory locks module
pub mod advisory_locks;

/// Supported engines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbEngine {
    Postgres,
    MySql,
    Sqlite,
}

/// Connection options.
#[derive(Clone, Debug)]
pub struct ConnectOpts {
    pub max_conns: Option<u32>,
    pub acquire_timeout: Option<Duration>,
    pub sqlite_busy_timeout: Option<Duration>,
    /// Create parent directories for SQLite file path DSNs.
    pub create_sqlite_dirs: bool,
}
impl Default for ConnectOpts {
    fn default() -> Self {
        Self {
            max_conns: Some(10),
            acquire_timeout: Some(Duration::from_secs(30)),
            sqlite_busy_timeout: Some(Duration::from_millis(5_000)),
            create_sqlite_dirs: true,
        }
    }
}

/// One concrete sqlx pool.
#[derive(Clone)]
pub enum DbPool {
    #[cfg(feature = "pg")]
    Postgres(PgPool),
    #[cfg(feature = "mysql")]
    MySql(MySqlPool),
    #[cfg(feature = "sqlite")]
    Sqlite(SqlitePool),
}

/// Database transaction wrapper (lifetime-bound to the pool).
pub enum DbTransaction<'a> {
    #[cfg(feature = "pg")]
    Postgres(sqlx::Transaction<'a, Postgres>),
    #[cfg(feature = "mysql")]
    MySql(sqlx::Transaction<'a, MySql>),
    #[cfg(feature = "sqlite")]
    Sqlite(sqlx::Transaction<'a, Sqlite>),
    // When no concrete DB feature is enabled, keep a variant to tie `'a` so
    // the type still compiles and can be referenced in signatures.
    #[cfg(not(any(feature = "pg", feature = "mysql", feature = "sqlite")))]
    _Phantom(std::marker::PhantomData<&'a ()>),
}

impl<'a> DbTransaction<'a> {
    /// Commit the transaction.
    pub async fn commit(self) -> Result<()> {
        match self {
            #[cfg(feature = "pg")]
            DbTransaction::Postgres(tx) => tx.commit().await.map_err(Into::into),
            #[cfg(feature = "mysql")]
            DbTransaction::MySql(tx) => tx.commit().await.map_err(Into::into),
            #[cfg(feature = "sqlite")]
            DbTransaction::Sqlite(tx) => tx.commit().await.map_err(Into::into),
            #[cfg(not(any(feature = "pg", feature = "mysql", feature = "sqlite")))]
            DbTransaction::_Phantom(_) => Ok(()),
        }
    }

    /// Roll back the transaction.
    pub async fn rollback(self) -> Result<()> {
        match self {
            #[cfg(feature = "pg")]
            DbTransaction::Postgres(tx) => tx.rollback().await.map_err(Into::into),
            #[cfg(feature = "mysql")]
            DbTransaction::MySql(tx) => tx.rollback().await.map_err(Into::into),
            #[cfg(feature = "sqlite")]
            DbTransaction::Sqlite(tx) => tx.rollback().await.map_err(Into::into),
            #[cfg(not(any(feature = "pg", feature = "mysql", feature = "sqlite")))]
            DbTransaction::_Phantom(_) => Ok(()),
        }
    }
}

/// Main handle.
pub struct DbHandle {
    engine: DbEngine,
    pool: DbPool,
    dsn: String,
    #[cfg(feature = "sea-orm")]
    sea: DatabaseConnection,
}

impl DbHandle {
    /// Detect engine by DSN.
    pub fn detect(dsn: &str) -> Result<DbEngine> {
        let s = dsn.to_ascii_lowercase();
        if s.starts_with("postgres://") || s.starts_with("postgresql://") {
            Ok(DbEngine::Postgres)
        } else if s.starts_with("mysql://") {
            Ok(DbEngine::MySql)
        } else if s.starts_with("sqlite:") || s.starts_with("sqlite://") {
            Ok(DbEngine::Sqlite)
        } else {
            bail!("Unknown DSN: {dsn}");
        }
    }

    /// Connect and build handle.
    pub async fn connect(dsn: &str, opts: ConnectOpts) -> Result<Self> {
        let engine = Self::detect(dsn)?;
        match engine {
            #[cfg(feature = "pg")]
            DbEngine::Postgres => {
                let mut o = PgPoolOptions::new();
                if let Some(n) = opts.max_conns {
                    o = o.max_connections(n);
                }
                if let Some(t) = opts.acquire_timeout {
                    o = o.acquire_timeout(t);
                }
                let pool = o.connect(dsn).await.context("connect postgres")?;
                #[cfg(feature = "sea-orm")]
                let sea = SqlxPostgresConnector::from_sqlx_postgres_pool(pool.clone());
                Ok(Self {
                    engine,
                    pool: DbPool::Postgres(pool),
                    dsn: dsn.to_string(),
                    #[cfg(feature = "sea-orm")]
                    sea,
                })
            }
            #[cfg(feature = "mysql")]
            DbEngine::MySql => {
                let mut o = MySqlPoolOptions::new();
                if let Some(n) = opts.max_conns {
                    o = o.max_connections(n);
                }
                if let Some(t) = opts.acquire_timeout {
                    o = o.acquire_timeout(t);
                }
                let pool = o.connect(dsn).await.context("connect mysql")?;
                #[cfg(feature = "sea-orm")]
                let sea = SqlxMySqlConnector::from_sqlx_mysql_pool(pool.clone());
                Ok(Self {
                    engine,
                    pool: DbPool::MySql(pool),
                    dsn: dsn.to_string(),
                    #[cfg(feature = "sea-orm")]
                    sea,
                })
            }
            #[cfg(feature = "sqlite")]
            DbEngine::Sqlite => {
                let dsn = prepare_sqlite_path(dsn, opts.create_sqlite_dirs)?;
                let mut o = SqlitePoolOptions::new();
                if let Some(n) = opts.max_conns {
                    o = o.max_connections(n);
                }
                if let Some(t) = opts.acquire_timeout {
                    o = o.acquire_timeout(t);
                }
                let pool = o.connect(&dsn).await.context("connect sqlite")?;
                apply_sqlite_pragmas(&pool, opts.sqlite_busy_timeout).await?;
                #[cfg(feature = "sea-orm")]
                let sea = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool.clone());
                Ok(Self {
                    engine,
                    pool: DbPool::Sqlite(pool),
                    dsn: dsn.to_string(),
                    #[cfg(feature = "sea-orm")]
                    sea,
                })
            }
            #[cfg(not(feature = "pg"))]
            DbEngine::Postgres => bail!("PostgreSQL feature not enabled"),
            #[cfg(not(feature = "mysql"))]
            DbEngine::MySql => bail!("MySQL feature not enabled"),
            #[cfg(not(any(feature = "pg", feature = "mysql", feature = "sqlite")))]
            _ => bail!("No DB features enabled"),
        }
    }

    /// Graceful pool close. (Dropping the pool also closes it; this just makes it explicit.)
    pub async fn close(self) {
        match self.pool {
            #[cfg(feature = "pg")]
            DbPool::Postgres(p) => p.close().await,
            #[cfg(feature = "mysql")]
            DbPool::MySql(p) => p.close().await,
            #[cfg(feature = "sqlite")]
            DbPool::Sqlite(p) => p.close().await,
        }
    }

    /// Get the backend.
    pub fn engine(&self) -> DbEngine {
        self.engine
    }

    // --- sqlx accessors ---
    #[cfg(feature = "pg")]
    pub fn sqlx_postgres(&self) -> Option<&PgPool> {
        match self.pool {
            DbPool::Postgres(ref p) => Some(p),
            _ => None,
        }
    }
    #[cfg(feature = "mysql")]
    pub fn sqlx_mysql(&self) -> Option<&MySqlPool> {
        match self.pool {
            DbPool::MySql(ref p) => Some(p),
            _ => None,
        }
    }
    #[cfg(feature = "sqlite")]
    pub fn sqlx_sqlite(&self) -> Option<&SqlitePool> {
        match self.pool {
            DbPool::Sqlite(ref p) => Some(p),
            _ => None,
        }
    }

    // --- SeaORM accessor ---
    #[cfg(feature = "sea-orm")]
    /// Get SeaORM connection (clone; cheap handle).
    pub fn sea(&self) -> DatabaseConnection {
        self.sea.clone()
    }

    #[cfg(feature = "sea-orm")]
    /// Backward-compatible alias; not async (no await inside).
    pub fn seaorm(&self) -> &DatabaseConnection {
        &self.sea
    }

    // --- Transaction helpers (engine-specific) ---
    #[cfg(feature = "pg")]
    pub async fn with_pg_tx<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut sqlx::Transaction<'_, Postgres>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let pool = self.sqlx_postgres().context("not a postgres pool")?;
        let mut tx = pool.begin().await?;
        let out = f(&mut tx).await?;
        tx.commit().await?;
        Ok(out)
    }

    #[cfg(feature = "mysql")]
    pub async fn with_mysql_tx<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut sqlx::Transaction<'_, MySql>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let pool = self.sqlx_mysql().context("not a mysql pool")?;
        let mut tx = pool.begin().await?;
        let out = f(&mut tx).await?;
        tx.commit().await?;
        Ok(out)
    }

    #[cfg(feature = "sqlite")]
    pub async fn with_sqlite_tx<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut sqlx::Transaction<'_, Sqlite>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let pool = self.sqlx_sqlite().context("not a sqlite pool")?;
        let mut tx = pool.begin().await?;
        let out = f(&mut tx).await?;
        tx.commit().await?;
        Ok(out)
    }

    // --- Advisory locks ---

    /// Acquire an advisory lock with the given key and module namespace.
    pub async fn lock(&self, module: &str, key: &str) -> Result<DbLockGuard> {
        let lock_manager =
            advisory_locks::LockManager::new(self.engine, self.pool.clone(), self.dsn.clone());
        lock_manager.lock(module, key).await
    }

    /// Try to acquire an advisory lock with configurable retry/backoff policy.
    pub async fn try_lock(
        &self,
        module: &str,
        key: &str,
        config: LockConfig,
    ) -> Result<Option<DbLockGuard>> {
        let lock_manager =
            advisory_locks::LockManager::new(self.engine, self.pool.clone(), self.dsn.clone());
        lock_manager.try_lock(module, key, config).await
    }

    // --- Generic transaction begin (returns proper enum with lifetime) ---

    /// Begin a transaction (returns appropriate transaction type based on backend).
    pub async fn begin<'a>(&'a self) -> Result<DbTransaction<'a>> {
        match &self.pool {
            #[cfg(feature = "pg")]
            DbPool::Postgres(pool) => {
                let tx = pool.begin().await?;
                Ok(DbTransaction::Postgres(tx))
            }
            #[cfg(feature = "mysql")]
            DbPool::MySql(pool) => {
                let tx = pool.begin().await?;
                Ok(DbTransaction::MySql(tx))
            }
            #[cfg(feature = "sqlite")]
            DbPool::Sqlite(pool) => {
                let tx = pool.begin().await?;
                Ok(DbTransaction::Sqlite(tx))
            }
            #[cfg(not(any(feature = "pg", feature = "mysql", feature = "sqlite")))]
            _ => unreachable!("No database features enabled"),
        }
    }
}

// ===================== helpers =====================

#[cfg(feature = "sqlite")]
async fn apply_sqlite_pragmas(pool: &SqlitePool, busy: Option<Duration>) -> Result<()> {
    // Sane defaults for app DBs; adjust to your needs.
    sqlx::query("PRAGMA journal_mode=WAL").execute(pool).await?;
    sqlx::query("PRAGMA synchronous=NORMAL")
        .execute(pool)
        .await?;
    if let Some(ms) = busy {
        // busy_timeout expects integer milliseconds.
        sqlx::query(&format!("PRAGMA busy_timeout={}", ms.as_millis()))
            .execute(pool)
            .await?;
    }
    Ok(())
}

#[cfg(feature = "sqlite")]
fn prepare_sqlite_path(dsn: &str, create_dirs: bool) -> Result<String> {
    // Only try to create directories for plain file paths; ignore :memory: cases.
    if !create_dirs || dsn.contains(":memory:") {
        return Ok(dsn.to_string());
    }

    // This is a pragmatic parser: it handles "sqlite:/path" and "sqlite://path".
    // For URI forms like "sqlite:file:memdb?..." there is no filesystem dir to create.
    let raw = if let Some(rest) = dsn.strip_prefix("sqlite://") {
        rest
    } else if let Some(rest) = dsn.strip_prefix("sqlite:") {
        rest
    } else {
        dsn
    };

    // If DSN looks like a plain path (no "file:" scheme or query), create parent dir.
    if !raw.starts_with("file:") && !raw.contains('?') {
        if let Some(parent) = std::path::Path::new(raw).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create dir for sqlite: {parent:?}"))?;
            }
        }
    }

    Ok(dsn.to_string())
}

// ===================== tests =====================

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Duration;

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_sqlite_connection() -> Result<()> {
        let dsn = "sqlite::memory:";
        let opts = ConnectOpts::default();
        let db = DbHandle::connect(dsn, opts).await?;
        assert_eq!(db.engine(), DbEngine::Sqlite);
        Ok(())
    }

    #[tokio::test]
    async fn test_backend_detection() {
        assert_eq!(
            DbHandle::detect("sqlite://test.db").unwrap(),
            DbEngine::Sqlite
        );
        assert_eq!(
            DbHandle::detect("postgres://localhost/test").unwrap(),
            DbEngine::Postgres
        );
        assert_eq!(
            DbHandle::detect("mysql://localhost/test").unwrap(),
            DbEngine::MySql
        );
        assert!(DbHandle::detect("unknown://test").is_err());
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_advisory_lock_sqlite() -> Result<()> {
        let dsn = "sqlite:file:memdb1?mode=memory&cache=shared";
        let db = DbHandle::connect(dsn, ConnectOpts::default()).await?;

        let test_id = format!(
            "test_basic_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        let guard1 = db.lock("test_module", &format!("{}_key1", test_id)).await?;
        let _guard2 = db.lock("test_module", &format!("{}_key2", test_id)).await?;
        let _guard3 = db
            .lock("different_module", &format!("{}_key1", test_id))
            .await?;

        // Deterministic unlock to avoid races with async Drop cleanup
        guard1.release().await;
        let _guard4 = db.lock("test_module", &format!("{}_key1", test_id)).await?;
        Ok(())
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_advisory_lock_different_keys() -> Result<()> {
        let dsn = "sqlite:file:memdb_diff_keys?mode=memory&cache=shared";
        let db = DbHandle::connect(dsn, ConnectOpts::default()).await?;

        let test_id = format!(
            "test_diff_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        let _guard1 = db.lock("test_module", &format!("{}_key1", test_id)).await?;
        let _guard2 = db.lock("test_module", &format!("{}_key2", test_id)).await?;
        let _guard3 = db
            .lock("other_module", &format!("{}_key1", test_id))
            .await?;
        Ok(())
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_try_lock_with_config() -> Result<()> {
        let dsn = "sqlite:file:memdb2?mode=memory&cache=shared";
        let db = DbHandle::connect(dsn, ConnectOpts::default()).await?;

        let test_id = format!(
            "test_config_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        let _guard1 = db.lock("test_module", &format!("{}_key", test_id)).await?;

        let config = LockConfig {
            max_wait: Some(Duration::from_millis(200)),
            initial_backoff: Duration::from_millis(50),
            max_attempts: Some(3),
            ..Default::default()
        };

        let result = db
            .try_lock("test_module", &format!("{}_different_key", test_id), config)
            .await?;
        assert!(
            result.is_some(),
            "expected lock acquisition for different key"
        );
        Ok(())
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_transaction() -> Result<()> {
        let dsn = "sqlite::memory:";
        let db = DbHandle::connect(dsn, ConnectOpts::default()).await?;
        let tx = db.begin().await?;
        tx.commit().await?;
        Ok(())
    }

    #[cfg(all(feature = "sea-orm", feature = "sqlite"))]
    #[tokio::test]
    async fn test_seaorm_connection() -> Result<()> {
        let dsn = "sqlite::memory:";
        let db = DbHandle::connect(dsn, ConnectOpts::default()).await?;
        let _conn = db.sea();
        Ok(())
    }
}
