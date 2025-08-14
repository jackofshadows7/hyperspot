# HyperSpot Server

HyperSpot Server is a modular, high-performance platform for AI services built in Rust. It provides a comprehensive framework for building scalable AI applications with automatic REST API generation, comprehensive OpenAPI documentation, and a flexible modular architecture.

## 🚀 Quick Start

### Prerequisites

- Rust 1.75+ with Cargo
- Optional: PostgreSQL (can run with SQLite or in-memory database)

### CI/Development Commands

```bash
# Clone the repository
git clone <repository-url>
cd lmstudio-rust

# Unix/Linux/macOS (using Makefile)
make ci       # Run full CI pipeline (linting, tests, security checks)
make fmt      # Format code
make clippy   # Lint code  
make test     # Run tests
make check    # All checks
make audit    # Security audit
make deny     # License and dependency checks

# Windows (using PowerShell script)
./scripts/ci.ps1 check        # Run full CI pipeline
./scripts/ci.ps1 fmt          # Check formatting
./scripts/ci.ps1 fmt -Fix     # Auto-format code
./scripts/ci.ps1 clippy       # Run linter
./scripts/ci.ps1 clippy -Fix  # Auto-fix linter issues
./scripts/ci.ps1 test         # Run tests
./scripts/ci.ps1 audit        # Security audit
./scripts/ci.ps1 deny         # License and dependency checks
```

### Running the Server

```bash
# Option 1: Run with SQLite database (recommended for development)
cargo run --bin hyperspot-server -- --config config/quickstart.yaml run

# Option 2: Run without database (no-db mode)
cargo run --bin hyperspot-server -- --config config/no-db.yaml run

# Option 3: Run with mock in-memory database for testing
cargo run --bin hyperspot-server -- --config config/quickstart.yaml --mock run

# Check if server is ready
curl http://127.0.0.1:8087/health
```

### Example Configuration (config/quickstart.yaml)

```yaml
# HyperSpot Server Configuration

# Core server configuration (global section)  
server:
  home_dir: "~/.hyperspot"

# Database configuration (global section)
database:
  url: "sqlite://database/database.db"
  max_conns: 10
  busy_timeout_ms: 5000

# Logging configuration (global section)
logging:
  default:
    console_level: info
    file: "logs/hyperspot.log"
    file_level: warn
    max_age_days: 28
    max_backups: 3
    max_size_mb: 1000

# Per-module configurations moved under modules section
modules:
  api_ingress:
    bind_addr: "127.0.0.1:8087"
    enable_docs: true
    cors_enabled: false
```

### Smoke Test Examples

```bash
# Start the server in background
cargo run --bin hyperspot-server -- --config config/quickstart.yaml run &
SERVER_PID=$!

# Wait for server to start
sleep 3

# Health check
curl -f http://127.0.0.1:8087/health
# Expected: {"status":"healthy","timestamp":"..."}

# OpenAPI documentation
curl -f http://127.0.0.1:8087/openapi.json | jq '.info.title'
# Expected: "HyperSpot API"

# Interactive docs (in browser)
echo "Open http://127.0.0.1:8087/docs for Stoplight Elements"

# CORS test (if enabled)
curl -f -H "Origin: http://localhost:8080" \
     -H "Access-Control-Request-Method: GET" \
     -H "Access-Control-Request-Headers: Content-Type" \
     -X OPTIONS http://127.0.0.1:8087/health
# Expected: CORS headers in response

# Cleanup
kill $SERVER_PID
```

### Creating Your First Module

```rust
use modkit::*;
use serde::{Deserialize, Serialize};
use axum::{Json, routing::get};
use schemars::JsonSchema;
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct MyResource {
    pub id: u64,
    pub name: String,
    pub description: String,
}

#[modkit::module(
    name = "my_module",
    deps = [],
    caps = [rest]
)]
#[derive(Clone, Default)]
pub struct MyModule;

#[async_trait]
impl Module for MyModule {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        tracing::info!("My module initialized");
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl RestfulModule for MyModule {
    fn register_rest(
        &self,
        _ctx: &ModuleCtx,
        router: axum::Router,
        openapi: &mut dyn modkit::api::OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        use modkit::api::OperationBuilder;
        
        // Register schema for OpenAPI documentation
        openapi.register_schema("MyResource", schemars::schema_for!(MyResource));
        
        // GET /my-resources - List all resources
        let router = OperationBuilder::get("/my-resources")
            .operation_id("my_module.list")
            .summary("List all resources")
            .description("Retrieve a list of all available resources")
            .tag("my_module")
            .json_response(200, "List of resources")
            .json_response(500, "Internal server error")
            .handler(get(list_resources_handler))
            .register(router, openapi);
            
        Ok(router)
    }
}

async fn list_resources_handler() -> Json<Vec<MyResource>> {
    Json(vec![
        MyResource { 
            id: 1, 
            name: "Resource 1".to_string(),
            description: "First resource".to_string()
        }
    ])
}
```

## 📖 Documentation

- **[Architecture Guide](docs/ARCHITECTURE.md)** - System design, module lifecycle, and data flow
- **[Module Development Guide](docs/MODKIT_UNIFIED_SYSTEM.md)** - How to create modules with the ModKit framework
- **[API Ingress](docs/API_INGRESS.md)** - HTTP routing and OpenAPI documentation
- **[Lock-Free Patterns](docs/LOCK_FREE_PATTERNS.md)** - Performance optimization patterns
- **[Contributing](CONTRIBUTING.md)** - Development workflow and coding standards

