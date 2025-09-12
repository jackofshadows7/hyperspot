# New Module Guideline (Hyperspot / ModKit)

This guide provides a comprehensive, step-by-step process for creating production-grade Hyperspot modules. It is designed to be actionable for both human developers and LLM-based code generators, consolidating best practices from across the Hyperspot ecosystem.

## ModKit Core Concepts

ModKit provides a powerful framework for building production-grade modules:

- **Composable Modules**: Discovered via `inventory` and initialized in dependency order.
- **Ingress as a Module**: `api_ingress` owns the Axum router and OpenAPI document.
- **Type-Safe REST**: An operation builder prevents half-wired routes at compile time.
- **Server-Sent Events (SSE)**: Type-safe broadcasters for real-time domain event integration.
- **Standardized HTTP Errors**: Built-in support for RFC-9457 `Problem` and `ProblemResponse`.
- **Typed ClientHub**: For in-process clients, resolved by interface type.
- **Lifecycle Management**: Helpers for long-running tasks and graceful shutdown.

## HyperSpot Modular architecture

A module is a composable unit implementing typically some business logic with either REST API and/or peristent storage. Common and stateless logic that can be reusable across modules should be implemented in the `libs` crate.

## Canonical Project Layout

Modules follow a DDD-light architecture to ensure a clean separation of concerns:

- **`contract`**: The public API surface for inter-module communication. Transport-agnostic.
- **`domain`**: The core business logic and rules, independent of any infrastructure.
- **`infra`**: Implementations of external concerns like databases, message queues, or other system interactions.
- **`api`**: Adapters that translate transport-level requests (e.g., REST) to domain-level commands.

All modules MUST adhere to the following directory structure:

```
modules/<name>/
  src/
    lib.rs              # Public exports (contract, module type), hides internals
    module.rs           # Module struct, #[modkit::module], and trait impls
    config.rs           # Typed config with defaults
    api/                # Transport adapters
      mod.rs            # Re-exports for api
      rest/             # HTTP REST layer
        mod.rs          # Re-exports for rest
        dto.rs          # DTOs (serde, ToSchema)
        handlers.rs     # Thin Axum handlers
        mapper.rs       # From/Into DTO<->Model conversions
        routes.rs       # OperationBuilder registrations
        error.rs        # ProblemResponse mapping
        sse_adapter.rs  # SSE event publisher adapter (optional)
    contract/           # Public API for other modules (NO serde)
      mod.rs            # Re-exports for contract submodules
      client.rs         # Native ClientHub trait (<Module>Api)
      model.rs          # Transport-agnostic models
      error.rs          # Transport-agnostic errors
    domain/             # Internal business logic
      mod.rs            # Re-exports for domain
      events.rs         # Domain events (transport-agnostic)
      ports.rs          # Output ports (e.g., EventPublisher)
      repository.rs     # Repository traits (ports)
      service.rs        # Service orchestrating business logic
    infra/              # Infrastructure adapters
      mod.rs            # Re-exports for infra
      storage/          # Database layer
        entity.rs       # SeaORM entities
        mapper.rs       # From/Into Model<->Entity conversions
        repositories.rs # SeaORM repository implementations
        migrations/     # SeaORM migrations
          mod.rs        # Migrator module entry
    gateways/           # Adapters for client traits
      mod.rs            # Re-exports for gateways
      local.rs          # Local client implementing contract API
  Cargo.toml
```

---

## Step-by-Step Generation Guide

> Note: Strictly mirror the style, naming, and structure of the `examples/modkit/users_info/` reference when generating code.

### Step 1: Project & Cargo Setup

1.  **Create `Cargo.toml`:**
    -   **Rule:** Dependencies and features MUST mirror the canonical `users_info` example to ensure workspace compatibility.

    ```toml
    [package]
    name = "<your-module-name>"
    version = "0.1.0"
    publish = false
    edition.workspace = true

    [dependencies]
    anyhow = "1.0"
    async-trait = "0.1"
    tokio = { version = "1.47", features = ["full"] }
    tracing = "0.1"
    inventory = "0.3"
    serde = { version = "1.0", features = ["derive"] }
    serde_json = "1.0"
    utoipa = { workspace = true }
    axum = { workspace = true, features = ["macros"] }
    tower-http = { version = "0.6", features = ["timeout"] }
    futures = "0.3"
    chrono = { version = "0.4", features = ["serde"] }
    uuid = { version = "1.18", features = ["v4", "serde"] }
    arc-swap = "1.7"
    sea-orm = { version = "1.1", features = ["sqlx-sqlite", "runtime-tokio-rustls", "macros", "with-chrono", "with-uuid"] }
    sea-orm-migration = "1.1"
    thiserror = "2.0"
    modkit = { path = "../../../libs/modkit" }
    db = { path = "../../../libs/db" }

    [dev-dependencies]
    tower = { version = "0.5", features = ["util"] }
    api_ingress = { path = "../../../modules/api_ingress" }
    ```

