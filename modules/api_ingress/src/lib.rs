use async_trait::async_trait;
use std::sync::Arc;

use arc_swap::ArcSwap;
use dashmap::DashMap;

use anyhow::Result;
use axum::http::Method;
use axum::{middleware::from_fn, routing::get, Router};
use modkit::api::OpenApiRegistry;
use modkit::lifecycle::ReadySignal;
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tower_http::{
    cors::CorsLayer,
    limit::RequestBodyLimitLayer,
    request_id::{PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
};

mod assets;

mod config;
pub mod error;
mod model;
mod openapi;
pub mod request_id;
mod router_cache;
mod web;

pub use config::ApiIngressConfig;
use router_cache::RouterCache;

#[cfg(test)]
pub mod example_user_module;

use model::ComponentsRegistry;

/// Standard API error response (TODO: good to register in OpenAPI once and reuse)
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ErrorResponse {
    /// Error message
    pub error: String,
    /// HTTP status code
    pub code: u16,
    /// RFC3339 timestamp when the error occurred
    pub timestamp: String,
    /// Optional request ID for tracking
    pub request_id: Option<String>,
}

impl ErrorResponse {
    /// Create a new error response
    pub fn new(error: impl Into<String>, code: u16) -> Self {
        Self {
            error: error.into(),
            code,
            timestamp: chrono::Utc::now().to_rfc3339(),
            request_id: None,
        }
    }

    /// Create an error response with request ID
    pub fn with_request_id(
        error: impl Into<String>,
        code: u16,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            error: error.into(),
            code,
            timestamp: chrono::Utc::now().to_rfc3339(),
            request_id: Some(request_id.into()),
        }
    }
}

/// Main API Ingress module — owns the HTTP server (rest_host) and collects
/// typed operation specs to emit a single OpenAPI document.
#[modkit::module(
    name = "api_ingress",
    caps = [rest_host, rest, stateful],
    lifecycle(entry = "serve", stop_timeout = "30s", await_ready)
)]
pub struct ApiIngress {
    // Lock-free config using arc-swap for read-mostly access
    config: ArcSwap<ApiIngressConfig>,
    // Lock-free components registry for read-mostly access
    components_registry: ArcSwap<ComponentsRegistry>,
    // Built router cache for zero-lock hot path access
    router_cache: RouterCache<axum::Router>,
    // Store the finalized router from REST phase for serving
    final_router: Mutex<Option<axum::Router>>,

    // Duplicate detection (per (method, path) and per handler id)
    registered_routes: DashMap<(Method, String), ()>,
    registered_handlers: DashMap<String, ()>,

    // Store operation specs for OpenAPI generation
    operation_specs: DashMap<String, modkit::api::OperationSpec>,
}

impl Default for ApiIngress {
    fn default() -> Self {
        let default_router = Router::new();
        Self {
            config: ArcSwap::from_pointee(ApiIngressConfig::default()),
            components_registry: ArcSwap::from_pointee(ComponentsRegistry::default()),
            router_cache: RouterCache::new(default_router),
            final_router: Mutex::new(None),
            registered_routes: DashMap::new(),
            registered_handlers: DashMap::new(),
            operation_specs: DashMap::new(),
        }
    }
}

impl ApiIngress {
    /// Create a new ApiIngress instance with the given configuration
    pub fn new(config: ApiIngressConfig) -> Self {
        let default_router = Router::new();
        Self {
            config: ArcSwap::from_pointee(config),
            router_cache: RouterCache::new(default_router),
            final_router: Mutex::new(None),
            operation_specs: DashMap::new(),
            ..Default::default()
        }
    }

    /// Short helper: take the last path segment of a fully-qualified Rust type name.
    /// Example: "my_crate::foo::BarDto" -> "BarDto"
    fn short_from_fqn(fqn: &str) -> &str {
        fqn.rsplit("::").next().unwrap_or(fqn)
    }

    /// Decide under which **component key** we store a RootSchema so that schemars'
    /// internal `$ref` (which uses `title` or a short name) resolves without rewrites.
    ///
    /// Priority:
    /// 1) schema.metadata.title if present & non-empty
    /// 2) short name from the provided `fqn_hint`
    fn pick_component_key(fqn_hint: &str, schema: &schemars::schema::RootSchema) -> String {
        if let Some(title) = schema
            .schema
            .metadata
            .as_ref()
            .and_then(|m| m.title.clone())
        {
            if !title.trim().is_empty() {
                return title;
            }
        }
        Self::short_from_fqn(fqn_hint).to_string()
    }

