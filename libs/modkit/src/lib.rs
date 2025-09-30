//! # ModKit - Declarative Module System
//!
//! A unified crate for building modular applications with declarative module definitions.
//!
//! ## Features
//!
//! - **Declarative**: Use `#[module(...)]` attribute to declare modules
//! - **Auto-discovery**: Modules are automatically discovered via inventory
//! - **Type-safe**: Compile-time validation of capabilities
//! - **Phase-based lifecycle**: DB → init → REST → start → stop
//!
//! ## Golden Path: Stateless Handlers
//!
//! For optimal performance and readability, prefer stateless handlers that receive
//! `Extension<T>` and other extractors rather than closures that capture environment.
//!
//! ### Recommended Pattern
//!
//! ```rust,ignore
//! use axum::{Extension, Json};
//! use modkit::api::{OperationBuilder, Problem};
//! use std::sync::Arc;
//!
//! async fn list_users(
//!     Extension(svc): Extension<Arc<UserService>>,
//! ) -> Result<Json<Vec<UserDto>>, Problem> {
//!     let users = svc.list_users().await.map_err(Problem::from)?;
//!     Ok(Json(users))
//! }
//!
//! pub fn router(service: Arc<UserService>) -> axum::Router {
//!     let op = OperationBuilder::get("/users")
//!         .summary("List users")
//!         .handler(list_users)
//!         .json_response(200, "List of users")
//!         .standard_errors(&registry);
//!
//!     axum::Router::new()
//!         .route("/users", axum::routing::get(list_users))
//!         .layer(Extension(service))
//!         .layer(op.to_layer())
//! }
//! ```
//!
//! ### Benefits
//!
//! - **Performance**: No closure captures or cloning on each request
//! - **Readability**: Clear function signatures show exactly what data is needed
//! - **Testability**: Easy to unit test handlers with mock state
//! - **Type Safety**: Compile-time verification of dependencies
//! - **Flexibility**: Individual service injection without coupling
//!
//! ## Basic Module Example
//!
//! ```rust,ignore
//! use modkit::{module, Module, DbModule, RestfulModule, StatefulModule};
//!
//! #[derive(Default)]
//! #[module(name = "user", deps = ["database"], capabilities = [db, rest, stateful])]
//! pub struct UserModule;
//!
//! // Implement the declared capabilities...
//! ```

pub use anyhow::Result;
pub use async_trait::async_trait;

// Re-export inventory for user convenience
pub use inventory;

// Module system exports
pub use crate::contracts::*;
pub mod context;
pub use context::{
    module_config_typed, ConfigError, ConfigProvider, ConfigProviderExt, ModuleCtx,
    ModuleCtxBuilder,
};

// Module system implementations for macro code
pub mod client_hub;
pub mod registry;

// Re-export main types
pub use client_hub::ClientHub;
pub use registry::ModuleRegistry;

// Re-export the macros from the proc-macro crate
pub use modkit_macros::{lifecycle, module};

// Core module contracts and traits
pub mod contracts;
// Type-safe API operation builder
pub mod api;
pub use api::{error_mapping_middleware, IntoProblemResponse, OpenApiRegistry, OperationBuilder};
pub use odata_core::{Page, PageInfo};

// HTTP utilities
pub mod http;
pub use api::problem::{
    bad_request, conflict, internal_error, not_found, Problem, ProblemResponse, ValidationError,
};
pub use http::sse::SseBroadcaster;

pub mod lifecycle;
pub mod runtime;

pub use lifecycle::{Lifecycle, Runnable, Status, StopReason, WithLifecycle};
pub use runtime::{run, DbOptions, RunOptions, ShutdownOptions};

#[cfg(test)]
mod tests;
