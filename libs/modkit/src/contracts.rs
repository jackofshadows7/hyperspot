use async_trait::async_trait;
use axum::Router;
use tokio_util::sync::CancellationToken;

pub use crate::api::OpenApiRegistry;

/// Core module: DI/wiring; do not rely on migrated schema here.
#[async_trait]
pub trait Module: Send + Sync + 'static {
    async fn init(&self, ctx: &crate::context::ModuleCtx) -> anyhow::Result<()>;
    fn as_any(&self) -> &dyn std::any::Any;
}

#[async_trait]
pub trait DbModule: Send + Sync {
    /// Runs AFTER init, BEFORE REST/start.
    async fn migrate(&self, db: &modkit_db::DbHandle) -> anyhow::Result<()>;
}

/// Pure wiring; must be sync. Runs AFTER DB migrations.
pub trait RestfulModule: Send + Sync {
    fn register_rest(
        &self,
        ctx: &crate::context::ModuleCtx,
        router: Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<Router>;
}

/// REST host module: handles ingress hosting with prepare/finalize phases.
/// Must be sync. Runs during REST phase, but doesn't start the server.
#[allow(dead_code)]
pub trait RestHostModule: Send + Sync + 'static {
    /// Prepare a base Router (e.g., global middlewares, /healthz) and optionally touch OpenAPI meta.
    /// Do NOT start the server here.
    fn rest_prepare(
        &self,
        ctx: &crate::context::ModuleCtx,
        router: Router,
    ) -> anyhow::Result<Router>;

    /// Finalize before start: attach /openapi.json, /docs, persist the Router internally if needed.
    /// Do NOT start the server here.
    fn rest_finalize(
        &self,
        ctx: &crate::context::ModuleCtx,
        router: Router,
    ) -> anyhow::Result<Router>;

    // Return OpenAPI registry of the module, e.g., to register endpoints
    fn as_registry(&self) -> &dyn crate::contracts::OpenApiRegistry;
}

#[async_trait]
pub trait StatefulModule: Send + Sync {
    async fn start(&self, cancel: CancellationToken) -> anyhow::Result<()>;
    async fn stop(&self, cancel: CancellationToken) -> anyhow::Result<()>;
}
