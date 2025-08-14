# ModKit — Out-of-Process (OoP) Modules Proposal

This document specifies how **Out-of-Process modules** integrate with ModKit: process model, lifecycle, REST registration & routing via the **Ingress bridge**, typed client exposure through **ClientHub**, configuration, security, observability, and performance notes (incl. SSE/streaming).

The design preserves a **single HTTP surface** (the ingress), while allowing modules to run in **separate OS processes** that communicate over **gRPC** (preferably over **Unix Domain Sockets**) with low overhead and strong isolation.

---

## 1) Goals & Non-Goals

### Goals

* OoP modules run in separate processes, but look like normal modules to the runtime:

  * they can **register REST** operations,
  * expose a **typed client** via ClientHub,
  * participate in **lifecycle** (start/stop/status).
* No HTTP servers in modules: **only gRPC** to the ingress & runtime.
* **Same developer ergonomics** as in-process: the module uses `OperationBuilder` and the same handler shape; consumers use the same typed client traits.
* **Hot swap** OoP module binaries when their wire API is backward-compatible (no main runtime rebuild).

### Non-Goals

* Cross-host cluster orchestration. (Single-host, multi-process is the baseline; you can still point the gRPC endpoint at a remote host if you want.)
* Replacing your package manager / process supervisor. (We provide hooks for restart policy; you can wire your supervisor of choice.)

---

## 2) High-Level Architecture

```
            +-------------------------+
HTTP        |       API Ingress       |
/SSE        |  Axum Router + OpenAPI  |
            |  REST <-> gRPC Bridge   |
            +-----------+-------------+
                        | gRPC (UDS preferred)
                        v
            +-------------------------+
            |    OoP Module Process   |
            |  RestBridge gRPC server |
            |  domain / infra / svc   |
            +-------------------------+

            +-------------------------+
            |  Runtime (main process) |
            |  Registry + ClientHub   |
            |  ProxyModule (spawns)   |
            +-------------------------+
```

**Data flows**

* **Registration**: OoP module connects to ingress over gRPC and **pushes** its OpenAPI/route spec and handler bindings.
* **Invocation**: Ingress receives HTTP, resolves the `operation_id`, and **forwards** the request to the OoP module over gRPC; replies (or streams) back to the client.
* **Lifecycle**: The **ProxyModule** (in main process) spawns the OoP binary, watches it, relays stop/kill, pipes stdout/stderr to logs, reports status.
* **Typed Client**: Proxy publishes a **gRPC-backed client implementation** into ClientHub; consumers use the same trait they would for an in-process module.

---

## 3) Process Roles

### 3.1 API Ingress (unchanged surface)

* Owns the **only HTTP server**.
* Maintains **OpenAPI** and the **operation registry**.
* Exposes a gRPC service (**RestRegistry**, **RestInvoke**) for OoP modules:

  * **RegisterRoutes**(ModuleSpec) / **Unregister**(module).
  * **UnaryCall** / **ServerStreamCall** (SSE/WebSocket-like pushes).
  * Optional **Health** pings.

### 3.2 OoP Module Process

* Starts a **gRPC server** (RestBridge) and a **gRPC client** to ingress.
* Builds routes with **the same `OperationBuilder`**; uses a **Remote OpenAPI registry** that forwards spec/ops to ingress via gRPC.
* Registers **operation handlers** (function pointers/closures) in a local dispatch table keyed by `operation_id`.
* On incoming gRPC invocation from ingress, it:

  1. parses path/query/body/headers into DTOs,
  2. executes the handler,
  3. returns JSON/body (or streams bytes for SSE).

### 3.3 ProxyModule (main process)

* Implements `Module` & `StatefulModule`; **spawns** the OoP binary, sets env, limits, and socket paths.
* Pipes **stdout/stderr** to structured logs.
* Creates and **publishes** a **gRPC client** (gateway) for the module into **ClientHub**:

  * Local consumers call the trait; the gateway forwards to OoP.
* Reports status (maps child process health & readiness to ModKit `Status`).

---

## 4) Wire Protocols (gRPC)

### 4.1 RestRegistry (ingress server, module client)

```proto
service RestRegistry {
  rpc RegisterRoutes(RegisterRequest) returns (RegisterReply);
  rpc Unregister(UnregisterRequest) returns (google.protobuf.Empty);
  rpc Heartbeat(HeartbeatRequest) returns (HeartbeatReply); // optional
}

message RegisterRequest {
  string module_name = 1;
  string version     = 2; // module's semver
  bytes  openapi     = 3; // JSON/YAML OpenAPI
  repeated OperationBinding bindings = 4;
}

message OperationBinding {
  string operation_id = 1;
  // Dispatch mode: unary or server_stream
  enum Kind { KIND_UNSPECIFIED = 0; UNARY = 1; SERVER_STREAM = 2; }
  Kind kind = 2;
  // Optional: custom timeouts, auth scope, etc.
  uint32 timeout_ms = 3;
}

message RegisterReply {}
```