2.  **Create `src/lib.rs`:**

    -   **Rule:** The public API surface MUST be limited to the `contract` and the main module type. All other modules (`api`, `domain`, `infra`, etc.) MUST be marked with `#[doc(hidden)]`.

    ```rust
    // === PUBLIC CONTRACT ===
    pub mod contract;
    pub use contract::{client, error, model};

    // === MODULE DEFINITION ===
    pub mod module;
    pub use module::<YourModuleTypeName>; // Replace with your module's struct name

    // === INTERNAL MODULES ===
    #[doc(hidden)] pub mod api;
    #[doc(hidden)] pub mod config;
    #[doc(hidden)] pub mod domain;
    #[doc(hidden)] pub mod gateways;
    #[doc(hidden)] pub mod infra;
    ```

### Step 2: Contract Layer (Public API)

This layer defines the transport-agnostic interface for your module.

1.  **Clean Contract**: `contract` models and errors MUST NOT have `serde` or any other transport-specific derives.

2.  **`src/contract/model.rs`:**
    -   **Rule:** Contract models MUST NOT have `serde` or any other transport-specific derives. They are plain Rust structs for inter-module communication.

    ```rust
    // Example from users_info
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct User {
        pub id: uuid::Uuid,
        pub email: String,
        pub display_name: String,
        pub created_at: chrono::DateTime<chrono::Utc>,
        pub updated_at: chrono::DateTime<chrono::Utc>,
    }
    ```

3.  **`src/contract/error.rs`:**
    -   **Rule:** Define a domain-specific error enum for the contract. This allows other modules to handle your errors without depending on implementation details.

    ```rust
    // Example from users_info
    #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
    pub enum UsersInfoError {
        #[error("User not found with ID: {0}")]
        NotFound(uuid::Uuid),
        #[error("Email already exists: {0}")]
        EmailConflict(String),
        #[error("Invalid input: {0}")]
        Validation(String),
        #[error("An internal database error occurred")]
        Database,
    }

    // Provide From conversions between domain and contract errors
    impl From<crate::domain::error::DomainError> for UsersInfoError {
        fn from(e: crate::domain::error::DomainError) -> Self {
            match e {
                crate::domain::error::DomainError::UserNotFound(id) => Self::NotFound(id),
                crate::domain::error::DomainError::EmailAlreadyExists(email) => Self::EmailConflict(email),
                crate::domain::error::DomainError::InvalidEmail(_) => Self::Validation("Invalid email format".to_string()),
                _ => Self::Database,
            }
        }
    }
    ```

4.  **`src/contract/client.rs`:**
    -   **Rule:** Define a native, async trait for the ClientHub. This is the primary way other modules will interact with your service. Name it `<PascalCaseModule>Api`.

    ```rust
    // Example from users_info
    #[async_trait::async_trait]
    pub trait UsersInfoApi: Send + Sync {
        async fn get_user(&self, id: uuid::Uuid) -> Result<super::model::User, super::error::UsersInfoError>;
        // ... other methods
    }
    ```

### Step 3: API Layer (REST)

This layer adapts HTTP requests to domain calls.

> Note:
> - Do NOT implement a REST host. `api_ingress` owns the Axum server and OpenAPI. Modules only register routes via `register_routes(...)`.
> - Use dependency injection with `Extension<Arc<Service>>` in handlers and attach the service ONCE after all routes are registered: `router = router.layer(Extension(service.clone()));`.
> - Follow the `<crate>.<resource>.<action>` convention for `operation_id` naming.

1.  **`src/api/rest/dto.rs`:**
    -   **Rule:** Create Data Transfer Objects (DTOs) for the REST API. These structs derive `serde` and `utoipa::ToSchema`.
    -   **Rule:** Map OpenAPI types correctly: `string: uuid` -> `uuid::Uuid`, `string: date-time` -> `chrono::DateTime<chrono::Utc>`.