    /// Get the current configuration (cheap clone from ArcSwap)
    pub fn get_config(&self) -> ApiIngressConfig {
        (**self.config.load()).clone()
    }

    /// Get cached configuration (lock-free with ArcSwap)
    pub fn get_cached_config(&self) -> ApiIngressConfig {
        (**self.config.load()).clone()
    }

    /// Get the cached router without rebuilding (useful for performance-critical paths)
    pub fn get_cached_router(&self) -> Arc<Router> {
        self.router_cache.load()
    }

    /// Force rebuild and cache of the router
    pub async fn rebuild_and_cache_router(&self) -> Result<()> {
        let new_router = self.build_router().await?;
        self.router_cache.store(new_router);
        Ok(())
    }

    /// Build the HTTP router from registered routes and operations
    pub async fn build_router(&self) -> Result<Router> {
        // If the cached router is currently held elsewhere (e.g., by the running server),
        // return it without rebuilding to avoid unnecessary allocations.
        let cached_router = self.router_cache.load();
        if Arc::strong_count(&cached_router) > 1 {
            tracing::debug!("Using cached router");
            return Ok((*cached_router).clone());
        }

        tracing::debug!("Building new router");
        let mut router = Router::new().route("/health", get(web::health_check));

        // Correct middleware order (outermost to innermost):
        // PropagateRequestId -> SetRequestId -> push_req_id_to_extensions -> Trace -> Timeout -> CORS -> BodyLimit
        let x_request_id = crate::request_id::header();

        // 1. If client sent x-request-id, propagate it; otherwise we will set it
        router = router.layer(PropagateRequestIdLayer::new(x_request_id.clone()));

        // 2. Generate x-request-id when missing
        router = router.layer(SetRequestIdLayer::new(
            x_request_id.clone(),
            crate::request_id::MakeReqId,
        ));

        // 3. Put request_id into extensions and span
        router = router.layer(from_fn(crate::request_id::push_req_id_to_extensions));

        // 4. Trace with request_id/status/latency
        router = router.layer(crate::request_id::create_trace_layer());

        // 5. Timeout layer - 30 second timeout for handlers
        router = router.layer(TimeoutLayer::new(Duration::from_secs(30)));

        // 6. CORS layer (if enabled)
        let config = self.get_cached_config();
        if config.cors_enabled {
            router = router.layer(CorsLayer::permissive());
        }

        // 7. Body limit layer - 16MB default limit
        router = router.layer(RequestBodyLimitLayer::new(16 * 1024 * 1024));

        // Cache the built router for future use
        self.router_cache.store(router.clone());

        Ok(router)
    }

    /// Build a schema value for content type + optional component name.
    ///
    /// Strategy:
    /// - If `schema_name` provided, try exact key; if missing, try its short form.
    /// - Otherwise emit a minimal inline schema by content type (keeps UI happy).
    fn make_schema(
        components: &crate::model::ComponentsRegistry,
        content_type: &str,
        schema_name: Option<&str>,
    ) -> serde_json::Value {
        if let Some(name) = schema_name {
            if components.schemas.contains_key(name) {
                return serde_json::json!({ "$ref": format!("#/components/schemas/{name}") });
            }
            if let Some(short) = name.rsplit("::").next() {
                if components.schemas.contains_key(short) {
                    return serde_json::json!({ "$ref": format!("#/components/schemas/{short}") });
                }
            }
        }
        match content_type {
            "application/json" => serde_json::json!({ "type": "object" }),
            "text/plain" | "text/html" => serde_json::json!({ "type": "string" }),
            _ => serde_json::json!({}), // free-form
        }
    }

    /// Build the "content" object for a single media type and optional schema name.
    /// If schema_name is None, we still emit an empty schema object to satisfy OAS UI.
    fn make_content_obj(
        components: &crate::model::ComponentsRegistry,
        content_type: &str,
        schema_name: &Option<String>,
    ) -> serde_json::Value {
        let mut content_type_obj = serde_json::Map::new();
        let schema_value = Self::make_schema(components, content_type, schema_name.as_deref());
        content_type_obj.insert("schema".to_string(), schema_value);

        let mut content = serde_json::Map::new();
        content.insert(
            content_type.to_string(),
            serde_json::Value::Object(content_type_obj),
        );
        serde_json::Value::Object(content)
    }

