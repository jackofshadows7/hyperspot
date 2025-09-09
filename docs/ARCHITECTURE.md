# HyperSpot Architecture

This document describes the high-level architecture of HyperSpot Server, a modular platform for building AI services with automatic REST API generation and comprehensive OpenAPI documentation using the ModKit framework.

## 🆕 Recent Architecture Improvements

- **Centralized Error Handling**: RFC-9457 Problem Details standardized across all modules
- **Enhanced Type Safety**: Improved `OperationBuilder` with compile-time guarantees for error responses
- **Schema Management**: Automatic OpenAPI schema registration with conflict resolution
- **Idiomatic Conversions**: Standardized `From` trait implementations for type conversions

## 🏛️ System Overview

HyperSpot follows a **modular monolith** architecture where functionality is organized into self-contained modules that communicate through well-defined interfaces. The system provides automatic service discovery, dependency resolution, and lifecycle management through the ModKit runtime.

```mermaid
graph TB
    subgraph "HTTP Layer"
        Client[HTTP Clients]
        Docs[Stoplight Elements]
        Gateway[API Gateway]
    end
    
    subgraph "ModKit Runtime"
        Runner[Runtime Runner]
        Registry[Module Registry]
        Hub[Client Hub]
        Context[Module Context]
    end
    
    subgraph "Modules"
        ApiIngress[API Ingress]
        CustomMod[Custom Modules...]
    end
    
    subgraph "Infrastructure"
        DB[(Database)]
        Config[Configuration]
        Logging[Tracing & Logs]
    end
    
    Client --> Gateway
    Docs --> Gateway
    Gateway --> ApiIngress
    Runner --> Registry
    Registry --> Hub
    Hub --> Modules
    Registry --> Context
    Context --> Config
    Modules --> DB
    Modules --> Logging
```

## 🔄 ModKit Runtime & Module Lifecycle

HyperSpot uses the ModKit runtime (`modkit::runtime::run`) to manage the complete module lifecycle through a series of well-defined phases executed in dependency order:

### Runtime Entry Point

```rust
use modkit::runtime::{run, RunOptions, DbOptions, ShutdownOptions, DbFactory};

async fn run_server(config: AppConfig, args: CliArgs) -> Result<()> {
    let config_provider = Arc::new(ModkitConfigAdapter(Arc::new(AppConfigProvider::new(config.clone()))));

    let db_options = if let Some(db_config) = config.database.clone() {
        let factory: DbFactory = Box::new(move || {
            let db_config = db_config.clone();
            Box::pin(async move {
                let connect_opts = ConnectOpts {
                    max_conns: db_config.max_conns,
                    acquire_timeout: Some(std::time::Duration::from_secs(5)),
                    sqlite_busy_timeout: db_config.busy_timeout_ms.map(|ms| Duration::from_millis(ms as u64)),
                    create_sqlite_dirs: true,
                };
                let db = DbHandle::connect(&db_config.url, connect_opts).await?;
                Ok(Arc::new(db))
            })
        });
        DbOptions::Auto(factory)
    } else {
        DbOptions::None
    };

    let run_options = RunOptions {
        modules_cfg: config_provider,
        db: db_options,
        openapi: Some(api_ingress::ApiIngress::default()),
        shutdown: ShutdownOptions::Signals,
    };

    run(run_options).await
}
```

### 1. **Discovery Phase**
```rust
let registry = ModuleRegistry::discover_and_build().await?;
```
- Auto-discovers modules via `inventory` crate and `#[modkit::module]` macro
- Registers modules with their capabilities (core, db, rest, stateful)
- Builds dependency graph from module metadata

### 2. **Topological Sort**
```rust
registry.topo_sort()?;
```
- Sorts modules by dependencies to ensure proper initialization order
- Detects circular dependencies and fails fast
- Creates execution plan for subsequent phases

### 3. **Initialization Phase**
```rust
registry.run_init_phase(ctx).await?;
```
- Initializes module state and resources
- Loads configuration from context
- Establishes connections and prepares for operation
- Modules publish their typed clients to ClientHub

### 4. **Database Phase**
```rust
registry.run_db_phase(ctx).await?;
```
- Runs database migrations for modules that implement `DbModule`
- Migrations run in dependency order with database connection
- Each module's `migrate()` method handles schema evolution

### 5. **REST Registration Phase**
```rust
let router = axum::Router::new();
let mut openapi_registry = ApiIngress::default();
let final_router = registry.run_rest_phase(ctx, router, &mut openapi_registry).await?;
```
- Modules register their REST endpoints using the type-safe `OperationBuilder`
- OpenAPI documentation is generated from route registrations
- Builds unified router with all endpoints

