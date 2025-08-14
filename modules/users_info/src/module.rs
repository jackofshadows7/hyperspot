use std::sync::Arc;

use async_trait::async_trait;
use modkit::api::OpenApiRegistry;
use modkit::{DbModule, Module, ModuleCtx, RestfulModule};
use sea_orm_migration::MigratorTrait;
use tracing::{debug, info};

use crate::api::rest::routes;
use crate::config::UsersInfoConfig;
use crate::contract::client::UsersInfoApi;
use crate::domain::service::{Service, ServiceConfig};
use crate::gateways::local::UsersInfoLocalClient;

/// Main module struct with DDD-light layout and proper ClientHub integration
#[modkit::module(
    name = "users_info",
    caps = [db, rest],
    client = crate::contract::client::UsersInfoApi
)]
#[derive(Default)]
pub struct UsersInfo {
    service: arc_swap::ArcSwapOption<Service>,
}

impl Clone for UsersInfo {
    fn clone(&self) -> Self {
        Self {
            service: arc_swap::ArcSwapOption::new(self.service.load().as_ref().map(|s| s.clone())),
        }
    }
}

#[async_trait]
impl Module for UsersInfo {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        info!("Initializing users_info module");

        // Load module configuration
        let cfg: UsersInfoConfig = ctx.module_config();
        debug!(
            "Loaded users_info config: default_page_size={}",
            cfg.default_page_size
        );

        // Get database connection
        let db = ctx.db().ok_or_else(|| anyhow::anyhow!("DB required"))?;
        let db_conn = db.seaorm();

        // Create domain service with configuration
        let service_config = ServiceConfig {
            max_display_name_length: 100,
            default_page_size: cfg.default_page_size,
            max_page_size: cfg.max_page_size,
        };

        let service = Service::new(db_conn.clone(), service_config);
        self.service.store(Some(Arc::new(service.clone())));

        // Create and register the local client implementation
        let api: Arc<dyn UsersInfoApi> = Arc::new(UsersInfoLocalClient::new(Arc::new(service)));
        expose_users_info_client(ctx, &api)?;

        info!("UsersInfo API exposed to ClientHub");
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait]
impl DbModule for UsersInfo {
    async fn migrate(&self, db: &db::DbHandle) -> anyhow::Result<()> {
        info!("Running users_info database migrations");

        let conn = db.seaorm();
        crate::infra::storage::migrations::Migrator::up(conn, None).await?;

        info!("Users database migrations completed successfully");
        Ok(())
    }
}

impl RestfulModule for UsersInfo {
    fn register_rest(
        &self,
        _ctx: &ModuleCtx,
        router: axum::Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        info!("Registering users_info REST routes");

        let service = self
            .service
            .load()
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Service not initialized"))?
            .clone();

        let router = routes::register_routes(router, openapi, service)?;

        info!("Users REST routes registered successfully");
        Ok(router)
    }
}
