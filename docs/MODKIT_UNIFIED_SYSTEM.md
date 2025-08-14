# ModKit — Architecture & Developer Guide (DDD-light)

This guide shows how to build production-grade modules on **ModKit**: how to lay out your module repo, declare modules with a macro, wire REST, publish typed clients, and run background services with a clean lifecycle. It also explains the rationale behind our DDD-light layering.

---

## What ModKit gives you

* **Plugin modules** discovered via `inventory`, initialized in dependency order.
* **Ingress as a module** (e.g., `api_ingress`) that owns Axum router & OpenAPI.
* **Type-safe REST** with a builder that prevents half-wired operations.
* **Typed ClientHub** for in-process clients (fetch by **interface type** + optional **scope**).
* **Lifecycle** helpers & wrappers for long-running tasks and graceful shutdown.
* **Lock-free hot paths** via atomic `Arc` swaps for read-mostly state.

---

## Canonical layout (DDD-light)

Place each module under `modules/rust/<name>/`:

```
modules/rust/<name>/
  ├─ src/
  │  ├─ lib.rs                       # module declaration, exports
  │  ├─ module.rs                    # main struct + Module/Db/Rest/Stateful impls
  │  ├─ config.rs                    # typed config (optional)
  │  ├─ contract/                    # public API surface (for other modules)
  │  │  ├─ mod.rs
  │  │  ├─ client.rs                 # traits for ClientHub (and DTOs)
  │  │  ├─ model.rs                  # DTOs exposed to other modules (no REST specifics)
  │  │  └─ error.rs
  │  ├─ domain/                      # internal business logic
  │  │  ├─ mod.rs
  │  │  ├─ model.rs                  # rich domain models
  │  │  ├─ error.rs
  │  │  └─ service.rs                # orchestration/business rules
  │  ├─ infra/                       # “low-level”: DB, system, IO, adapters
  │  │  ├─ storage/
  │  │  │  ├─ entity.rs              # e.g., SeaORM entities / SQL mappings
  │  │  │  ├─ mapper.rs              # entity <-> domain mapping
  │  │  │  └─ migrations/            # DB migrations live here
  │  │  │     ├─ mod.rs
  │  │  │     └─ initial_001.rs
  │  │  └─ (anything system/platform: files, CPU/GPU probes, processes, ...)
  │  ├─ gateways/                    # client implementations for ClientHub
  │  │  └─ local.rs                  # local (in-process) client impl
  │  └─ api/
  │     └─ rest/
  │        ├─ dto.rs                 # HTTP DTOs (serde/schemars) ← REST-only types
  │        ├─ handlers.rs            # Axum handlers (web controllers)
  │        └─ routes.rs              # route & OpenAPI registration (OperationBuilder)
  ├─ spec/
  │  └─ proto/                       # proto files (if present)
  └─ Cargo.toml
```

Notes:

* **Handlers may call `domain::service` directly**.
* For simple internal modules you **may re-export** domain models through `contract::model` for convenience.
* **Gateways** hold client implementations (e.g., “local”). Only the trait & DTOs live in `contract`.
* Infra may use SeaORM or **raw SQL** (SQLx or your choice).

---

## ModuleCtx (what you get at runtime)

```rust
pub trait ConfigProvider: Send + Sync {
    /// Returns raw JSON section for the module, if any.
    fn get_module_config(&self, module_name: &str) -> Option<&serde_json::Value>;
}

#[derive(Clone)]
pub struct ModuleCtx {
    pub(crate) db: Option<std::sync::Arc<db::DbHandle>>,
    pub(crate) config_provider: Option<std::sync::Arc<dyn ConfigProvider>>,
    pub(crate) client_hub: std::sync::Arc<crate::client_hub::ClientHub>,
    pub(crate) cancellation_token: tokio_util::sync::CancellationToken,
    pub(crate) module_name: Option<std::sync::Arc<str>>,
}
```

### How to use it

**Typed config:**

```rust
#[derive(serde::Deserialize, Default, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct MyModuleConfig { /* fields */ }
```

**DB access (SeaORM / SQLx via unified handle):**

```rust
let sea = db.seaorm();      // SeaORM connection
// or:
let pool = db.sqlx_pool();  // SQLx pool
```

**Clients (publish & consume):**