### 6. **Start Phase**
```rust
registry.run_start_phase(cancellation_token).await?;
```
- Starts background services and workers
- Opens network listeners
- Module becomes ready to serve requests

### 7. **Shutdown Phase**
```rust
registry.run_stop_phase(cancellation_token).await?;
```
- Graceful shutdown in reverse dependency order
- Uses cancellation tokens for cooperative shutdown
- Ensures data consistency during shutdown

## 🧩 Core Components

### ModKit Runtime

The `modkit::runtime::run` function is the central orchestrator that manages the entire application lifecycle:

```rust
pub async fn run(options: RunOptions) -> anyhow::Result<()> {
    // 1. Build module context from configuration
    let ctx = build_module_context(options).await?;
    
    // 2. Discover and register modules
    let registry = ModuleRegistry::discover_and_build().await?;
    
    // 3. Execute lifecycle phases
    registry.run_init_phase(&ctx).await?;
    registry.run_db_phase(&ctx).await?;
    let router = registry.run_rest_phase(&ctx, router, openapi).await?;
    registry.run_start_phase(cancellation_token).await?;
    
    // 4. Wait for shutdown signal
    wait_for_shutdown(cancellation_token).await;
    
    // 5. Graceful shutdown
    registry.run_stop_phase(cancellation_token).await?;
    
    Ok(())
}
```

### Module Registry

The `ModuleRegistry` manages module discovery, dependency resolution, and lifecycle execution.

```rust
pub struct ModuleRegistry {
    modules: HashMap<String, Arc<dyn Module>>,
    db_modules: HashMap<String, Arc<dyn DbModule>>,
    rest_modules: HashMap<String, Arc<dyn RestfulModule>>,
    stateful_modules: HashMap<String, Arc<dyn StatefulModule>>,
    sorted: Vec<String>,
    is_sorted: bool,
}
```

**Key Responsibilities:**
- Auto-discovery via `inventory::collect!()` and `#[modkit::module]` macro
- Dependency resolution and topological sorting
- Coordinated lifecycle phase execution
- Error handling and rollback on failures

### Module Context

`ModuleCtx` provides modules with access to shared resources and configuration during initialization.

```rust
pub struct ModuleCtx {
    pub db: Option<Arc<dyn DbHandle>>,
    pub client_hub: Arc<ClientHub>,
    config_provider: Arc<dyn ConfigProvider>,
}
```

**Features:**
- Type-safe configuration retrieval: `ctx.module_config::<T>()`
- Database handle sharing for data access
- ClientHub for typed inter-module communication
- Extensible for additional shared resources

### Client Hub

The `ClientHub` provides type-safe access to module clients with lock-free performance optimizations.

```rust
pub struct ClientHub {
    store: DashMap<ClientStoreKey, ClientStoreValue>,
}
```

**Usage Pattern:**
```rust
// Publishing a client (in module init)
let api: Arc<dyn MyModuleApi> = Arc::new(MyModuleApiImpl::new(service));
expose_my_module_api(ctx, &api)?;

// Consuming a client (from another module)
let client = my_module_client(ctx.client_hub()).await;
let result = client.some_operation().await?;
```

## 🌐 API Gateway Architecture

The API Gateway (`api_ingress` module) acts as the single entry point for all HTTP traffic, providing routing, documentation, and cross-cutting concerns.

### Request Flow

```mermaid
sequenceDiagram
    participant Client
    participant Gateway as API Gateway
    participant Router as Axum Router
    participant Handler as Module Handler
    participant Module
    
    Client->>Gateway: HTTP Request
    Gateway->>Router: Route Resolution
    Router->>Handler: Execute Handler
    Handler->>Module: Business Logic
    Module-->>Handler: Response Data
    Handler-->>Router: HTTP Response
    Router-->>Gateway: Response
    Gateway-->>Client: HTTP Response
```

### Route Registration

Modules register endpoints using the modern type-safe `OperationBuilder`:

```rust
impl RestfulModule for MyModule {
    fn register_rest(
        &self,
        _ctx: &ModuleCtx,
        router: axum::Router,
        openapi: &mut dyn OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        use modkit::api::OperationBuilder;
        
        // Schemas are automatically registered via OperationBuilder methods
        
        // Type-safe endpoint registration
        let router = OperationBuilder::get("/users/{id}")
            .operation_id("users.get")
            .summary("Get user by ID")
            .description("Retrieves a single user by their unique identifier")
            .tag("users")
            .path_param("id", "User ID")
            .json_response(200, "User details")
            .json_response(404, "User not found")
            .handler(axum::routing::get(get_user_handler))
            .register(router, openapi);
            
        Ok(router)
    }
}
```

