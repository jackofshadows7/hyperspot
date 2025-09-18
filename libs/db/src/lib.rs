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
//! async fn main() -> db::Result<()> {
//!     use db::{DbHandle, ConnectOpts};
//!
//!     let db = DbHandle::connect("postgres://user:pass@localhost/app", ConnectOpts::default()).await?;
//!
//!     // sqlx
//!     #[cfg(feature="pg")]
//!     {
//!         let pool = db.sqlx_postgres().unwrap();
//!         // Help type inference for doctests: specify database type explicitly
//!         sqlx::query::<sqlx::Postgres>("select 1").execute(pool).await?;
//!
//!         // Execute within a dedicated connection (doctest-friendly)
//!         let mut conn = pool.acquire().await?;
//!         sqlx::query::<sqlx::Postgres>("select 2").execute(&mut *conn).await?;
//!     }
//!
//!     #[cfg(feature="mysql")]
//!     {
//!         let pool = db.sqlx_mysql().unwrap();
//!         sqlx::query::<sqlx::MySql>("select 1").execute(pool).await?;
//!
//!         let mut conn = pool.acquire().await?;
//!         sqlx::query::<sqlx::MySql>("select 2").execute(&mut *conn).await?;
//!     }
//!
//!     #[cfg(feature="sqlite")]
//!     {
//!         let pool = db.sqlx_sqlite().unwrap();
//!         sqlx::query::<sqlx::Sqlite>("select 1").execute(pool).await?;
//!
//!         let mut conn = pool.acquire().await?;
//!         sqlx::query::<sqlx::Sqlite>("select 2").execute(&mut *conn).await?;
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

// Re-export key types for public API
pub use advisory_locks::{DbLockGuard, LockConfig};
// Advisory locks module
pub mod advisory_locks;
pub mod odata;

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

use thiserror::Error;

/// Library-local result type.
pub type Result<T> = std::result::Result<T, DbError>;

/// Typed error for the DB handle and helpers.
#[derive(Debug, Error)]
pub enum DbError {
    #[error("Unknown DSN: {0}")]
    UnknownDsn(String),

