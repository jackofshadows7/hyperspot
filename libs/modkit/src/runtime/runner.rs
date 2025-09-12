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

/// Per-module database factory - takes module name and returns a database handle for that module.
pub type PerModuleDbFactory = Box<
    dyn Fn(&str) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<Arc<db::DbHandle>>>> + Send>>
        + Send
        + Sync,
>;

/// The way to get a DB handle.
pub enum DbOptions {
    /// Do not open a DB.
    None,
    /// A ready handle from the caller (legacy, single DB for all modules).
    Existing(Arc<db::DbHandle>),
    /// Call a factory (closure can close anything: AppConfig, env, etc.) (legacy, single DB for all modules).
    Auto(DbFactory),
    /// Per-module database factory - will be called for each module during initialization.
    PerModuleFactory(PerModuleDbFactory),
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

    // Initialize databases based on the chosen strategy.
    let (legacy_db_handle, per_module_factory) = match &opts.db {
        DbOptions::None => (None, None),
        DbOptions::Existing(h) => (Some(h.clone()), None),
        DbOptions::Auto(factory) => (Some((factory)().await?), None),
        DbOptions::PerModuleFactory(factory) => {
            tracing::info!("Using per-module database factory");
            (None, Some(factory))
        }
    };

    // Discover modules and strict order of phases.
    let registry = crate::registry::ModuleRegistry::discover_and_build()?;

    // Build base context and per-module contexts
    let base_ctx = ModuleCtxBuilder::new(cancel.clone())
        .with_client_hub(hub.clone())
        .with_config_provider(opts.modules_cfg.clone())
        .build();

    tracing::info!("Phase: init");
    if let Some(factory) = &per_module_factory {
        // Per-module mode: create contexts with individual DB handles using factory
        registry
            .run_init_phase_with_factory(&base_ctx, factory)
            .await?;
    } else {
        // Legacy mode: use shared DB handle for all modules
        let mut b = ModuleCtxBuilder::new(cancel.clone())
            .with_client_hub(hub.clone())
            .with_config_provider(opts.modules_cfg.clone());
        if let Some(db) = &legacy_db_handle {
            b = b.with_db(db.clone());
        }
        let shared_ctx = b.build();
        registry.run_init_phase(&shared_ctx).await?;
    }

    // DB migration phase
    if let Some(db) = &legacy_db_handle {
        tracing::info!("Phase: db (legacy single DB)");
        registry.run_db_phase(db).await?;
    } else if let Some(factory) = &per_module_factory {
        tracing::info!("Phase: db (per-module)");
        registry.run_db_phase_with_factory(factory).await?;
    }

    tracing::info!("Phase: rest (sync)");
    let _app = if let Some(factory) = &per_module_factory {
        // Per-module: use factory for REST phase
        registry.run_rest_phase_with_factory(&base_ctx, factory, axum::Router::new())?
    } else {
        // Legacy: shared context
        let mut b = ModuleCtxBuilder::new(cancel.clone())
            .with_client_hub(hub.clone())
            .with_config_provider(opts.modules_cfg.clone());
        if let Some(db) = &legacy_db_handle {
            b = b.with_db(db.clone());
        }
        let context_for_rest = b.build();
        registry.run_rest_phase(&context_for_rest, axum::Router::new())?
    };

    tracing::info!("Phase: start");
    registry.run_start_phase(cancel.clone()).await?;

    // Wait for cancellation (signals/token/future).
    cancel.cancelled().await;

    tracing::info!("Phase: stop");
    registry.run_stop_phase(cancel).await?;
    Ok(())
}

// Removed per-module database initialization functions that depend on runtime types.
// Per-module database handling is now done via PerModuleDbFactory in the main application.

#[cfg(feature = "hs-runtime")]
#[allow(dead_code)]
pub async fn run_with_hyperspot_signals(mut opts: RunOptions) -> anyhow::Result<()> {
    opts.shutdown = ShutdownOptions::Signals;
    run(opts).await
}