```rust
// publish (provider module, in init()):
expose_my_module_client(&ctx, &api)?;

// consume (consumer module, in init()):
let api = my_module_client(&ctx.client_hub);
// or without helpers:
let api = ctx.client_hub.get::<dyn my_module::contract::client::MyModuleApi>()?;
```

**Cancellation:**

```rust
let child = ctx.cancellation_token.child_token();
// pass `child` into your background tasks for cooperative shutdown
```

---

## Declarative module registration — `#[modkit::module(...)]`

Attach the attribute to your main struct. The macro:

* Adds inventory entry for auto-discovery.
* Registers **name**, **deps**, **caps** (capabilities).
* Instantiates your type via `ctor = <expr>` or **`<T as Default>::default()`** if `ctor` is omitted.
* Optionally emits **ClientHub helpers** if you declare a `client` trait (see below).
* Optionally wires **lifecycle** when you add `lifecycle(...)` inside the macro.

### Full syntax

```rust
#[modkit::module(
    // Required:
    name = "my_module",

    // Optional:
    deps = ["db", "foo"],

    // Capabilities your type implements:
    caps = [db, rest, stateful, /* rest_host if you own the HTTP server */],

    // If you expose a typed API to other modules:
    client = "contract::client::MyModuleApi",

    // Custom constructor expression (otherwise uses Default):
    ctor = MyModule::new(),

    // Lifecycle glue (see next section):
    lifecycle(entry = "serve", stop_timeout = "30s", await_ready)
)]
pub struct MyModule { /* fields */ }
```

### Capabilities

* `db` → you implement `DbModule` (migrations / schema setup).
* `rest` → you implement `RestfulModule` (register routes synchronously).
* `rest_host` → you own the final Axum server/OpenAPI (e.g., `api_ingress`).
* `stateful` → the module has a background job:

  * With `lifecycle(...)`, the macro generates `Runnable` and registers `WithLifecycle<Self>`.
  * Without `lifecycle(...)`, you implement `StatefulModule` yourself.

### Client helpers (when `client` is set)

Generated helpers (synchronous):

* `expose_<module>_client(ctx, &Arc<dyn Trait>) -> anyhow::Result<()>`
* `expose_<module>_client_in(ctx, scope: &str, &Arc<dyn Trait>) -> anyhow::Result<()>`
* `<module>_client(hub: &ClientHub) -> Arc<dyn Trait>`
* `<module>_client_in(hub: &ClientHub, scope: &str) -> Arc<dyn Trait>`

These wrap the `ClientHub` under the hood.

---

## Lifecycle — macro attributes & state machine

ModKit provides a ready-to-use lifecycle with a small state machine and cancellation semantics.

### Inside `#[modkit::module(...)]`

```rust
#[modkit::module(
    name = "api_ingress",
    caps = [rest_host, rest, stateful],
    lifecycle(entry = "serve", stop_timeout = "30s", await_ready)
)]
pub struct ApiIngress { /* ... */ }

impl ApiIngress {
    // accepted signatures:
    // 1) async fn serve(self: Arc<Self>, cancel: CancellationToken) -> Result<()>
    // 2) async fn serve(self: Arc<Self>, cancel: CancellationToken, ready: ReadySignal) -> Result<()>
    async fn serve(
        self: std::sync::Arc<Self>,
        cancel: tokio_util::sync::CancellationToken,
        ready: modkit::lifecycle::ReadySignal
    ) -> anyhow::Result<()> {
        // bind sockets/resources before flipping to Running
        ready.notify();
        cancel.cancelled().await;
        Ok(())
    }
}
```

* `entry` — async method to run.
* `stop_timeout` — graceful stop timeout (`ms`, `s`, `m`, `h`).
* `await_ready` — if set, module stays **Starting** until `ready.notify()`.

The macro generates `impl Runnable` and registers `WithLifecycle<Self>` as the `stateful` capability. It also adds `into_module(self) -> WithLifecycle<Self>`.

### As an impl-block attribute

```rust
#[modkit::lifecycle(method = "serve", stop_timeout = "30s", await_ready = true)]
impl ApiIngress {
    async fn serve(self: Arc<Self>, cancel: CancellationToken, ready: ReadySignal) -> Result<()> {
        // ...
        Ok(())
    }
}
```

### Manual lifecycle (no macros)

