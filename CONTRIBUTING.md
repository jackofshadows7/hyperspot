# Contributing to HyperSpot Server

Thank you for your interest in contributing to HyperSpot Server! This document provides guidelines and information for contributors.

## 🚀 Quick Start

### Prerequisites

- **Rust 1.75+** with Cargo
- **PostgreSQL 12+** (optional, mock database available)
- **Git** for version control
- **Your favorite editor** (VS Code with rust-analyzer recommended)

### Development Setup

```bash
# Clone the repository
git clone <repository-url>
cd lmstudio-rust

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install required components
rustup component add clippy rustfmt

# Build the project
cargo build

# Run tests
cargo test

# Start the development server
cargo run --bin hyperspot-server -- run
```

### Workspace Layout

```
lmstudio-rust/
├── apps/
│   └── hyperspot-server/     # Main server binary
├── modules/
│   ├── api_ingress/          # HTTP gateway and OpenAPI
│   └── sysinfo/              # System information module
├── modkit/                   # Core framework and traits
│   ├── macros/               # Procedural macros
│   └── src/                  # Framework implementation
├── db/                       # Database abstraction layer
├── hyperspot_runtime/        # Runtime utilities and config
├── config/                   # Configuration files
└── docs/                     # Documentation (if present)
```

## 📝 Development Workflow

### 1. Create a Feature Branch

```bash
git checkout -b feature/your-feature-name
```

Use descriptive branch names:
- `feature/user-authentication`
- `fix/memory-leak-in-router`
- `docs/api-gateway-examples`
- `refactor/entity-to-contract-conversions`

### 2. Make Your Changes

Follow the coding standards and patterns described below.

### 3. Run Quality Checks

```bash
# Format code
cargo fmt

# Run linter
cargo clippy -- -D warnings

# Run tests
cargo test

# Check compilation
cargo check --all-targets
```

### 4. Commit Changes

Use clear, descriptive commit messages:

```bash
git add .
git commit -m "feat: add user authentication module

- Implements JWT-based authentication
- Adds login/logout endpoints
- Includes comprehensive tests
- Updates OpenAPI documentation

Closes #123"
```

### 5. Push and Create PR

```bash
git push origin feature/your-feature-name
```

Then create a Pull Request on GitHub with:
- Clear title and description
- Reference to related issues
- Test coverage information
- Breaking changes (if any)

## 🏗️ Architecture Guidelines

### Module Development

#### Creating a New Module

1. **Create module directory**:
   ```bash
   mkdir modules/my_module
   cd modules/my_module
   cargo init --lib
   ```

2. **Define module structure**:
   ```rust
   #[derive(Clone, Default)]
   #[module(name = "my_module", deps = [], capabilities = [core], client = MyModuleClient)]
   pub struct MyModule {
       config: Arc<RwLock<MyModuleConfig>>,
   }
   
   pub trait MyModuleClient: Send + Sync {
       async fn my_operation(&self) -> Result<String>;
   }
   
   impl MyModuleClient for MyModule {
       async fn my_operation(&self) -> Result<String> {
           Ok("Hello from MyModule".to_string())
       }
   }
   ```

3. **Implement required traits**:
   ```rust
   #[async_trait]
   impl Module for MyModule {
       fn name(&self) -> &str { "my_module" }
       fn dependencies(&self) -> Vec<&'static str> { vec![] }
       
       async fn init_ctx(&self, ctx: &ModuleCtx) -> Result<()> {
           // Load configuration, initialize resources
           Ok(())
       }
   }
   ```

#### Module Capabilities

- **Core**: Basic module functionality
- **Stateful**: Has start/stop lifecycle and status
- **Restful**: Provides REST API endpoints

```rust
// Stateful module
impl StatefulModule for MyModule {
    async fn start(&self, cancellation_token: tokio_util::sync::CancellationToken) -> Result<()> { 
        tracing::info!("MyModule starting");
        // Start background services, use cancellation_token for graceful shutdown
        Ok(())
    }
    
    async fn stop(&self, cancellation_token: tokio_util::sync::CancellationToken) -> Result<()> { 
        tracing::info!("MyModule stopping");
        // Graceful cleanup with timeout
        Ok(())
    }
}

// RESTful module using modern type-safe API
impl RestfulModule for MyModule {
    fn register_routes(&self, router: axum::Router<()>, openapi: &mut dyn modkit::api::OpenApiRegistry) -> axum::Router<()> {
        use modkit::api::OperationBuilder;
        
        // Register schemas for OpenAPI documentation
        openapi.register_schema("MyResponse", schemars::schema_for!(MyResponse));
        openapi.register_schema("CreateRequest", schemars::schema_for!(CreateRequest));
        
        // Register with handler for working endpoints
        let router = OperationBuilder::get("/my-endpoint")
            .operation_id("my_module.get")
            .summary("Get my resource")
            .description("Detailed description of what this endpoint does")
            .tag("my_module")
            .json_response(200, "Success")
            .handler(axum::routing::get(my_handler))
            .register(router, openapi);
            
        // Create endpoint with type-safe request/response
        OperationBuilder::post("/my-endpoint")
            .operation_id("my_module.create")
            .summary("Create my resource")
            .description("Create a new resource with the provided data")
            .tag("my_module")
            .json_response(201, "Created")
            .json_response(400, "Invalid input")
            .handler(axum::routing::post(create_my_resource_handler))
            .register(router, openapi)
    }
}
```