2.  **`src/api/rest/mapper.rs`:**
    -   **Rule:** Provide `From` implementations to convert between DTOs and `contract` models.

2.  **`src/api/rest/handlers.rs`:**
    -   **Rule:** Handlers must be thin. They extract data, call the domain service, and map results.
    -   **Rule:** Use `Extension<Arc<Service>>` for dependency injection.
    -   **Rule:** Handler return types must match the canonical patterns:
        -   `GET` -> `Result<Json<T>, ProblemResponse>`
        -   `POST` -> `Result<(StatusCode, Json<T>), ProblemResponse>`
        -   `DELETE/PUT (no body)` -> `Result<StatusCode, ProblemResponse>`

    ```rust
    // Example from users_info
    pub async fn get_user(
        Extension(svc): Extension<std::sync::Arc<Service>>,
        axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
        uri: axum::http::Uri,
    ) -> Result<axum::Json<super::dto::UserDto>, modkit::api::problem::ProblemResponse> {
        let user = svc.get_user(id).await.map_err(|e| super::error::map_domain_error(&e, uri.path()))?;
        Ok(axum::Json(user.into()))
    }
    ```

3.  **`src/api/rest/routes.rs`:**
    -   **Rule:** Register ALL endpoints in a single `register_routes` function.
    -   **Rule:** Use `OperationBuilder` for every route, following the strict order: describe -> handler -> responses -> register.
    -   **Rule:** After all routes are registered, attach the service ONCE with `router.layer(Extension(service.clone()))`.

    ```rust
    // Example from users_info
    pub fn register_routes(
        mut router: axum::Router,
        openapi: &dyn modkit::api::OpenApiRegistry,
        service: std::sync::Arc<Service>,
    ) -> anyhow::Result<axum::Router> {
        router = modkit::api::OperationBuilder::<_, _, ()>::get("/users/{id}")
            .operation_id("users_info.get_user")
            .handler(super::handlers::get_user)
            .json_response_with_schema::<super::dto::UserDto>(openapi, 200, "OK")
            .problem_response(openapi, 404, "Not Found")
            .register(router, openapi);

        // ... other routes

        router = router.layer(axum::Extension(service.clone()));
        Ok(router)
    }
    ```

### Step 4: Domain Layer

This layer contains the core business logic, free from infrastructure concerns.

1.  **`src/domain/events.rs`:**
    -   **Rule:** Define transport-agnostic domain events for important business actions.

    ```rust
    // Example from users_info
    #[derive(Debug, Clone)]
    pub enum UserDomainEvent {
        Created { id: uuid::Uuid, at: chrono::DateTime<chrono::Utc> },
        Updated { id: uuid::Uuid, at: chrono::DateTime<chrono::Utc> },
        Deleted { id: uuid::Uuid, at: chrono::DateTime<chrono::Utc> },
    }
    ```

2.  **`src/domain/ports.rs`:**
    -   **Rule:** Define output ports (interfaces) for external concerns like event publishing.

    ```rust
    // Example from users_info
    pub trait EventPublisher<E>: Send + Sync + 'static {
        fn publish(&self, event: &E);
    }
    ```

3.  **`src/domain/repository.rs`:**
    -   **Rule:** Define repository traits (ports) that the service will depend on. This decouples the domain from the database implementation.

    ```rust
    // Example from users_info
    #[async_trait::async_trait]
    pub trait UsersRepository: Send + Sync {
        async fn find_by_id(&self, id: uuid::Uuid) -> anyhow::Result<Option<crate::contract::model::User>>;
        async fn email_exists(&self, email: &str) -> anyhow::Result<bool>;
        // ... other data access methods
    }
    ```

4.  **`src/domain/service.rs`:**
    -   **Rule:** The `Service` struct encapsulates all business logic. It depends on repository traits and event publishers, not concrete implementations.

    ```rust
    // Example from users_info
    use super::events::UserDomainEvent;
    use super::ports::EventPublisher;

    pub struct Service {
        repo: std::sync::Arc<dyn UsersRepository>,
        events: std::sync::Arc<dyn EventPublisher<UserDomainEvent>>,
        config: ServiceConfig,
    }

    impl Service {
        pub fn new(
            repo: std::sync::Arc<dyn UsersRepository>,
            events: std::sync::Arc<dyn EventPublisher<UserDomainEvent>>,
            config: ServiceConfig,
        ) -> Self {
            Self { repo, events, config }
        }

        pub async fn create_user(&self, new_user: crate::contract::model::NewUser) -> Result<crate::contract::model::User, crate::domain::error::DomainError> {
            // Business logic here...
            let user = self.repo.insert(user.clone()).await?;

            // Publish domain event
            self.events.publish(&UserDomainEvent::Created {
                id: user.id,
                at: user.created_at,
            });

            Ok(user)
        }
    }
    ```