### 4.2 RestInvoke (ingress client, module server)

```proto
service RestInvoke {
  rpc UnaryCall(UnaryRequest) returns (UnaryResponse);
  rpc ServerStreamCall(StreamRequest) returns (stream StreamChunk);
}

message Header { string name = 1; bytes value = 2; }

message UnaryRequest {
  string operation_id = 1;
  string method       = 2; // GET/POST...
  string path         = 3; // fully formatted
  repeated Header headers = 4;
  bytes  body         = 5; // raw JSON or other content-type
  // Optional structured fields: path_params, query, content_type…
}

message UnaryResponse {
  uint32 status = 1;
  repeated Header headers = 2;
  bytes body = 3;
  // For JSON, we pass bytes; ingress sets content-type accordingly.
}

message StreamRequest {
  string operation_id = 1;
  string path         = 2;
  repeated Header headers = 3;
  bytes body = 4;
}

message StreamChunk {
  // Use prost with bytes=“bytes” on the Rust side for zero-copy.
  bytes chunk = 1;
  bool  is_end = 2;
  // Optionally: event/id/retry for SSE framing, or send plain bytes.
}
```

> **SSE:** Ingress converts `StreamChunk` into `text/event-stream` chunks.
> Use `bytes::Bytes` in Rust to avoid copies (`#[prost(bytes="bytes")]`).

---

## 5) Developer Ergonomics

### 5.1 Same `OperationBuilder` in OoP

Provide a **remote OpenAPI registry** that implements `OpenApiRegistry` and forwards `register_schema`/`register_operation` to ingress via gRPC. The same routing code runs, only **handlers are bound locally**.

```rust
// In OoP module:
pub fn register_routes(
    mut reg: RemoteOpenApiRegistry,          // wraps the gRPC client
    disp: &mut HandlerDispatcher,            // operation_id -> fn
) -> anyhow::Result<()> {
    use modkit::api::OperationBuilder;

    reg.register_schema("Item", schemars::schema_for!(dto::Item));

    OperationBuilder::get("/items/{id}")
        .operation_id("mymodule.get_item")
        .json_response(200, "OK")
        .handler(disp.bind("mymodule.get_item", handlers::get_item)) // <-- local fn
        .register_remote(&mut reg);

    Ok(())
}
```

On `register_remote`, the builder pushes the **OperationBinding** to ingress (via `RestRegistry::RegisterRoutes`), while `disp.bind` stores a pointer so the OoP **RestInvoke** server can dispatch calls.

### 5.2 Handlers (unchanged)

Handlers remain **Axum-style** functions taking domain state and extracting DTOs. For OoP, your RestBridge wraps/unwraps Axum extractors from the gRPC `UnaryRequest`/`StreamRequest`.

### 5.3 SSE/Streaming

Use `.server_stream_handler(...)` or `.streaming()` in your builder to indicate **server streaming**; your handler yields `Bytes`. The bridge batches appropriately and ingress emits SSE.

---

## 6) ClientHub Integration (consumers)

The proxy publishes a **typed client** into ClientHub so other modules can consume it exactly like an in-process client.

```rust
// contract::client.rs in provider module
#[async_trait::async_trait]
pub trait MyModuleApi: Send + Sync {
    async fn get_item(&self, id: u64) -> anyhow::Result<contract::model::Item>;
}

// gateways/grpc.rs (in proxy)
pub struct MyModuleGrpcClient { inner: tonic::client::MyApiClient<Channel> }
#[async_trait::async_trait]
impl contract::client::MyModuleApi for MyModuleGrpcClient {
    async fn get_item(&self, id: u64) -> anyhow::Result<Item> {
        let resp = self.inner.get_item(proto::GetItemReq{ id }).await?;
        Ok(Item::try_from(resp.into_inner())?)
    }
}

// In ProxyModule::init
let api: Arc<dyn contract::client::MyModuleApi> =
    Arc::new(gateways::grpc::MyModuleGrpcClient::new(channel));
expose_my_module_api(ctx, &api)?;
```

**Consumers**:

