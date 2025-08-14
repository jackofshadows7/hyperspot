# Lock-Free Performance Patterns

This document describes the lock-free patterns implemented in the codebase to optimize hot-path performance and reduce contention.

## Overview

The codebase has been optimized to minimize lock contention and eliminate performance bottlenecks on hot paths through:

1. **DashMap for Hot Mutable Collections**: Replacing `Arc<RwLock<HashMap<..>>>` with `DashMap` for concurrent access
2. **ArcSwap for Read-Mostly Data**: Using `arc-swap` for atomically swapping read-mostly structures
3. **Cancellation Tokens over RwLock**: Modern async lifecycle management 
4. **Typed ClientHub**: Single-downcast client access pattern

## 1. DashMap for Hot Mutable Collections

### Before (High Contention)
```rust
use std::sync::{Arc, RwLock};
use std::collections::HashMap;

struct Registry {
    routes: Arc<RwLock<HashMap<String, RouteHandler>>>,
}

impl Registry {
    fn register(&self, key: String, handler: RouteHandler) {
        let mut routes = self.routes.write().unwrap(); // Blocks all readers
        routes.insert(key, handler);
    }
    
    fn get(&self, key: &str) -> Option<RouteHandler> {
        let routes = self.routes.read().unwrap(); // May wait for writers
        routes.get(key).cloned()
    }
}
```

### After (Lock-Free)
```rust
use dashmap::DashMap;

struct Registry {
    routes: DashMap<String, RouteHandler>,
}

impl Registry {
    fn register(&self, key: String, handler: RouteHandler) {
        self.routes.insert(key, handler); // No global lock
    }
    
    fn get(&self, key: &str) -> Option<RouteHandler> {
        self.routes.get(key).map(|v| v.clone()) // Concurrent access
    }
}
```

### Benefits
- **Zero reader blocking**: Multiple readers can access different keys simultaneously
- **Fine-grained locking**: Only specific keys are locked during updates
- **Better cache locality**: Less lock contention improves CPU cache usage

## 2. ArcSwap for Read-Mostly Data

### Before (Lock Contention)
```rust
use std::sync::{Arc, RwLock};

struct RouterCache {
    router: Arc<RwLock<axum::Router>>,
}

impl RouterCache {
    fn get_router(&self) -> axum::Router {
        self.router.read().unwrap().clone() // Lock acquisition overhead
    }
    
    fn update_router(&self, new_router: axum::Router) {
        *self.router.write().unwrap() = new_router; // Blocks all readers
    }
}
```

### After (Lock-Free)
```rust
use arc_swap::ArcSwap;
use std::sync::Arc;

struct RouterCache {
    router: ArcSwap<axum::Router>,
}

impl RouterCache {
    fn get_router(&self) -> Arc<axum::Router> {
        self.router.load_full() // Atomic load, no locks
    }
    
    fn update_router(&self, new_router: axum::Router) {
        self.router.store(Arc::new(new_router)); // Atomic swap
    }
}
```

### Benefits
- **Zero lock overhead**: Readers never wait or acquire locks
- **Atomic updates**: Router swaps are consistent and atomic
- **Memory efficient**: Old routers are automatically freed when unreferenced

## 3. Cancellation Tokens over RwLock

### Before (Lifecycle Management with Locks)
```rust
use std::sync::{Arc, RwLock};
use tokio::task::JoinHandle;

struct Server {
    handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    shutdown_tx: Arc<RwLock<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl Server {
    async fn stop(&self) {
        // Complex lock management
        let tx = self.shutdown_tx.write().unwrap().take();
        if let Some(tx) = tx {
            let _ = tx.send(());
        }
        
        let handle = self.handle.write().unwrap().take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }
}
```

### After (Modern Cancellation Pattern)
```rust
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;
use std::sync::atomic::{AtomicBool, Ordering};

struct Server {
    handle: OnceCell<tokio::task::JoinHandle<()>>,
    shutdown_token: OnceCell<CancellationToken>,
    is_started: AtomicBool,
}

impl Server {
    async fn stop(&self) {
        if let Some(token) = self.shutdown_token.get() {
            token.cancel(); // Signal shutdown
        }
        
        // Wait for server task completion via status polling
        while self.status.load(Ordering::SeqCst) != STOPPED {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
```

### Benefits
- **No lock contention**: Uses atomic operations and cancellation signals
- **Cleaner shutdown**: Structured concurrency with cancellation tokens
- **Race-free initialization**: OnceCell ensures single initialization

## 4. Typed ClientHub with Single Downcast