## 🏗️ Key Features

### Modular Architecture
- **Auto-discovery**: Modules register automatically via the `#[modkit::module]` macro
- **Dependency Management**: Topological sorting ensures proper initialization order
- **Lifecycle Management**: Standardized phases (Init → DB → REST → Start → Stop)
- **Type-Safe Integration**: Compile-time guarantees for module contracts

### ModKit Runtime
- **Unified Startup**: Single `modkit::runtime::run()` function manages entire lifecycle
- **Database Integration**: Automatic connection pooling and migration management
- **Configuration Provider**: Hierarchical YAML configuration with environment overrides
- **Graceful Shutdown**: Proper cleanup with cancellation tokens

### Type-Safe API Development
- **Type-Safe Builder**: Compile-time guarantees with `OperationBuilder`
- **Automatic OpenAPI**: Generate documentation from Rust types
- **Schema Components**: Reusable type definitions with `schemars`
- **Handler Integration**: Direct Axum handler attachment

### Production Ready
- **Graceful Shutdown**: Proper cleanup with cancellation tokens
- **Error Handling**: Comprehensive error propagation with `anyhow`
- **Observability**: Structured logging with `tracing`
- **Configuration**: Flexible YAML-based configuration with environment overrides

### Developer Experience
- **Fast Development**: Hot reloading and quick iteration
- **Interactive Docs**: Stoplight Elements at `/docs` (CDN by default; embedded with `--features embed_elements`)
- **Health Checks**: Built-in `/health` endpoints
- **Type Safety**: Compile-time guarantees for API contracts

## 🔧 Configuration

### YAML Configuration Structure

```yaml
# config/server.yaml

# Global server configuration
server:
  home_dir: "~/.hyperspot"

# Database configuration
database:
  url: "sqlite://database/database.db"
  max_conns: 10
  busy_timeout_ms: 5000

# Logging configuration
logging:
  default:
    console_level: info
    file: "logs/hyperspot.log"
    file_level: warn
    max_age_days: 28
    max_backups: 3
    max_size_mb: 1000

# Module-specific configuration
modules:
  api_ingress:
    bind_addr: "127.0.0.1:8087"
    enable_docs: true
    cors_enabled: true
```

### Environment Variable Overrides

Configuration supports environment variable overrides with `HYPERSPOT_` prefix:

```bash
export HYPERSPOT_DATABASE_URL="postgres://user:pass@localhost/db"
export HYPERSPOT_MODULES_API_INGRESS_BIND_ADDR="0.0.0.0:8080"
export HYPERSPOT_LOGGING_DEFAULT_CONSOLE_LEVEL="debug"
```

## 🧪 Testing

```bash
# Run all tests
cargo test

# Run specific module tests
cargo test -p api_ingress
cargo test -p modkit

# Integration tests with database
cargo test --test integration

# Check compilation without running
cargo check
```

## 📦 Project Structure

```
├── apps/
│   └── hyperspot-server/         # Main server application with ModKit runtime
├── modules/
│   └── api_ingress/              # HTTP routing and OpenAPI documentation
├── libs/
│   ├── modkit/                   # Core ModKit framework and traits
│   ├── db/                       # Database abstraction layer
│   └── runtime/                  # Runtime utilities and configuration
└── config/                       # Configuration files
    ├── quickstart.yaml           # Development configuration with SQLite
    ├── server.yaml               # Full production configuration
    └── no-db.yaml                # No-database mode configuration
```

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes with tests
4. Run `cargo fmt` and `cargo clippy`
5. Commit changes (`git commit -am 'Add amazing feature'`)
6. Push to branch (`git push origin feature/amazing-feature`)
7. Open a Pull Request

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed guidelines.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🎯 Roadmap

- [ ] gRPC support for module communication
- [ ] Distributed tracing integration
- [ ] Kubernetes deployment manifests
- [ ] WebSocket support in API gateway
- [ ] Module marketplace and registry
- [ ] Out-of-process module support

## 🏃‍♂️ Performance

HyperSpot is designed for high performance:

- **Lock-free routing**: Efficient request dispatching with `DashMap` and `ArcSwap`
- **Async-first**: Built on Tokio for scalable concurrency
- **Type-safe hot paths**: Zero runtime type checks with compile-time guarantees
- **Fast serialization**: Optimized JSON handling with `serde`
- **Connection pooling**: Efficient database connection reuse

Benchmarks show excellent performance for production workloads with minimal resource overhead.

## 🌟 Getting Started Tutorial

1. **Clone and Setup**:
   ```bash
   git clone <repository-url>
   cd lmstudio-rust
   cargo build
   ```

2. **Run Development Server**:
   ```bash
   cargo run --bin hyperspot-server -- --config config/quickstart.yaml run
   ```

3. **Explore the API**:
   - Visit http://127.0.0.1:8087/docs for interactive documentation
   - Check health at http://127.0.0.1:8087/health

4. **Create Your First Module**: Follow the module creation example above

5. **Add to Configuration**: Update `config/quickstart.yaml` to include your module

The ModKit framework provides everything you need to build scalable, maintainable AI services with excellent developer experience and production-ready features out of the box.