```rust
use modkit::lifecycle::{WithLifecycle, Runnable, ReadySignal};

#[async_trait::async_trait]
impl Runnable for MyWorker {
    async fn run(self: Arc<Self>, cancel: CancellationToken) -> anyhow::Result<()> {
        Ok(())
    }
}

let stateful = WithLifecycle::new(MyWorker::new())
    .with_stop_timeout(std::time::Duration::from_secs(30))
    .with_ready_mode(false, false, None);
```

### States & transitions

```
Stopped ── start() ─▶ Starting ──(await_ready? then ready.notify())──▶ Running
   ▲                                  │
   │                                  └─ if await_ready = false → Running immediately after spawn
   └──────────── stop()/cancel ────────────────────────────────────────────────┘
```

* **Stopped**: no task is running.
* **Starting**: task spawned; if `await_ready`, waiting for `ready.notify()`.
* **Running**: serving/working.
* **Stopping**: graceful shutdown requested; cancellation token fired.
* On exit, state returns to **Stopped**.

`WithLifecycle::stop()` waits up to `stop_timeout`:

* Returns **Finished** if the task ended by itself.
* Returns **Cancelled** if cancelled and then ended.
* Returns **Timeout** and **aborts** the task if it didn’t stop in time.

> Tip: do not hold mutex guards across `await` in your entry method. Take what you need, drop the guard, then `await`. This keeps the future `Send` under executors that move tasks.

---

## Contracts & lifecycle traits

```rust
#[async_trait::async_trait]
pub trait Module: Send + Sync + 'static {
    async fn init(&self, ctx: &crate::context::ModuleCtx) -> anyhow::Result<()>;
    fn as_any(&self) -> &dyn std::any::Any;
}

#[async_trait::async_trait]
pub trait DbModule: Send + Sync {
    async fn migrate(&self, db: &db::DbHandle) -> anyhow::Result<()>;
}

pub trait RestfulModule: Send + Sync {
    fn register_rest(
        &self,
        ctx: &crate::context::ModuleCtx,
        router: axum::Router,
        openapi: &mut dyn crate::api::OpenApiRegistry,
    ) -> anyhow::Result<axum::Router>;
}

#[async_trait::async_trait]
pub trait StatefulModule: Send + Sync {
    async fn start(&self, cancel: tokio_util::sync::CancellationToken) -> anyhow::Result<()>;
    async fn stop(&self, cancel: tokio_util::sync::CancellationToken) -> anyhow::Result<()>;
}
```

**Order:** `init → migrate → register_rest → start → stop` (topologically sorted by `deps`).

---

## REST with `OperationBuilder`: routes, schemas, requests, responses, state

`OperationBuilder` is a type-state builder that **won’t compile** unless you set both a **handler** and at least one **response** before calling `register()`. It also lets you attach request bodies and component schemas.

### Quick reference

**Constructors**

```rust
OperationBuilder::<Missing, Missing, S>::get("/path")
OperationBuilder::<Missing, Missing, S>::post("/path")
put/patch/delete are available too
```

**Describe**

```rust
.operation_id("module.op")
.summary("Short summary")
.description("Longer description")
.tag("group")
.path_param("id", "ID description")
.query_param("q", /*required=*/false, "Query description")
```

**Request body (JSON)**

```rust
// Auto-register schema for T with schemars; with/without description:
.json_request::<T>(openapi, "body description")
.json_request_no_desc::<T>(openapi)

// Reference a named schema you registered yourself; with/without description:
.json_request_schema("MySchema", "body description")
.json_request_schema_no_desc("MySchema")
```

**Handler / method router**

```rust
.handler(my_function_handler)         // preferred: free functions using State<S>
.method_router(my_method_router)      // advanced: attach layers/middleware per route
```

**Responses**

```rust
// First response (moves state Missing -> Present):
.json_response(200, "OK")
.text_response(400, "Bad request")
.html_response(200, "HTML")

// Schema-aware variants (auto-register T):
.json_response_with_schema::<T>(openapi, 200, "OK with schema")

// Additional responses (stay in Present state):
.json_response(404, "Not found")
```

**Register**

```rust
.register(router, openapi) -> Router<S>
```

### Using Router state (`S`) to avoid clones

Pass a single state value once via `Router::with_state(S)`. Your handlers are **free functions** that take `State<S>`, so you don’t capture/clone your service for each route.