### Before (Repeated Downcasts)
```rust
use std::any::Any;
use std::collections::HashMap;

struct ClientHub {
    clients: HashMap<String, Arc<dyn Any>>,
}

impl ClientHub {
    fn get<T: 'static>(&self, key: &str) -> Option<Arc<T>> {
        let any_client = self.clients.get(key)?;
        any_client.downcast::<T>().ok() // Downcast on every call
    }
}

// Usage - downcasts repeatedly
let client1 = hub.get::<dyn MyService>("my_service")?; // Downcast 1
let client2 = hub.get::<dyn MyService>("my_service")?; // Downcast 2 (same type!)
```

### After (Single Downcast Pattern)
```rust
use dashmap::DashMap;
use tokio::sync::OnceCell;
use std::any::{Any, TypeId};

struct ClientHub {
    store: DashMap<(ClientKey, TypeId), Arc<OnceCell<Arc<dyn Any + Send + Sync>>>>,
}

impl ClientHub {
    pub async fn get_or_init<T>(&self, key: &ClientKey, init: impl FnOnce(&ClientKey) -> Pin<Box<dyn Future<Output = Arc<T>> + Send>>) -> Arc<T>
    where T: Send + Sync + 'static 
    {
        // Single downcast per type/key combination
        // Subsequent calls return the typed Arc<T> directly
    }
}

// Usage - initialize once, use typed handles
let key = ClientKey::global("my_service");
let client: Arc<dyn MyService> = hub.get_or_init(&key, create_client).await; // Single downcast
// client is now Arc<dyn MyService> - no further downcasts needed
```

### Benefits
- **Single downcast**: Type erasure and restoration happens exactly once
- **Typed handles**: Callers work with `Arc<T>` directly, no runtime type checks
- **Concurrent initialization**: Race-free lazy initialization under load

## Performance Impact

### Hot Path Optimizations

1. **Request Routing**: Router access via `ArcSwap` eliminates lock acquisition
2. **Route Registration**: `DashMap` allows concurrent registration/lookup  
3. **Client Access**: Single downcast with typed handles eliminates repeated type checks
4. **Duplicate Detection**: Concurrent duplicate checking via `DashMap`

### Memory Efficiency

1. **Reference Counting**: Smart pointers avoid unnecessary cloning
2. **Atomic Operations**: CPU-level atomics replace mutex overhead
3. **Cache Locality**: Reduced lock contention improves CPU cache utilization

### Concurrency Benefits

1. **Reader Scalability**: Multiple readers never block each other
2. **Fine-Grained Locking**: Operations lock only necessary data
3. **Lock-Free Reads**: Hot paths avoid lock acquisition entirely

## Migration Checklist

When implementing these patterns:

- [ ] Replace `Arc<RwLock<HashMap<K, V>>>` with `DashMap<K, V>`
- [ ] Use `ArcSwap<T>` for read-mostly data that rebuilds infrequently
- [ ] Avoid `parking_lot::RwLock` in async code; prefer `tokio::sync::RwLock` or atomics
- [ ] Use `CancellationToken` for graceful shutdown instead of channel-based signaling
- [ ] Move client lookup out of handlers into initialization; handlers use typed `Arc<T>`
- [ ] Ensure exactly one downcast per client type/key combination

## Testing Patterns

```rust
#[tokio::test]
async fn test_concurrent_initialization() {
    let hub = ClientHub::new();
    let key = ClientKey::global("test");
    
    static INIT_COUNT: AtomicUsize = AtomicUsize::new(0);
    
    // Race multiple initializations
    let (c1, c2, c3) = tokio::join!(
        hub.get_or_init(&key, |_| Box::pin(async { 
            INIT_COUNT.fetch_add(1, Ordering::SeqCst);
            Arc::new(TestClient) 
        })),
        hub.get_or_init(&key, |_| Box::pin(async { 
            INIT_COUNT.fetch_add(1, Ordering::SeqCst);
            Arc::new(TestClient) 
        })),
        hub.get_or_init(&key, |_| Box::pin(async { 
            INIT_COUNT.fetch_add(1, Ordering::SeqCst);
            Arc::new(TestClient) 
        }))
    );
    
    // Should initialize exactly once
    assert_eq!(INIT_COUNT.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&c1, &c2) && Arc::ptr_eq(&c2, &c3));
}
```

## Future Optimizations

1. **Lock-Free Data Structures**: Consider `crossbeam` for specialized collections
2. **Memory Ordering**: Fine-tune atomic ordering for specific use cases  
3. **Custom Allocators**: Reduce allocation overhead for high-frequency operations
4. **NUMA Awareness**: Consider CPU topology for large-scale deployments