    /// Build OpenAPI specification from registered routes and components.
    pub fn build_openapi(&self) -> Result<crate::openapi::OpenApi> {
        let components_registry = self.components_registry.load();

        // Log operation count for visibility
        let op_count = self.operation_specs.len();
        tracing::info!("Building OpenAPI: found {op_count} registered operations");

        // Build paths map from stored operation specs
        let mut paths_map: std::collections::BTreeMap<
            String,
            std::collections::BTreeMap<String, serde_json::Value>,
        > = std::collections::BTreeMap::new();

        for spec_entry in self.operation_specs.iter() {
            let spec = spec_entry.value();
            let method = spec.method.as_str().to_lowercase();
            let path = &spec.path;

            // Create operation object
            let mut operation = serde_json::Map::new();

            // Prefer explicit operation_id, fallback to handler_id
            let op_id = spec
                .operation_id
                .clone()
                .unwrap_or_else(|| spec.handler_id.clone());
            operation.insert("operationId".to_string(), serde_json::Value::String(op_id));

            if let Some(summary) = &spec.summary {
                operation.insert(
                    "summary".to_string(),
                    serde_json::Value::String(summary.clone()),
                );
            }

            if let Some(description) = &spec.description {
                operation.insert(
                    "description".to_string(),
                    serde_json::Value::String(description.clone()),
                );
            }

            if !spec.tags.is_empty() {
                let tags: Vec<serde_json::Value> = spec
                    .tags
                    .iter()
                    .map(|tag| serde_json::Value::String(tag.clone()))
                    .collect();
                operation.insert("tags".to_string(), serde_json::Value::Array(tags));
            }

            // Request body (if any)
            if let Some(req) = &spec.request_body {
                let mut rb = serde_json::Map::new();
                if let Some(desc) = &req.description {
                    rb.insert(
                        "description".to_string(),
                        serde_json::Value::String(desc.clone()),
                    );
                }

                let content = Self::make_content_obj(
                    &components_registry,
                    req.content_type,
                    &req.schema_name,
                );
                rb.insert("content".to_string(), content);

                operation.insert("requestBody".to_string(), serde_json::Value::Object(rb));
            }

            // Responses (always emit content including application/json)
            let mut responses = serde_json::Map::new();
            for response_spec in &spec.responses {
                let mut response_obj = serde_json::Map::new();
                response_obj.insert(
                    "description".to_string(),
                    serde_json::Value::String(response_spec.description.clone()),
                );

                let content = Self::make_content_obj(
                    &components_registry,
                    response_spec.content_type,
                    &response_spec.schema_name,
                );
                response_obj.insert("content".to_string(), content);

                responses.insert(
                    response_spec.status.to_string(),
                    serde_json::Value::Object(response_obj),
                );
            }
            operation.insert(
                "responses".to_string(),
                serde_json::Value::Object(responses),
            );

            // Parameters
            if !spec.params.is_empty() {
                let parameters: Vec<serde_json::Value> = spec
                    .params
                    .iter()
                    .map(|param_spec| {
                        let mut param = serde_json::Map::new();
                        param.insert(
                            "name".to_string(),
                            serde_json::Value::String(param_spec.name.clone()),
                        );

                        let location_str = match param_spec.location {
                            modkit::api::ParamLocation::Path => "path",
                            modkit::api::ParamLocation::Query => "query",
                            modkit::api::ParamLocation::Header => "header",
                            modkit::api::ParamLocation::Cookie => "cookie",
                        };
                        param.insert(
                            "in".to_string(),
                            serde_json::Value::String(location_str.to_string()),
                        );

                        // OpenAPI requires all path params to be required.
                        let is_required = match param_spec.location {
                            modkit::api::ParamLocation::Path => true,
                            _ => param_spec.required,
                        };
                        param.insert("required".to_string(), serde_json::Value::Bool(is_required));

                        if let Some(description) = &param_spec.description {
                            param.insert(
                                "description".to_string(),
                                serde_json::Value::String(description.clone()),
                            );
                        }

                        // TODO: consider a `format` field in ParamSpec (e.g., "uuid") and map it here.
                        param.insert(
                            "schema".to_string(),
                            serde_json::json!({ "type": param_spec.param_type }),
                        );

                        serde_json::Value::Object(param)
                    })
                    .collect();

                operation.insert(
                    "parameters".to_string(),
                    serde_json::Value::Array(parameters),
                );
            }

            // Merge into paths map
            let path_entry = paths_map.entry(path.clone()).or_default();
            path_entry.insert(method, serde_json::Value::Object(operation));
        }

        let paths = serde_json::to_value(paths_map)?;

        // Components: schemas that were registered via OpenApiRegistry
        let mut components = openapi::OpenApiComponents::default();
        for (name, schema) in &components_registry.schemas {
            let schema_value = serde_json::to_value(schema)?;
            components.schemas.insert(name.clone(), schema_value);
        }

        Ok(openapi::OpenApi {
            openapi: "3.0.3",
            info: openapi::OpenApiInfo {
                title: "HyperSpot API",
                version: "0.1.0".to_string(),
                description: Some("HyperSpot Server API Documentation"),
            },
            paths,
            components: Some(components),
        })
    }