```rust
// api/rest/routes.rs
use axum::{Router, extract::State};
use serde::{Serialize, Deserialize};
use schemars::JsonSchema;
use modkit::api::OperationBuilder;

#[derive(Clone)]
pub struct ApiState {
    pub svc: std::sync::Arc<crate::domain::service::Service>,
}

// REST DTOs (REST-only; not in contract::model)
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CreateUserReq { pub email: String, pub name: String }

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct UserDto { pub id: u64, pub email: String, pub name: String }

pub async fn create_user(
    State(st): State<ApiState>,
    axum::Json(req): axum::Json<CreateUserReq>,
) -> Result<axum::Json<UserDto>, axum::http::StatusCode> {
    let m = st.svc.create_user(req.email, req.name).await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(axum::Json(UserDto { id: m.id, email: m.email, name: m.name }))
}

pub async fn get_user(
    State(st): State<ApiState>,
    axum::extract::Path(id): axum::extract::Path<u64>,
) -> Result<axum::Json<UserDto>, axum::http::StatusCode> {
    let m = st.svc.get_user(id).await
        .map_err(|_| axum::http::StatusCode::NOT_FOUND)?;
    Ok(axum::Json(UserDto { id: m.id, email: m.email, name: m.name }))
}

pub fn register_routes(
    router: Router<ApiState>,
    openapi: &mut dyn crate::api::OpenApiRegistry,
) -> Router<ApiState> {
    // register components once (schemars generates JSON Schemas)
    openapi.register_schema("CreateUserReq", schemars::schema_for!(CreateUserReq));
    openapi.register_schema("UserDto", schemars::schema_for!(UserDto));

    // POST /users with request body and schema’d response
    let router = OperationBuilder::<modkit::api::Missing, modkit::api::Missing, ApiState>::post("/users")
        .operation_id("users_info.create_user")
        .summary("Create a new user")
        .json_request::<CreateUserReq>(openapi, "Create-user payload")
        .handler(create_user)
        .json_response_with_schema::<UserDto>(openapi, 201, "Created user")
        .json_response(400, "Invalid input")
        .json_response(409, "Email already exists")
        .register(router, openapi);

    // GET /users/{id}
    let router = OperationBuilder::<_, _, ApiState>::get("/users/{id}")
        .operation_id("users_info.get_user")
        .summary("Get user by id")
        .path_param("id", "User id")
        .handler(get_user)
        .json_response_with_schema::<UserDto>(openapi, 200, "User")
        .json_response(404, "Not found")
        .register(router, openapi);

    router
}
```

**In your module’s `register_rest`:**

```rust
impl crate::contracts::RestfulModule for MyModule {
    fn register_rest(
        &self,
        _ctx: &crate::context::ModuleCtx,
        router: axum::Router,
        openapi: &mut dyn crate::api::OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        let svc = self.service.load().as_ref().unwrap().clone();
        let api_state = crate::api::rest::routes::ApiState { svc };
        let router = router.with_state(api_state);
        Ok(crate::api::rest::routes::register_routes(router, openapi))
    }
}
```

**Why free functions (not per-route closures)?**

* Free functions with `State<S>` are easy to test and don’t capture/clone service per route.
* Your **state** can aggregate multiple dependencies (services, repositories, config).
* For per-route middleware, use `.method_router(...)` and attach layers to that `MethodRouter<S>` before `register()`.

---

## Typed ClientHub (publish & consume)

* **`contract::client`** defines the trait & DTOs exposed to other modules.
* **`gateways/local.rs`** implements that trait and is published in `init`.
* Consumers resolve the typed client from ClientHub by **interface type** (+ scope if needed).

**Publish in `init`:**

```rust
#[async_trait::async_trait]
impl Module for MyModule {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        let cfg = ctx.module_config::<crate::config::Config>();
        let svc = std::sync::Arc::new(domain::service::MyService::new(ctx.db.clone(), cfg));
        self.service.store(Some(svc.clone()));

        let api: std::sync::Arc<dyn contract::client::MyModuleApi> =
            std::sync::Arc::new(gateways::local::MyModuleLocalClient::new(svc));

        // Macro-generated helper (also has a scoped variant)
        expose_my_module_client(ctx, &api)?;
        Ok(())
    }
    fn as_any(&self) -> &dyn std::any::Any { self }
}
```

**Consume:**

```rust
let api = my_module_client(&ctx.client_hub);                // via helper
// or:
let api = ctx.client_hub.get::<dyn my_module::contract::client::MyModuleApi>()?; // raw
```

---

## Scaffold blueprint (feedable to an LLM)

Ask your editor/LLM to scaffold:

* Root at `modules/rust/<name>/`
* Create files shown in the layout
* Add `#[modkit::module(...)]` with `name`, `deps`, `caps`, optional `client`, optional `lifecycle(...)`
* Wire `Module::init`, `RestfulModule::register_rest`, optional `DbModule::migrate`
* In `init`, build `domain::service::Service`, keep `Arc<Service>` inside the module (e.g., field or `ArcSwap`)
* In `api/rest/routes.rs`, call `with_state(ApiState { svc })`, register routes via `OperationBuilder`
* Handlers map **domain↔HTTP DTO** in `api/rest/dto.rs`
* If exposing to others, define `contract::client::Trait` + DTOs and implement a **local** client in `gateways/local.rs`
* (Optional) Put proto files under `spec/proto/`

**Template snippet (module head):**

```rust
// src/module.rs
#[modkit::module(
    name = "<name>",
    deps = [],
    caps = [rest],                      // add db/stateful as needed
    client = "contract::client::Api",   // optional
    lifecycle(entry = "serve", await_ready, stop_timeout = "30s") // optional
)]
pub struct <PascalName> {
    service: arc_swap::ArcSwapOption<domain::service::Service>,
}

#[async_trait::async_trait]
impl Module for <PascalName> {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        let cfg = ctx.module_config::<crate::config::Config>();
        let svc = std::sync::Arc::new(domain::service::Service::new(ctx.db.clone(), cfg));
        self.service.store(Some(svc.clone()));
        // let api: Arc<dyn contract::client::Api> = Arc::new(gateways::local::Client::new(svc));
        // expose_<name>_client(ctx, &api)?;
        Ok(())
    }
    fn as_any(&self) -> &dyn std::any::Any { self }
}

impl RestfulModule for <PascalName> {
    fn register_rest(
        &self,
        _ctx: &ModuleCtx,
        router: axum::Router,
        openapi: &mut dyn crate::api::OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        let svc = self.service.load().as_ref().unwrap().clone();
        let state = crate::api::rest::routes::ApiState { svc };
        let router = router.with_state(state);
        Ok(crate::api::rest::routes::register_routes(router, openapi))
    }
}
```

---

## Configuration & environment

* Per-module config lives in your `config.rs` type and is loaded via `ctx.module_config::<T>()`.
* Global YAML can override module sections; env vars (e.g., `HYPERSPOT_MODULES_<NAME>_...`) can override YAML.

---

## Testing

* **Unit test** domain services by mocking infra.
* **REST test** handlers with `Router::oneshot` and stubbed `ApiState`.
* **Integration test** module wiring: call `init`, resolve typed clients from ClientHub, assert behavior.
* For stateful modules, exercise lifecycle: start with a `CancellationToken`, signal shutdown, assert transitions.

---

## Addendum — Rationale (DDD-light)

1. **What does a domain service do?**
   Encodes **business rules/orchestration**. It calls repositories/infrastructure, applies invariants, aggregates data, owns retries/timeouts at the business level.

2. **Where to put “low-level” things?**
   In **infra/** (storage, system probes, processes, files, raw SQL, HTTP to other systems). Domain calls infra via small interfaces/constructors.

3. **Where to keep “glue”?**
   Glue that adapts domain to transport lives in **api/rest** (HTTP DTOs, handlers). Glue that adapts domain to **other modules** lives in **gateways/** (client implementations). DB mapping glue sits in **infra/storage**.

4. **Why not put platform-dependent logic into service?**
   To keep business rules portable/testable. Platform logic churns often; isolating it in infra avoids leaking OS/DB concerns into your domain.

5. **What is `contract` and why separate?**
   It’s the **public API** of your module for **other modules**: traits + DTOs + domain errors safe to expose. This separation allows swapping local/remote clients without changing consumers. For simple internal modules you may re-export a subset of domain models via `contract::model`.

6. **How to hide domain & internals from other modules?**
   Re-export only what’s needed via `contract`. Consumers depend on `contract` and `gateways` through the ClientHub; they never import your domain/infra directly.

---

## Best practices

* Handlers are thin; domain services are cohesive and testable.
* Keep DTO mapping in `api/rest/dto.rs`. Don’t leak HTTP types into domain.
* Prefer `ArcSwap`/lock-free caches for read-mostly state.
* Use `tracing` with module/operation fields.
* Migrations live in `infra/storage/migrations/` and run in `DbModule::migrate`.