### OpenAPI Generation

The gateway automatically generates OpenAPI 3.0 specifications with enhanced features:

- **Operations**: Generated from `OperationBuilder` registrations with RFC-9457 error responses
- **Schemas**: Auto-registered from Rust types using `utoipa::ToSchema` with conflict resolution
- **Problem Details**: Centralized RFC-9457 error schema for consistent error responses
- **Components**: Shared schema definitions managed by `ComponentsRegistry` with atomic updates
- **Documentation**: Available at `/docs` (Stoplight Elements) and `/openapi.json`

#### RFC-9457 Error Integration

All modules now use standardized error responses:

```rust
OperationBuilder::post("/users")
    .json_request::<CreateUserRequest>(openapi, "User data")
    .json_response_with_schema::<User>(openapi, 201, "User created")
    .problem_response(openapi, 400, "Invalid input")
    .problem_response(openapi, 409, "Email already exists")
    .problem_response(openapi, 500, "Internal server error")
    .handler(create_user)
    .register(router, openapi)
```

This generates proper OpenAPI responses with `application/problem+json` content type and references to the shared `Problem` schema.

## 🔒 Type Safety & Error Handling

### Compile-Time Guarantees

HyperSpot leverages Rust's type system for comprehensive compile-time validation:

```rust
// OperationBuilder enforces compile-time constraints
let router = OperationBuilder::get("/users")
    .summary("List users")
    .json_response(200, "User list")
    .handler(get(list_users))  // Handler is required
    .register(router, openapi); // Won't compile without handler and response
```

### Error Propagation & RFC-9457 Handling

The system uses multiple error handling strategies:

#### Module Lifecycle Errors
Module operations use `anyhow::Result` for consistent error handling:

```rust
#[async_trait]
impl Module for MyModule {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        let config = ctx.module_config::<MyConfig>()
            .context("Failed to load module configuration")?;
        // ...
        Ok(())
    }
}
```

#### HTTP Error Responses
REST handlers use `ProblemResponse` for standardized HTTP error responses:

```rust
use modkit::{ProblemResponse, bad_request, not_found, conflict, internal_error};

async fn get_user_handler(
    Path(id): Path<String>
) -> Result<Json<User>, ProblemResponse> {
    match user_service.find_by_id(&id).await {
        Ok(Some(user)) => Ok(Json(user.into())),
        Ok(None) => Err(not_found(format!("User '{}' not found", id))),
        Err(DomainError::InvalidFormat { .. }) => {
            Err(bad_request("Invalid user ID format"))
        }
        Err(e) => {
            tracing::error!("Failed to fetch user: {}", e);
            Err(internal_error("User retrieval failed"))
        }
    }
}
```

#### Domain Error Mapping
Domain errors are mapped to appropriate HTTP problems:

```rust
impl From<DomainError> for ProblemResponse {
    fn from(error: DomainError) -> Self {
        match error {
            DomainError::NotFound { .. } => not_found(error.to_string()),
            DomainError::AlreadyExists { .. } => conflict(error.to_string()),
            DomainError::InvalidInput { .. } => bad_request(error.to_string()),
            _ => internal_error("Operation failed")
        }
    }
}
```

### Lock-Free Performance

The system uses lock-free patterns for hot paths:

- **DashMap**: Concurrent access to collections without global locks
- **ArcSwap**: Lock-free atomic updates for read-mostly data
- **OnceCell**: Race-free lazy initialization
- **CancellationToken**: Modern async lifecycle management

## 🏗️ Module Implementation Patterns

### Modern Module Declaration

```rust
#[modkit::module(
    name = "my_module",
    deps = ["db"],
    capabilities = [db, rest, stateful],
    client = "MyModuleApi"
)]
#[derive(Clone, Default)]
pub struct MyModule {
    service: Option<Arc<MyService>>,
}
```

### Module Trait Implementation

```rust
#[async_trait]
impl Module for MyModule {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        let config: MyConfig = ctx.module_config();
        let db = ctx.db().ok_or_else(|| anyhow!("Database required"))?;
        
        let service = Arc::new(MyService::new(db.seaorm()));
        let api: Arc<dyn MyModuleApi> = Arc::new(MyModuleApiImpl::new(service));
        
        expose_my_module_api(ctx, &api)?;
        tracing::info!("MyModule initialized");
        Ok(())
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait]
impl DbModule for MyModule {
    async fn migrate(&self, db: &db::DbHandle) -> anyhow::Result<()> {
        let conn = db.seaorm();
        MyMigrator::up(conn, None).await?;
        Ok(())
    }
}

impl RestfulModule for MyModule {
    fn register_rest(
        &self,
        ctx: &ModuleCtx,
        router: axum::Router,
        openapi: &mut dyn OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        // Register routes with type-safe OperationBuilder
        Ok(register_my_module_routes(router, openapi))
    }
}

#[async_trait]
impl StatefulModule for MyModule {
    async fn start(&self, cancellation_token: CancellationToken) -> anyhow::Result<()> {
        tracing::info!("MyModule started");
        Ok(())
    }
    
    async fn stop(&self, cancellation_token: CancellationToken) -> anyhow::Result<()> {
        tracing::info!("MyModule stopped");
        Ok(())
    }
}
```

