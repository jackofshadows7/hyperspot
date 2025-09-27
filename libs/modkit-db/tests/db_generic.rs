#![cfg(feature = "integration")]

mod common;
use anyhow::Result;
#[cfg(feature = "pg")]
use sqlx::Row;

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn generic_sqlite() -> Result<()> {
    let dut = common::bring_up_sqlite().await?;
    run_common_suite(&dut.url).await
}

#[cfg(feature = "pg")]
#[tokio::test]
async fn generic_postgres() -> Result<()> {
    let dut = common::bring_up_postgres().await?;
    run_common_suite(&dut.url).await
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn generic_mysql() -> Result<()> {
    let dut = common::bring_up_mysql().await?;
    run_common_suite(&dut.url).await
}

/// Runs the same assertions for any backend.
/// Tests basic DbHandle functionality without requiring migrations.
async fn run_common_suite(database_url: &str) -> Result<()> {
    // Test basic connection
    let db = modkit_db::DbHandle::connect(database_url, modkit_db::ConnectOpts::default()).await?;

    // Verify engine detection
    let engine = modkit_db::DbHandle::detect(database_url)?;
    assert_eq!(db.engine(), engine);

    // Test DSN redaction (should not panic)
    let redacted = modkit_db::redact_credentials_in_dsn(Some(database_url));
    assert!(!redacted.contains("pass"));

    // Test basic SQL execution based on engine
    match engine {
        #[cfg(feature = "pg")]
        modkit_db::DbEngine::Postgres => {
            let pool = db.sqlx_postgres().unwrap();
            // Create a simple test table and verify
            sqlx::query("CREATE TABLE IF NOT EXISTS test_pg (id SERIAL PRIMARY KEY, name TEXT)")
                .execute(pool)
                .await?;

            let result = sqlx::query("INSERT INTO test_pg (name) VALUES ($1) RETURNING id")
                .bind("test_user")
                .fetch_one(pool)
                .await?;

            let id: i32 = result.get("id");
            assert!(id > 0);

            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM test_pg")
                .fetch_one(pool)
                .await?;
            assert_eq!(count, 1);
        }
        #[cfg(feature = "mysql")]
        modkit_db::DbEngine::MySql => {
            let pool = db.sqlx_mysql().unwrap();
            // Create a simple test table and verify
            sqlx::query("CREATE TABLE IF NOT EXISTS test_mysql (id INT AUTO_INCREMENT PRIMARY KEY, name TEXT)")
                .execute(pool)
                .await?;

            sqlx::query("INSERT INTO test_mysql (name) VALUES (?)")
                .bind("test_user")
                .execute(pool)
                .await?;

            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM test_mysql")
                .fetch_one(pool)
                .await?;
            assert_eq!(count, 1);
        }
        #[cfg(feature = "sqlite")]
        modkit_db::DbEngine::Sqlite => {
            let pool = db.sqlx_sqlite().unwrap();
            // Create a simple test table and verify
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS test_sqlite (id INTEGER PRIMARY KEY, name TEXT)",
            )
            .execute(pool)
            .await?;

            sqlx::query("INSERT INTO test_sqlite (name) VALUES (?)")
                .bind("test_user")
                .execute(pool)
                .await?;

            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM test_sqlite")
                .fetch_one(pool)
                .await?;
            assert_eq!(count, 1);
        }
        #[cfg(not(all(feature = "pg", feature = "mysql", feature = "sqlite")))]
        _ => {
            anyhow::bail!("Unsupported engine: {:?}", engine);
        }
    }

    // Test transaction functionality - temporarily disabled due to lifetime issues
    // test_transaction(&db).await?;

    #[cfg(feature = "sea-orm")]
    {
        // Test SeaORM connection
        let _sea = db.sea();
        // Basic ping to verify connection works
        db.seaorm()
            .ping()
            .await
            .map_err(|e| anyhow::anyhow!("SeaORM ping failed: {}", e))?;
    }

    // Clean up
    db.close().await;
    Ok(())
}

// Transaction tests temporarily disabled due to lifetime issues with async closures
// async fn test_transaction(db: &modkit_db::DbHandle) -> Result<()> {
//     // Implementation would go here
//     Ok(())
// }