### Step 5: Infra Layer (Storage)

If no database requred Skip `DbModule`, remove `db` from capabilities

This layer implements the domain's repository traits.

1.  **`src/infra/storage/repositories.rs`:**
    -   **Rule:** Implement the repository trait using SeaORM. The implementation should be generic over `C: ConnectionTrait` to support both direct connections and transactions.

    ```rust
    // Example from users_info
    use sea_orm::{ConnectionTrait, EntityTrait};

    pub struct SeaOrmUsersRepository<C> where C: ConnectionTrait + Send + Sync {
        conn: C,
    }

    impl<C> SeaOrmUsersRepository<C> where C: ConnectionTrait + Send + Sync {
        pub fn new(conn: C) -> Self { Self { conn } }
    }

    #[async_trait::async_trait]
    impl<C> crate::domain::repository::UsersRepository for SeaOrmUsersRepository<C>
    where C: ConnectionTrait + Send + Sync + 'static {
        async fn find_by_id(&self, id: uuid::Uuid) -> anyhow::Result<Option<crate::contract::model::User>> {
            let found = super::entity::Entity::find_by_id(id).one(&self.conn).await?;
            Ok(found.map(Into::into))
        }
        // ... other implementations
    }
    ```

2.  **`src/infra/storage/migrations/`:**
    -   **Rule:** Create a SeaORM migrator. This is mandatory for any module with the `db` capability.

### Step 6: Module Wiring & Lifecycle

#### `#[modkit::module]` Full Syntax

The `#[modkit::module]` macro provides declarative registration and lifecycle management.

```rust
#[modkit::module(
    name = "my_module",
    deps = ["db"], // Dependencies on other modules
    capabilities = [db, rest, stateful],
    client = "contract::client::MyModuleApi", // Generates ClientHub helpers
    ctor = MyModule::new(), // Constructor expression (defaults to `Default`)
    lifecycle(entry = "serve", stop_timeout = "30s", await_ready) // For stateful background tasks
)]
pub struct MyModule { /* ... */ }
```

#### `ModuleCtx` Runtime Context

The `init` function receives a `ModuleCtx` struct, which provides access to essential runtime components:

- **`ctx.db()`**: Returns an `Option<Arc<db::DbHandle>>` for database access.
- **`ctx.module_config()`**: Deserializes the module's typed configuration.
- **`ctx.client_hub()`**: Provides access to the `ClientHub` for resolving other module clients.
- **`ctx.cancellation_token()`**: A `CancellationToken` for graceful shutdown of background tasks.

This is where all components are assembled and registered with ModKit.

1.  **`src/module.rs` - The `#[modkit::module]` macro:**
    -   **Rule:** The module MUST declare `capabilities = [db, rest]`.
    -   **Rule:** The `client` property MUST be set to the path of your native client trait (e.g., `crate::contract::client::UsersInfoApi`).

2.  **`src/module.rs` - `impl Module for YourModule`:**
    -   **Rule:** The `init` function is the composition root. It MUST:
        1.  Read the typed config: `let cfg: Config = ctx.module_config();`
        2.  Get a DB handle and fail if it's missing: `let db = ctx.db().ok_or_else(...)`.
        3.  Instantiate the repository, service, and any other dependencies.
        4.  Store the `Arc<Service>` in a thread-safe container like `arc_swap::ArcSwapOption`.
        5.  Instantiate and expose the native client to the `ClientHub` using the generated `expose_<module_name>_client` function.

3.  **`src/module.rs` - `impl DbModule` and `impl RestfulModule`:**
    -   **Rule:** `DbModule::migrate` MUST be implemented to run your SeaORM migrations.
    -   **Rule:** `RestfulModule::register_rest` MUST fail if the service is not yet initialized, then call your single `register_routes` function.