## 🚀 Performance Characteristics

### Concurrency Model

- **Async-first**: Built on Tokio for high-concurrency workloads
- **Lock-free hot paths**: DashMap and ArcSwap eliminate lock contention
- **Typed client access**: Single downcast per client type/key combination

### Memory Management

- **Zero-copy routing**: Request routing avoids unnecessary allocations
- **Schema reuse**: OpenAPI components prevent schema duplication
- **Efficient serialization**: Direct `serde` integration for JSON handling

### Scalability Patterns

- **Horizontal scaling**: Stateless design enables multiple instances
- **Resource isolation**: Module boundaries prevent cascading failures
- **Graceful degradation**: Individual module failures don't crash the system

## 🔧 Configuration System

### Hierarchical Configuration

```yaml
# Global configuration
server:
  home_dir: "~/.hyperspot"

database:
  url: "sqlite://database/database.db"
  max_conns: 10
  busy_timeout_ms: 5000

# Module-specific configuration  
modules:
  api_ingress:
    bind_addr: "127.0.0.1:8087"
    enable_docs: true
    cors_enabled: true
    
  my_module:
    setting1: "value1"
    setting2: 42
```

### Environment Overrides

Configuration supports environment variable overrides with `HYPERSPOT_` prefix:

```bash
export HYPERSPOT_DATABASE_URL="postgres://user:pass@localhost/db"
export HYPERSPOT_MODULES_API_INGRESS_BIND_ADDR="0.0.0.0:8080"
export HYPERSPOT_MODULES_MY_MODULE_SETTING1="override_value"
```

## 📊 Observability

### Structured Logging

All components use structured logging via `tracing`:

```rust
tracing::info!(
    module = "api_ingress",
    method = %method,
    path = %path,
    "Route registered successfully"
);
```

### Health Checks

Built-in health check endpoints:

- `GET /health` - Basic liveness check
- Module-specific health via `StatefulModule` lifecycle

### Request Tracing

Request correlation and tracking:

```rust
tracing::Span::current().record("request_id", tracing::field::display(&request_id));
```

## 🔮 Future Architecture

### Planned Enhancements

1. **gRPC Support**: Inter-module communication via gRPC
2. **Event Streaming**: Async event bus for loose coupling  
3. **Plugin System**: Dynamic module loading and hot-reload
4. **Distributed Tracing**: OpenTelemetry integration
5. **Service Mesh**: Istio/Linkerd integration for K8s deployments

### Microservices Evolution

The current modular monolith can evolve to microservices:

1. **Module Extraction**: Move modules to separate processes
2. **Service Discovery**: Replace ClientHub with service registry
3. **API Gateway**: Expand to handle external routing
4. **Data Consistency**: Implement event sourcing patterns

## 🎯 Key Architectural Decisions

### Why ModKit Runtime?

- **Unified Lifecycle**: Single entry point for all module management
- **Type Safety**: Compile-time validation of module contracts
- **Dependency Management**: Automatic topological sorting
- **Configuration Integration**: Seamless YAML config binding

### Why Modular Monolith?

- **Single deployable unit**: Simplified deployment and testing
- **Shared resources**: Efficient database connection pooling
- **Type safety**: Compile-time interface validation
- **Evolution path**: Can extract to microservices later

### Why Type-Safe API Builder?

- **Compile-time validation**: Prevents runtime API definition errors
- **Better developer experience**: IDE support and autocomplete
- **Self-documenting**: Code structure reflects API structure
- **Refactoring safety**: Changes propagate automatically

### Why Lock-Free Patterns?

- **Performance**: Eliminates lock contention on hot paths
- **Scalability**: Better utilization of multi-core systems
- **Simplicity**: Reduces complexity of concurrent code
- **Reliability**: Fewer deadlock and race condition opportunities

This architecture ensures HyperSpot can handle production workloads while maintaining developer productivity and system reliability. The ModKit runtime provides a solid foundation that scales from development to production with excellent observability and maintainability.