```rust
// either generic:
let api = ctx.client_hub().get::<dyn mymodule::contract::client::MyModuleApi>().await?;

// or with generated helper:
let api = my_module_client(ctx.client_hub()).await;

let item = api.get_item(42).await?;
```

---

## 7) Lifecycle & Proxying

### 7.1 Status model (unchanged)

* `Stopped → Starting → Running → Stopping → Stopped`
* **Ready gating**: in proxy, the state stays **Starting** until the OoP module either:

  * calls `ready.notify()` over a small control RPC (or
  * completes `RegisterRoutes` successfully).

### 7.2 ProxyModule responsibilities

* Spawn binary (path from config), set **UDS path** / `PORT` / env, resource limits (ulimits/cgroups).
* Pipe **stdout/stderr** to the runtime logger (tagged with module name).
* Maintain a **CancellationToken**; on stop:

  * send a **Terminate** control signal (or cancel over gRPC),
  * wait `stop_timeout`, then kill if needed.
* Watch child exit code; expose status.

### 7.3 Module process (OoP) main

* Initializes domain/infra just like a normal module.
* Starts **RestBridge gRPC server**.
* Pushes **RegisterRoutes** to ingress.
* Optionally heartbeats.

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load config, build domain/infrastructure...
    let (bridge, registry) = rest_bridge::new().await?;
    routes::register_routes(registry.clone(), bridge.dispatcher_mut())?;

    // Connect to ingress and register routes/OpenAPI
    registry.flush_to_ingress().await?;

    // Signal readiness back to proxy (optional)
    control_client.ready().await?;

    bridge.serve_until_shutdown().await
}
```

---

## 8) Configuration

### 8.1 ProxyModule config (in main runtime)

```yaml
modules:
  mymodule:
    mode: "oop"
    binary: "/opt/mymodule/bin/mymodule"
    grpc_socket: "/run/modkit/mymodule.sock"   # UDS; or grpc_addr: "127.0.0.1:50071"
    restart: "on-failure"                      # never | on-failure | always
    stop_timeout: "30s"
    env:
      RUST_LOG: "info"
    limits:
      cpu_quota: "500m"
      mem_limit: "512Mi"
```

### 8.2 OoP module config (in its own process)

```yaml
server:
  bridge_socket: "/run/modkit/mymodule.sock"   # where to serve RestInvoke
ingress:
  registry_socket: "/run/modkit/ingress.sock"  # where to call RestRegistry
database:
  url: "postgres://..."
```

> DB credentials are passed/loaded **in the module process**; the module uses `DbHandle::connect(url, opts)` as usual.

---

## 9) Security

* Prefer **UDS** for both directions (ingress ⇄ module, proxy ⇄ module).
* If TCP, use **mTLS** with per-module certs.
* Ingress validates:

  * module identity (socket path, client cert),
  * **allowed prefixes/tags** for `operation_id` (policy).
* Optionally sign `RegisterRoutes` with a shared key per module.

---

## 10) Observability

* **Tracing context**: ingress forwards `traceparent`/`tracestate` in gRPC metadata; module attaches it to spans.
* **Metrics**: expose per-operation latency/errcount on both sides.
* **Logs**: child `stdout/stderr` captured by proxy; optionally, module exports structured logs over a side channel.

---

## 11) Versioning & Deployability

* **Typed client trait (contract crate)**: if it changes and a local consumer uses it → you must rebuild the main runtime (as with in-process).
* **Wire schema (gRPC)** for **RestRegistry/RestInvoke**: keep **backward-compatible**; then you can hot-swap OoP binaries without rebuilding ingress/runtime.
* **REST surface only changes**: since ingress stores OpenAPI and handler bindings **per module**, adding new operations in an OoP module requires **no main runtime rebuild**.

---

## 12) Performance Notes (SSE included)

* **UDS** + **`bytes::Bytes`** in protobuf → near zero-copy for chunks.
* Batch small fragments (0.5–4 KB) to amortize HTTP/2 framing.
* Bounded channels and cancellation propagation protect memory.
* Typical overhead vs. direct in-process SSE is **small** (often ≪10% CPU), with the benefit of isolation and hot-swap.

---

## 13) API & Code Sketches

### 13.1 Remote openapi/operation registry (OoP side)

```rust
pub struct RemoteOpenApiRegistry {
    registry: tonic::client::RestRegistryClient<Channel>,
    module_name: String,
    openapi: openapi::Doc,
    bindings: Vec<OperationBinding>,
}

impl OpenApiRegistry for RemoteOpenApiRegistry {
    fn register_schema(&mut self, name: &str, schema: schemars::schema::RootSchema) {
        self.openapi.add_schema(name, schema);
    }
    fn register_operation(&mut self, op: &OperationSpec) {
        self.openapi.add_operation(op);
        self.bindings.push(OperationBinding::from(op));
    }
}