```rust
// Example from users_info/src/module.rs
#[modkit::module(
    name = "users_info",
    deps = ["db"],
    capabilities = [db, rest],
    client = crate::contract::client::UsersInfoApi
)]
pub struct UsersInfo {
    service: arc_swap::ArcSwapOption<Service>,
    sse: modkit::SseBroadcaster<UserEvent>, // Optional: for real-time events
}

#[async_trait::async_trait]
impl Module for UsersInfo {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        let cfg: UsersInfoConfig = ctx.module_config();
        let db = ctx.db().ok_or_else(|| anyhow::anyhow!("DB required"))?;
        let repo = SeaOrmUsersRepository::new(db.sea());

        // Create event publisher adapter for SSE (optional)
        let event_publisher: Arc<dyn EventPublisher<UserDomainEvent>> =
            Arc::new(SseUserEventPublisher::new(self.sse.clone()));

        let service = Service::new(Arc::new(repo), event_publisher, cfg.into());
        self.service.store(Some(Arc::new(service.clone())));

        let api: Arc<dyn UsersInfoApi> = Arc::new(UsersInfoLocalClient::new(Arc::new(service)));
        expose_users_info_client(ctx, &api)?;
        Ok(())
    }
    // ...
}

#[async_trait::async_trait]
impl DbModule for UsersInfo {
    async fn migrate(&self, db: &db::DbHandle) -> anyhow::Result<()> {
        crate::infra::storage::migrations::Migrator::up(db.seaorm(), None).await?;
        Ok(())
    }
}

impl RestfulModule for UsersInfo {
    fn register_rest(&self, _ctx: &ModuleCtx, router: axum::Router, openapi: &dyn OpenApiRegistry) -> anyhow::Result<axum::Router> {
        let service = self.service.load().as_ref().ok_or_else(|| anyhow::anyhow!("Service not initialized"))?.clone();
        let router = routes::register_routes(router, openapi, service)?;

        // Optional: Register SSE route for real-time events
        let router = routes::register_sse_route(router, openapi, self.sse.clone());

        Ok(router)
    }
}
```

### Step 7: SSE Integration (Optional)

If no SSE required: Remove `SseBroadcaster` and event publishing

For real-time event streaming, add Server-Sent Events support.

1.  **`src/api/rest/sse_adapter.rs`:**
    -   **Rule:** Create an adapter that implements the domain `EventPublisher` port and forwards events to the SSE broadcaster.

    ```rust
    // Example from users_info
    use modkit::SseBroadcaster;
    use crate::domain::{events::UserDomainEvent, ports::EventPublisher};
    use super::dto::UserEvent;

    pub struct SseUserEventPublisher {
        out: SseBroadcaster<UserEvent>,
    }

    impl SseUserEventPublisher {
        pub fn new(out: SseBroadcaster<UserEvent>) -> Self {
            Self { out }
        }
    }

    impl EventPublisher<UserDomainEvent> for SseUserEventPublisher {
        fn publish(&self, event: &UserDomainEvent) {
            self.out.send(UserEvent::from(event));
        }
    }

    // Convert domain events to transport events
    impl From<&UserDomainEvent> for UserEvent {
        fn from(e: &UserDomainEvent) -> Self {
            use UserDomainEvent::*;
            match e {
                Created { id, at } => Self { kind: "created".into(), id: *id, at: *at },
                Updated { id, at } => Self { kind: "updated".into(), id: *id, at: *at },
                Deleted { id, at } => Self { kind: "deleted".into(), id: *id, at: *at },
            }
        }
    }
    ```

2.  **Add SSE route registration:**
    -   **Rule:** Register SSE routes separately from CRUD routes, with proper timeout and Extension layers.

    ```rust
    // In api/rest/routes.rs
    pub fn register_sse_route(
        router: axum::Router,
        openapi: &dyn modkit::api::OpenApiRegistry,
        sse: modkit::SseBroadcaster<UserEvent>,
    ) -> axum::Router {
        modkit::api::OperationBuilder::<_, _, ()>::get("/users/events")
            .operation_id("users_info.events")
            .summary("User events stream (SSE)")
            .description("Real-time stream of user events as Server-Sent Events")
            .tag("users")
            .handler(handlers::users_events)
            .sse_json::<UserEvent>(openapi, "SSE stream of UserEvent")
            .register(router, openapi)
            .layer(axum::Extension(sse))
            .layer(tower_http::timeout::TimeoutLayer::new(std::time::Duration::from_secs(3600)))
    }
    ```

### Step 8: Gateway Implementation

Implement the local client that bridges the domain service to the contract API.