### API Design Standards

#### REST Endpoint Guidelines

1. **Use consistent resource naming**:
   - `/users` (plural nouns)
   - `/users/{id}` (path parameters)
   - `/users/{id}/posts` (nested resources)

2. **HTTP methods**:
   - `GET` for reading data
   - `POST` for creating resources
   - `PUT` for updating entire resources
   - `PATCH` for partial updates
   - `DELETE` for removing resources

3. **Status codes**:
   ```rust
   // Success responses
   200 OK              // Successful GET, PUT, PATCH
   201 Created         // Successful POST
   204 No Content      // Successful DELETE
   
   // Client error responses
   400 Bad Request     // Invalid request data
   401 Unauthorized    // Authentication required
   403 Forbidden       // Insufficient permissions
   404 Not Found       // Resource doesn't exist
   409 Conflict        // Resource conflict
   422 Unprocessable   // Validation errors
   
   // Server error responses
   500 Internal Error  // Unexpected server error
   501 Not Implemented // Handler not implemented
   503 Service Unavail // Service temporarily down
   ```

#### Schema Design

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct User {
    /// Unique user identifier
    pub id: u64,
    
    /// Full name of the user
    #[schemars(length(min = 1, max = 100))]
    pub name: String,
    
    /// Email address (must be valid email)
    #[schemars(format = "email")]
    pub email: String,
    
    /// Account creation timestamp
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    
    /// Optional user profile data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<UserProfile>,
}
```

## 🧪 Testing Standards

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_user_creation() {
        let user_service = UserService::new();
        let request = CreateUserRequest {
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
        };
        
        let user = user_service.create_user(request).await.unwrap();
        assert_eq!(user.name, "Alice");
        assert_eq!(user.email, "alice@example.com");
    }
    
    #[test]
    fn test_config_validation() {
        let config = MyModuleConfig {
            timeout: 0, // Invalid
        };
        assert!(config.validate().is_err());
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_api_integration() {
    let api_ingress = ApiIngress::new(ApiIngressConfig::default());
    let my_module = MyModule::new();
    
    // Register endpoints
    let router = axum::Router::new();
    let mut openapi_registry = MockOpenApiRegistry::new();
    let router = my_module.register_routes(router, &mut openapi_registry);
    
    // Test request
    let request = Request::builder()
        .uri("/my-endpoint")
        .body(Body::empty())
        .unwrap();
        
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
```

### Test Coverage

Aim for high test coverage:
- **Unit tests**: Test individual functions and methods
- **Integration tests**: Test module interactions
- **End-to-end tests**: Test complete request flows

```bash
# Run tests with coverage (requires tarpaulin)
cargo install cargo-tarpaulin
cargo tarpaulin --out html
```

## 🎨 Code Style

### Formatting

Use `rustfmt` with the project configuration:

```bash
cargo fmt
```

Key formatting rules:
- **Line length**: 100 characters max
- **Indentation**: 4 spaces (no tabs)
- **Trailing commas**: Required in multi-line expressions
- **Import organization**: Group by source (std, external, internal)

### Naming Conventions

```rust
// Types: PascalCase
struct UserService;
enum ResponseStatus;

// Functions and variables: snake_case
fn create_user() {}
let user_name = "Alice";

// Constants: SCREAMING_SNAKE_CASE
const MAX_RETRIES: u32 = 3;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

// Module names: snake_case
mod user_service;
mod api_client;
```

### Documentation

#### Rustdoc Comments

```rust
/// Service for managing user accounts
/// 
/// Provides CRUD operations for users with proper validation
/// and error handling. All operations are async and thread-safe.
/// 
/// # Examples
/// 
/// ```rust
/// let service = UserService::new(db_pool);
/// let user = service.get_user(123).await?;
/// println!("User: {}", user.name);
/// ```
pub struct UserService {
    /// Database connection pool
    pool: Arc<DbPool>,
}

impl UserService {
    /// Create a new user with the given details
    /// 
    /// # Arguments
    /// 
    /// * `request` - User creation data including name and email
    /// 
    /// # Returns
    /// 
    /// Returns the created user with assigned ID, or an error if
    /// validation fails or the email already exists.
    /// 
    /// # Errors
    /// 
    /// * `ValidationError` - Invalid input data
    /// * `ConflictError` - Email already registered
    /// * `DatabaseError` - Database operation failed
    pub async fn create_user(&self, request: CreateUserRequest) -> Result<User, UserError> {
        // Implementation
    }
}
```

#### Error Documentation

```rust
/// Errors that can occur during user operations
#[derive(Debug, thiserror::Error)]
pub enum UserError {
    /// User input validation failed
    #[error("Validation error: {message}")]
    Validation { message: String },
    
    /// User with given email already exists
    #[error("User with email {email} already exists")]
    Conflict { email: String },
    