impl RemoteOpenApiRegistry {
    pub async fn flush_to_ingress(&self) -> anyhow::Result<()> {
        self.registry.register_routes(RegisterRequest {
            module_name: self.module_name.clone(),
            version: env!("CARGO_PKG_VERSION").into(),
            openapi: self.openapi.to_json_bytes(),
            bindings: self.bindings.clone().into_proto(),
        }).await?;
        Ok(())
    }
}
```

### 13.2 Handler dispatcher (OoP side)

```rust
pub struct HandlerDispatcher {
    map: dashmap::DashMap<String, Handler>,
}
enum Handler {
    Unary(fn(HttpRequest) -> Pin<Box<dyn Future<Output=HttpResponse> + Send>>),
    Stream(fn(HttpRequest) -> Pin<Box<dyn Stream<Item=Bytes> + Send>>),
}

impl HandlerDispatcher {
    pub fn bind_unary(&self, op: &str, f: impl Fn(HttpRequest)->F + Send + Sync + 'static) { /* store boxed */ }
    pub fn bind_stream(&self, op: &str, f: impl Fn(HttpRequest)->S + Send + Sync + 'static) { /* store boxed */ }
    pub async fn invoke_unary(&self, req: UnaryRequest) -> UnaryResponse { /* ... */ }
    pub async fn invoke_stream(&self, req: StreamRequest) -> impl Stream<Item=StreamChunk> { /* ... */ }
}
```

### 13.3 Ingress bridge handler

Ingress creates generic Axum routes for registered operations:

```rust
// Pseudocode
async fn generic_unary(op_id: &'static str, req: axum::Request<Body>) -> impl IntoResponse {
    let wire = to_unary_request(op_id, req).await?;
    let resp = rest_invoke.unary_call(wire).await?;
    from_unary_response(resp)
}
```

---

## 14) Macro & Config Hooks (optional ergonomics)

Add an optional nested attr to `#[modkit::module]`:

```rust
#[modkit::module(
  name="mymodule",
  caps=[stateful],             // no rest_host/rest here (ingress owns HTTP)
  client="contract::client::MyModuleApi",
  oop(                          // enables ProxyModule behavior
    binary="mymodule",
    grpc_socket="/run/modkit/mymodule.sock",
    stop_timeout="30s",
    restart="on-failure"
  )
)]
pub struct MyModuleProxy;
```

* Expands to a **ProxyModule** that:

  * implements `Module`/`StatefulModule`,
  * spawns the binary,
  * publishes the **gRPC client** in ClientHub.
* OoP module itself is a normal binary crate with `main` as shown above.

*(If you prefer explicit code over macros, you can build the ProxyModule by hand.)*

---

## 15) Security & Policy Controls

* ACL on `operation_id` prefixes per module.
* Resource limits (cpu/mem) on the child process.
* Socket permissions on the UDS path.
* Optional auth token or mTLS cert required to call `RegisterRoutes`.

---

## 16) Testing Strategy

* **Unit tests**: domain & handlers in the OoP module (invoke handlers directly).
* **Bridge tests**: spawn the OoP bridge in-process and call it via tonic.
* **E2E**: start ingress on a test UDS, run the OoP binary, register routes, issue HTTP requests to ingress (including SSE), assert responses.

---

## 17) Migration Guide (from in-process)

1. Move HTTP handlers unchanged.
2. Replace local OpenAPI registry with `RemoteOpenApiRegistry`; call `flush_to_ingress()`.
3. Add RestBridge server to dispatch `operation_id`.
4. Introduce a ProxyModule in the main runtime (or use the macro’s `oop(...)`).
5. Switch ClientHub publishing to the **gRPC gateway** implementation.

No changes for **consumers** of your module’s typed client.

---

## 18) Appendix: SSE Tips

* Use `Bytes` in protobuf to avoid copies.
* Aggregate to 0.5–4 KB chunks when feasible.
* Prefer UDS; if TCP, enable keepalive & tune HTTP/2 windows only if needed.
* Propagate cancellation fast: close SSE → cancel gRPC stream → stop producer.

---

**Bottom line:** OoP modules keep the ModKit programming model intact—**same OperationBuilder, same handlers, same typed clients**—while gaining process-level isolation and independent deployability (within wire-compatibility bounds). The ingress remains your single HTTP entrypoint; modules register REST remotely and stream results efficiently over a thin gRPC bridge.