1.  **`src/gateways/local.rs`:**
    -   **Rule:** Create a local implementation of your contract client trait that delegates to the domain service.

    ```rust
    // Example from users_info
    use async_trait::async_trait;
    use std::sync::Arc;
    use crate::contract::{client::UsersInfoApi, error::UsersInfoError, model::*};
    use crate::domain::service::Service;

    pub struct UsersInfoLocalClient {
        service: Arc<Service>,
    }

    impl UsersInfoLocalClient {
        pub fn new(service: Arc<Service>) -> Self {
            Self { service }
        }
    }

    #[async_trait]
    impl UsersInfoApi for UsersInfoLocalClient {
        async fn get_user(&self, id: uuid::Uuid) -> Result<User, UsersInfoError> {
            self.service.get_user(id).await.map_err(Into::into)
        }
        // ... implement all trait methods
    }
    ```

### Step 9: Testing

-   **Unit Tests:** Place next to the code being tested. Mock repository traits to test domain service logic in isolation.
-   **Integration/REST Tests:** Place in the `tests/` directory. Use `Router::oneshot` with a stubbed service or a real service connected to a test database to verify handlers, serialization, and error mapping.
-   **Static Verification:** Always run `cargo clippy`, `cargo fmt`, and `cargo audit` before committing.

---

## Pitfalls Checklist (must pass)

- capabilities include [db, rest]; module macro includes `deps = ["db"]` and `client = "contract::client::<...>Api"`.
- do not implement a REST host; `api_ingress` owns the Axum server/OpenAPI.
- read typed config in `init()` with `#[serde(deny_unknown_fields)]` and provide safe defaults.
- require DB presence in `init()`; pass the SeaORM connection into repo implementations.
- build the domain `Service` in `init()` with repository and event publisher dependencies; store it in a thread-safe cell and ensure it's initialized before REST registration.
- contract models/errors are transport-agnostic (NO serde derives); provide `From` conversions between domain and contract errors.
- define domain events and ports for clean separation of concerns; use adapters to bridge domain events to transport (SSE).
- implement local client in `gateways/` that delegates to domain service and implements contract client trait.
- register ALL endpoints in ONE `register_routes(...)` function using `OperationBuilder` in order: describe → handler → responses → register.
- after registering routes, attach the service ONCE: `router = router.layer(Extension(service.clone()));`.
- centralized RFC-9457 mapping to `ProblemResponse`; handler return types follow: GET→`Result<Json<T>, ProblemResponse>`, POST/201→`Result<(StatusCode, Json<T>), ProblemResponse>`, 204→`Result<StatusCode, ProblemResponse>`.
- implement `DbModule::migrate()` to run the SeaORM migrator.
- public exports limited to `contract` and the module type; internals marked `#[doc(hidden)]`.
- include proper `mod.rs` files in all internal module directories for clean re-exports.

---

### Edge Cases for LLMs
- **Complex queries**: Use `impl RangeBounds<T>` in repository traits
- **File uploads**: Use `impl AsyncRead` in handlers

---

### Rust Best Practices

- **Panic Policy**: Panics mean "stop the program". Use for programming errors only, never for recoverable conditions.

- **Public Types**: All public types MUST implement `Debug`. Types for user display should implement `Display`.

- **API Design**:
   - Don't expose smart pointers (`Arc<T>`, `Box<T>`) in public APIs
   - Accept `impl AsRef<str>` instead of `&str` for flexibility
   - Accept `impl AsRef<Path>` for file paths
   - Use inherent methods for core functionality, traits for extensions
   - Accept `impl RangeBounds<T>` for range parameters

- **Error Design**: Use situation-specific error structs with `Backtrace`, not mega-enums. Provide `is_xxx()` helper methods.

- **Static Verification**: Run these tools in CI:
   - `cargo clippy --all-targets --all-features`
   - `cargo fmt --check`
   - `cargo audit` (security vulnerabilities)
   - `cargo-hack` (feature combinations)
   - `cargo-udeps` (unused dependencies)
   - `miri` (unsafe code validation)

- **Type Safety**:
   - All public types must be `Send` (especially futures)
   - Don't leak external crate types in public APIs
   - Use `#[expect]` for lint overrides (not `#[allow]`)

- **Initialization**: Types with 4+ initialization permutations should provide builders named `FooBuilder`.

- **Avoid Statics**: Use dependency injection instead of global statics for correctness.

---

## References

-   `examples/modkit/users_info/` — The canonical reference implementation.