    #[error("Feature not enabled: {0}")]
    FeatureDisabled(&'static str),

    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    #[cfg(feature = "sea-orm")]
    #[error(transparent)]
    Sea(#[from] sea_orm::DbErr),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("SQLite pragma error: {0}")]
    SqlitePragma(String),

    // make advisory_locks errors flow into DbError via `?`
    #[error(transparent)]
    Lock(#[from] advisory_locks::DbLockError),
}

/// Supported engines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbEngine {
    Postgres,
    MySql,
    Sqlite,
}

/// Connection options.
/// Extended to cover common sqlx pool knobs; each driver applies the subset it supports.
#[derive(Clone, Debug)]
pub struct ConnectOpts {
    /// Maximum number of connections in the pool.
    pub max_conns: Option<u32>,
    /// Minimum number of connections in the pool.
    pub min_conns: Option<u32>,
    /// Timeout to acquire a connection from the pool.
    pub acquire_timeout: Option<Duration>,
    /// Idle timeout before a connection is closed.
    pub idle_timeout: Option<Duration>,
    /// Maximum lifetime for a connection.
    pub max_lifetime: Option<Duration>,
    /// Test connection health before acquire.
    pub test_before_acquire: bool,

    /// SQLite-specific: busy timeout used via PRAGMA busy_timeout.
    pub sqlite_busy_timeout: Option<Duration>,
    /// For SQLite file DSNs, create parent directories if missing.
    pub create_sqlite_dirs: bool,
}
impl Default for ConnectOpts {
    fn default() -> Self {
        Self {
            max_conns: Some(10),
            min_conns: None,
            acquire_timeout: Some(Duration::from_secs(30)),
            idle_timeout: None,
            max_lifetime: None,
            test_before_acquire: false,

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
    ///
    /// Note: we only check scheme prefixes and don't mutate the tail (credentials etc.).
    pub fn detect(dsn: &str) -> Result<DbEngine> {
        // Trim only leading spaces/newlines to be forgiving with env files.
        let s = dsn.trim_start();

        // Explicit, case-sensitive checks for common schemes.
        // Add more variants as needed (e.g., postgres+unix://).
        if s.starts_with("postgres://") || s.starts_with("postgresql://") {
            Ok(DbEngine::Postgres)
        } else if s.starts_with("mysql://") {
            Ok(DbEngine::MySql)
        } else if s.starts_with("sqlite:") || s.starts_with("sqlite://") {
            Ok(DbEngine::Sqlite)
        } else {
            Err(DbError::UnknownDsn(dsn.to_string()))
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
                if let Some(n) = opts.min_conns {
                    o = o.min_connections(n);
                }
                if let Some(t) = opts.acquire_timeout {
                    o = o.acquire_timeout(t);
                }
                if let Some(t) = opts.idle_timeout {
                    o = o.idle_timeout(t);
                }
                if let Some(t) = opts.max_lifetime {
                    o = o.max_lifetime(t);
                }
                if opts.test_before_acquire {
                    o = o.test_before_acquire(true);
                }
                let pool = o.connect(dsn).await?;
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
                if let Some(n) = opts.min_conns {
                    o = o.min_connections(n);
                }
                if let Some(t) = opts.acquire_timeout {
                    o = o.acquire_timeout(t);
                }
                if let Some(t) = opts.idle_timeout {
                    o = o.idle_timeout(t);
                }
                if let Some(t) = opts.max_lifetime {
                    o = o.max_lifetime(t);
                }
                if opts.test_before_acquire {
                    o = o.test_before_acquire(true);
                }
                let pool = o.connect(dsn).await?;
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
                if let Some(n) = opts.min_conns {
                    o = o.min_connections(n);
                }
                if let Some(t) = opts.acquire_timeout {
                    o = o.acquire_timeout(t);
                }
                if let Some(t) = opts.idle_timeout {
                    o = o.idle_timeout(t);
                }
                if let Some(t) = opts.max_lifetime {
                    o = o.max_lifetime(t);
                }
                if opts.test_before_acquire {
                    o = o.test_before_acquire(true);
                }

                // Copy busy timeout into the closure (per-connection PRAGMAs)
                let busy = opts.sqlite_busy_timeout;
                o = o.after_connect(move |conn, _meta| {
                    let busy = busy;
                    Box::pin(async move {
                        // Each call borrows `conn` mutably for the duration of the await,
                        // without moving the &mut SqliteConnection itself.
                        sqlx::query("PRAGMA journal_mode = WAL")
                            .execute(&mut *conn)
                            .await?;

                        sqlx::query("PRAGMA synchronous = NORMAL")
                            .execute(&mut *conn)
                            .await?;

                        if let Some(ms) = busy {
                            // PRAGMA can't use bind parameters; use a numeric literal.
                            let ms = std::cmp::min(ms.as_millis(), i64::MAX as u128) as i64;
                            let stmt = format!("PRAGMA busy_timeout = {ms}");
                            sqlx::query(&stmt).execute(&mut *conn).await?;
                        }

                        Ok(())
                    })
                });

                let pool = o.connect(&dsn).await?;
                // No extra call to apply_sqlite_pragmas here anymore.
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
            DbEngine::Postgres => Err(DbError::FeatureDisabled("PostgreSQL feature not enabled")),
            #[cfg(not(feature = "mysql"))]
            DbEngine::MySql => Err(DbError::FeatureDisabled("MySQL feature not enabled")),
            #[cfg(not(any(feature = "pg", feature = "mysql", feature = "sqlite")))]
            _ => Err(DbError::FeatureDisabled("no DB features enabled")),
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
            #[cfg(any(feature = "mysql", feature = "sqlite"))]
            _ => None,
        }
    }
    #[cfg(feature = "mysql")]
    pub fn sqlx_mysql(&self) -> Option<&MySqlPool> {
        match self.pool {
            DbPool::MySql(ref p) => Some(p),
            #[cfg(any(feature = "pg", feature = "sqlite"))]
            _ => None,
        }
    }
    #[cfg(feature = "sqlite")]
    pub fn sqlx_sqlite(&self) -> Option<&SqlitePool> {
        match self.pool {
            DbPool::Sqlite(ref p) => Some(p),
            #[cfg(any(feature = "pg", feature = "mysql"))]
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
        let pool = self
            .sqlx_postgres()
            .ok_or(DbError::FeatureDisabled("not a postgres pool"))?;
        let mut tx = pool.begin().await?;
        let res = f(&mut tx).await;
        match res {
            Ok(v) => {
                tx.commit().await?;
                Ok(v)
            }
            Err(e) => {
                // Best-effort rollback; keep the original error.
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    #[cfg(feature = "mysql")]
    pub async fn with_mysql_tx<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut sqlx::Transaction<'_, MySql>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let pool = self
            .sqlx_mysql()
            .ok_or(DbError::FeatureDisabled("not a mysql pool"))?;
        let mut tx = pool.begin().await?;
        let res = f(&mut tx).await;
        match res {
            Ok(v) => {
                tx.commit().await?;
                Ok(v)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    #[cfg(feature = "sqlite")]
    pub async fn with_sqlite_tx<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut sqlx::Transaction<'_, Sqlite>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let pool = self
            .sqlx_sqlite()
            .ok_or(DbError::FeatureDisabled("not a sqlite pool"))?;
        let mut tx = pool.begin().await?;
        let res = f(&mut tx).await;
        match res {
            Ok(v) => {
                tx.commit().await?;
                Ok(v)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    // --- Advisory locks ---

    /// Acquire an advisory lock with the given key and module namespace.
    pub async fn lock(&self, module: &str, key: &str) -> Result<DbLockGuard> {
        let lock_manager =
            advisory_locks::LockManager::new(self.engine, self.pool.clone(), self.dsn.clone());
        let guard = lock_manager.lock(module, key).await?;
        Ok(guard)
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
        let res = lock_manager.try_lock(module, key, config).await?;
        Ok(res)
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
            _ => Err(DbError::FeatureDisabled("no database backends enabled")),
        }
    }
}

// ===================== helpers =====================

#[cfg(feature = "sqlite")]
#[allow(dead_code)]
async fn apply_sqlite_pragmas(pool: &SqlitePool, busy: Option<Duration>) -> Result<()> {
    // Sane defaults for app DBs; adjust to your needs.
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(pool)
        .await?;
    sqlx::query("PRAGMA synchronous = NORMAL")
        .execute(pool)
        .await?;
    if let Some(ms) = busy {
        // Prefer bound parameter; ensure type fits into i64.
        let ms = i64::try_from(ms.as_millis()).unwrap_or(i64::MAX);
        sqlx::query("PRAGMA busy_timeout = ?")
            .bind(ms)
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
                // One-time blocking call during startup; acceptable for setup paths.
                std::fs::create_dir_all(parent)?;
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

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let test_id = format!("test_basic_{now}");

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

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let test_id = format!("test_diff_{now}");

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

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let test_id = format!("test_config_{now}");

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