    /// Database operation failed
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
```

### Error Handling

Use `anyhow` for application errors and `thiserror` for library errors:

```rust
use anyhow::{Context, Result};

pub async fn process_user(id: u64) -> Result<User> {
    let user = database::get_user(id)
        .await
        .context("Failed to fetch user from database")?;
        
    validate_user(&user)
        .context("User validation failed")?;
        
    Ok(user)
}
```

### Performance Guidelines

1. **Avoid blocking operations** in async context:
   ```rust
   // ❌ Bad: blocking in async
   async fn bad_handler() {
       let data = std::fs::read_to_string("file.txt").unwrap();
   }
   
   // ✅ Good: use async I/O
   async fn good_handler() {
       let data = tokio::fs::read_to_string("file.txt").await?;
   }
   ```

2. **Use efficient data structures**:
   ```rust
   // ❌ Bad: unnecessary allocations
   fn process_items(items: &[Item]) -> Vec<String> {
       items.iter().map(|item| item.name.clone()).collect()
   }
   
   // ✅ Good: avoid clones when possible
   fn process_items(items: &[Item]) -> Vec<&str> {
       items.iter().map(|item| item.name.as_str()).collect()
   }
   ```

3. **Use appropriate synchronization**:
   ```rust
   // Short-lived locks
   use parking_lot::{RwLock, Mutex};
   
   // Long-running async operations
   use tokio::sync::{RwLock as AsyncRwLock, Mutex as AsyncMutex};
   ```

## 🐛 Debugging

### Logging

Use structured logging with `tracing`:

```rust
use tracing::{info, warn, error, debug, trace};

#[tracing::instrument(skip(db))]
async fn process_user(user_id: u64, db: &Database) -> Result<User> {
    debug!(user_id, "Starting user processing");
    
    let user = db.get_user(user_id).await
        .context("Failed to fetch user")?;
        
    info!(user_id, user_name = %user.name, "User retrieved successfully");
    
    if user.is_inactive() {
        warn!(user_id, "Processing inactive user");
    }
    
    Ok(user)
}
```

### Environment Setup

```bash
# Enable debug logging
export RUST_LOG=debug

# Enable backtraces
export RUST_BACKTRACE=1

# For detailed backtraces
export RUST_BACKTRACE=full
```

### Testing Tools

```bash
# Run specific test
cargo test test_user_creation

# Run tests with output
cargo test -- --nocapture

# Run tests in single thread
cargo test -- --test-threads=1

# Run ignored tests
cargo test -- --ignored
```

## 📋 Pull Request Guidelines

### PR Description Template

```markdown
## Description
Brief description of the changes made.

## Type of Change
- [ ] Bug fix (non-breaking change which fixes an issue)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] Documentation update

## Testing
- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] Manual testing completed
- [ ] New tests added for new functionality

## Documentation
- [ ] Code is documented with rustdoc comments
- [ ] README updated (if applicable)
- [ ] API documentation updated (if applicable)

## Checklist
- [ ] Code follows project style guidelines
- [ ] Self-review completed
- [ ] No linting errors (`cargo clippy`)
- [ ] Code is properly formatted (`cargo fmt`)
- [ ] Tests pass (`cargo test`)

## Related Issues
Closes #issue_number
```

### Review Process

1. **Automated checks** must pass (CI/CD pipeline)
2. **At least one approval** from maintainer required
3. **All conversations resolved** before merge
4. **Up-to-date with main** branch

### Merge Strategy

- **Squash and merge** for feature branches
- **Rebase and merge** for simple fixes
- **Merge commit** for release branches

## 🚨 Security Guidelines

### Input Validation

```rust
use validator::{Validate, ValidationError};

#[derive(Deserialize, Validate)]
pub struct CreateUserRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: String,
    
    #[validate(email)]
    pub email: String,
    
    #[validate(custom = "validate_password")]
    pub password: String,
}

fn validate_password(password: &str) -> Result<(), ValidationError> {
    if password.len() < 8 {
        return Err(ValidationError::new("Password too short"));
    }
    Ok(())
}
```

### Secrets Management

- **Never commit secrets** to version control
- **Use environment variables** for configuration
- **Rotate secrets regularly**
- **Use secure random generation** for tokens

```rust
// ❌ Bad: hardcoded secret
const API_KEY: &str = "sk-1234567890abcdef";

// ✅ Good: environment variable
let api_key = std::env::var("API_KEY")
    .context("API_KEY environment variable not set")?;
```

## 📞 Getting Help

- **GitHub Issues**: For bug reports and feature requests
- **GitHub Discussions**: For questions and general discussion
- **Documentation**: Check existing docs first
- **Code Examples**: Look at existing modules for patterns

## 🎯 Contribution Areas

We welcome contributions in:

- **New modules**: Add functionality to the platform
- **Bug fixes**: Fix issues in existing code
- **Documentation**: Improve guides and examples
- **Testing**: Add test coverage and improve test quality
- **Performance**: Optimize critical paths
- **Developer experience**: Improve tooling and workflows

Thank you for contributing to HyperSpot Server! 🚀
