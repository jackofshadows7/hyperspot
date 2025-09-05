use crate::context::{ConfigProvider, ModuleCtxBuilder};
use crate::runtime::shutdown;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio_util::sync::CancellationToken;

/// Async factory for auto-initializing the database (the caller decides where to obtain
/// configuration/settings from).
pub type DbFactory = Box<
    dyn Fn() -> Pin<Box<dyn Future<Output = anyhow::Result<Arc<db::DbHandle>>> + Send>>
        + Send
        + Sync,
>;

/// The way to get a DB handle.
pub enum DbOptions {
    /// Do not open a DB.
    None,
    /// A ready handle from the caller.
    Existing(Arc<db::DbHandle>),
    /// Call a factory (closure can close anything: AppConfig, env, etc.).
    Auto(DbFactory),
}

/// The way to decide when to stop.
pub enum ShutdownOptions {
    /// Wait for system signals (Ctrl+C / SIGTERM).
    Signals,
    /// External CancellationToken.
    Token(CancellationToken),
    /// Arbitrary future, when it completes, we initiate shutdown.
    Future(Pin<Box<dyn Future<Output = ()> + Send>>),
}

/// Options for running ModKit runner.
pub struct RunOptions {
    /// Provider of module config sections (raw JSON by module name).
    pub modules_cfg: Arc<dyn ConfigProvider>,
    /// DB initialization strategy.
    pub db: DbOptions,
    /// Shutdown strategy.
    pub shutdown: ShutdownOptions,
}

/// Full cycle: init + DB + REST (sync) + start + wait + stop.
/// Full cycle: init → DB → REST (sync) → start → wait → stop.
pub async fn run(opts: RunOptions) -> anyhow::Result<()> {
    // Initialize DB by the chosen strategy.
    let db_handle = match &opts.db {
        DbOptions::None => None,
        DbOptions::Existing(h) => Some(h.clone()),
        DbOptions::Auto(factory) => Some((factory)().await?),
    };

    // Common objects: ClientHub and CancellationToken.
    let hub = Arc::new(crate::client_hub::ClientHub::default());
    let cancel = match &opts.shutdown {
        ShutdownOptions::Token(t) => t.clone(),
        _ => CancellationToken::new(),
    };

    // Background shutdown waiters (cross-platform, safe)
    match opts.shutdown {
        ShutdownOptions::Signals => {
            let c = cancel.clone();
            tokio::spawn(async move {
                match shutdown::wait_for_shutdown().await {
                    Ok(()) => {
                        tracing::info!("shutdown: signal received");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "shutdown: primary waiter failed; falling back to ctrl_c()");
                        // Fallback that works everywhere, incl. Windows
                        let _ = tokio::signal::ctrl_c().await;
                    }
                }
                c.cancel();
            });
        }
        ShutdownOptions::Future(waiter) => {
            let c = cancel.clone();
            tokio::spawn(async move {
                waiter.await;
                tracing::info!("shutdown: external future completed");
                c.cancel();
            });
        }
        ShutdownOptions::Token(_) => {
            // External owner will call cancel(); just log for clarity
            tracing::info!("shutdown: external token will control lifecycle");
        }
    }

    // Build the base ModuleCtx (builder — crate-private; outward — read-only API).
    let mut b = ModuleCtxBuilder::new(cancel.clone())
        .with_client_hub(hub.clone())
        .with_config_provider(opts.modules_cfg.clone());
    if let Some(db) = &db_handle {
        b = b.with_db(db.clone());
    }
    let base_ctx = b.build();

    // Discover modules and strict order of phases.
    let registry = crate::registry::ModuleRegistry::discover_and_build()?;

    tracing::info!("Phase: init");
    registry.run_init_phase(&base_ctx).await?;

    if let Some(db) = &db_handle {
        tracing::info!("Phase: db");
        registry.run_db_phase(db).await?;
    }

    tracing::info!("Phase: rest (sync)");
    let _app = registry.run_rest_phase(&base_ctx, axum::Router::new())?;

    tracing::info!("Phase: start");
    registry.run_start_phase(cancel.clone()).await?;

    // Wait for cancellation (signals/token/future).
    cancel.cancelled().await;

    tracing::info!("Phase: stop");
    registry.run_stop_phase(cancel).await?;
    Ok(())
}

#[cfg(feature = "hs-runtime")]
pub async fn run_with_hyperspot_signals(mut opts: RunOptions) -> anyhow::Result<()> {
    opts.shutdown = ShutdownOptions::Signals;
    run(opts).await
}