    /// Background HTTP server: bind, notify ready, serve until cancelled.
    ///
    /// This method is the lifecycle entry-point generated by the macro
    /// (`#[modkit::module(..., lifecycle(...))]`).
    async fn serve(
        self: Arc<Self>,
        cancel: CancellationToken,
        ready: ReadySignal,
    ) -> anyhow::Result<()> {
        let cfg = self.get_cached_config();
        let addr: SocketAddr = cfg
            .bind_addr
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid bind address '{}': {}", cfg.bind_addr, e))?;

        // Take the finalized router so the MutexGuard is dropped before awaits
        let stored = { self.final_router.lock().take() };
        let router = if let Some(r) = stored {
            tracing::debug!("Using router from REST phase");
            r
        } else {
            tracing::debug!("No router from REST phase, building default router");
            self.build_router().await?
        };

        // Bind the socket, only now consider the service "ready"
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!("HTTP server bound on {}", addr);
        ready.notify(); // Starting -> Running

        // Graceful shutdown on cancel
        let shutdown = {
            let cancel = cancel.clone();
            async move {
                cancel.cancelled().await;
                tracing::info!("HTTP server shutting down gracefully (cancellation)");
            }
        };

        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }
}

// Manual implementation of Module trait with config loading
#[async_trait]
impl modkit::Module for ApiIngress {
    async fn init(&self, ctx: &modkit::ModuleCtx) -> anyhow::Result<()> {
        tracing::debug!(module = "api_ingress", "Module initialized with context");
        let cfg = ctx.module_config::<crate::config::ApiIngressConfig>();
        self.config.store(Arc::new(cfg));
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// Test that the module is properly registered via inventory
#[cfg(test)]
mod tests {
    use super::*;
    use modkit::ModuleRegistry;

    #[test]
    fn test_module_registration() {
        // Ensure the module is discoverable via inventory
        let registry = ModuleRegistry::discover_and_build().expect("Failed to build registry");
        let module = registry.modules().iter().find(|m| m.name == "api_ingress");
        assert!(
            module.is_some(),
            "api_ingress module should be registered via inventory"
        );
    }

    #[test]
    fn test_module_capabilities() {
        let registry = ModuleRegistry::discover_and_build().expect("Failed to build registry");
        let module = registry
            .modules()
            .iter()
            .find(|m| m.name == "api_ingress")
            .expect("api_ingress should be registered");

        // Verify module properties
        assert_eq!(module.name, "api_ingress");

        // Downcast to verify the actual type behind the module
        if let Some(_api_module) = module.core.as_any().downcast_ref::<ApiIngress>() {
            // With lifecycle(...) on the type, stateful capability is provided via WithLifecycle
            assert!(
                module.stateful.is_some(),
                "Module should have stateful capability"
            );
        } else {
            panic!("Failed to downcast to ApiIngress - module not registered correctly");
        }
    }
}

// REST host role: prepare/finalize the router, but do not start the server here.
impl modkit::contracts::RestHostModule for ApiIngress {
    fn rest_prepare(
        &self,
        _ctx: &modkit::context::ModuleCtx,
        router: axum::Router,
    ) -> anyhow::Result<axum::Router> {
        // Add basic health check endpoint and any global middlewares
        let router = router.route("/healthz", get(|| async { "ok" }));

        // You may attach global middlewares here (trace, compression, cors), but do not start server.
        tracing::debug!("REST host prepared base router with health check");
        Ok(router)
    }

    fn rest_finalize(
        &self,
        _ctx: &modkit::context::ModuleCtx,
        mut router: axum::Router,
    ) -> anyhow::Result<axum::Router> {
        let config = self.get_cached_config();

        if config.enable_docs {
            // Build once, serve as static JSON (no per-request parsing)
            let op_count = self.operation_specs.len();
            tracing::info!(
                "🔍 rest_finalize: emitting OpenAPI with {} operations",
                op_count
            );

            let openapi_value = Arc::new(serde_json::to_value(self.build_openapi()?)?);

            router = router
                .route(
                    "/openapi.json",
                    get({
                        use axum::{http::header, response::IntoResponse};
                        let v = openapi_value.clone();
                        move || async move {
                            let json = axum::Json((*v).clone());
                            ([(header::CACHE_CONTROL, "no-store")], json).into_response()
                        }
                    }),
                )
                .route("/docs", get(web::serve_docs));

            #[cfg(feature = "embed_elements")]
            {
                router = router.route("/docs/assets/{*file}", get(assets::serve_elements_asset));
            }
        }

        // Keep the finalized router to be used by `serve()`
        *self.final_router.lock() = Some(router.clone());

        tracing::debug!("REST host finalized router with OpenAPI endpoints");
        Ok(router)
    }

    fn as_registry(&self) -> &dyn modkit::contracts::OpenApiRegistry {
        self
    }
}

impl modkit::contracts::RestfulModule for ApiIngress {
    fn register_rest(
        &self,
        _ctx: &modkit::context::ModuleCtx,
        router: axum::Router,
        _openapi: &dyn modkit::contracts::OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        // This module acts as both rest_host and rest, but actual REST endpoints
        // are handled in the host methods above.
        Ok(router)
    }
}

impl OpenApiRegistry for ApiIngress {
    fn register_operation(&self, spec: &modkit::api::OperationSpec) {
        // Reject duplicates with "first wins" policy (second registration = programmer error).
        if self
            .registered_handlers
            .insert(spec.handler_id.clone(), ())
            .is_some()
        {
            tracing::error!(
                handler_id = %spec.handler_id,
                method = %spec.method.as_str(),
                path = %spec.path,
                "Duplicate handler_id detected; ignoring subsequent registration"
            );
            return;
        }

        let route_key = (spec.method.clone(), spec.path.clone());
        if self.registered_routes.insert(route_key, ()).is_some() {
            tracing::error!(
                method = %spec.method.as_str(),
                path = %spec.path,
                "Duplicate (method, path) detected; ignoring subsequent registration"
            );
            return;
        }

        // Store the operation spec for OpenAPI generation
        let operation_key = format!("{}:{}", spec.method.as_str(), spec.path);
        self.operation_specs
            .insert(operation_key.clone(), spec.clone());

        // Debug: Log the operation registration with current count
        let current_count = self.operation_specs.len();
        tracing::debug!(
            handler_id = %spec.handler_id,
            method = %spec.method.as_str(),
            path = %spec.path,
            summary = %spec.summary.as_deref().unwrap_or("No summary"),
            operation_key = %operation_key,
            total_operations = current_count,
            "🔧 Registered API operation"
        );
    }

    fn register_schema(&self, name: &str, schema: schemars::schema::RootSchema) {
        // Snapshot current registry, copy-on-write
        let current = self.components_registry.load();
        let mut reg = (**current).clone();

        // Select stable component key: title or short FQN
        let mut key = Self::pick_component_key(name, &schema);

        // Normalize to JSON for content comparison
        let new_json = match serde_json::to_value(&schema) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(%name, %key, error=%e, "Failed to serialize schema to JSON");
                return;
            }
        };

        if let Some(existing) = reg.schemas.get(&key) {
            // There is already a schema with this key - compare content
            let Ok(existing_json) = serde_json::to_value(existing) else {
                tracing::error!(%name, %key, "Existing schema failed to serialize; keeping existing, skipping new");
                return;
            };

            if existing_json == new_json {
                // Full duplicates - do not override, to avoid bloating components
                tracing::warn!(
                    original = %name,
                    key = %key,
                    "Schema already registered with identical content; reusing existing"
                );
                // Do nothing
            } else {
                // Conflict of different schemas under the same key - consider it an authorization error
                tracing::error!(
                    original = %name,
                    key = %key,
                    "Conflicting schema content under the same component key; keeping the first and ignoring the new one"
                );
                // TODO: handle this case as an error
            }
        } else {
            // No conflicts - register under the selected key
            reg.register_schema(key.clone(), schema);
            tracing::debug!(original = %name, used_key = %key, "Registered schema");
            self.components_registry.store(Arc::new(reg));
            return;
        }

        // If we got here - nothing changed (duplicate or conflict), but still need to store a copy
        self.components_registry.store(Arc::new(reg));
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
