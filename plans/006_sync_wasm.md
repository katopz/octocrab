# Plan: Synchronization Primitives Abstraction for WASM Support

## Overview
Replace `tokio::sync` primitives with an abstraction layer that supports both native (tokio::sync or parking_lot) and WASM (WASM-compatible alternatives) platforms, enabling Cloudflare Workers compatibility while maintaining thread safety.

## Problem Statement
- Tokio runtime sync primitives are not compatible with WASM/Cloudflare Workers
- Octocrab may use `tokio::sync::{Mutex, RwLock, OnceLock, Semaphore}` in various modules
- Need to maintain both native and WASM builds with feature flags
- Workers is single-threaded, but code may assume multi-threaded environments
- Must preserve thread safety on native platform

## Scope

**Affected Modules:**
- `src/lib.rs` - Global static instance and caching
- `src/service/middleware/cache.rs` - In-memory cache (uses DashMap)
- `src/service/middleware/cache/mem.rs` - Cache implementation
- `src/auth.rs` - Token caching
- Any other modules using tokio::sync

**In Scope:**
- Synchronization primitives abstraction (Mutex, RwLock)
- Static singleton initialization (OnceLock)
- Atomic operations (if needed)
- Concurrent data structures (DashMap alternatives)
- Thread-safe initialization patterns

**Out of Scope:**
- tokio::spawn / task spawning (covered in async runtime plan)
- Tokio channels (not typically used in Octocrab)
- Complex concurrent data structures
- Distributed synchronization

## Current Usage Analysis

### Static Instance
Octocrab uses a static instance pattern:
```rust
// In lib.rs
static STATIC_INSTANCE: OnceLock<Octocrab> = OnceLock::const_new();
```

### In-Memory Cache
Uses DashMap for concurrent access:
```rust
// In service/middleware/cache/mem.rs
use dashmap::DashMap;

pub struct InMemoryCache {
    storage: Arc<DashMap<Uri, CacheEntry>>,
}
```

### Token Caching
May use sync primitives for thread-safe token caching:
```rust
struct CachedTokenInner {
    expiration: SystemTime,
    secret: SecretString,
}

// May use Arc<Mutex<>> or similar
```

## Implementation Plan

### Phase 1: Feature Flag and Dependencies

1. **Update `Cargo.toml`**
```toml
[dependencies]
# Sync primitives
parking_lot = { version = "0.12.1", optional = true }

# No additional WASM sync dependencies needed
# We'll use parking_lot on both platforms

# Conditional dependencies
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
# parking_lot is optional, we can use tokio::sync instead if preferred
# But parking_lot is generally better and works on WASM
parking_lot = "0.12.1"
```

2. **Update feature flags**
```toml
[features]
# Existing features
default = [
    "default-client",
    "follow-redirect",
    "retry",
    "rustls",
    "timeout",
    "tracing",
    "rustls-ring",
    "jwt-rust-crypto",
]

# WASM support
wasm = [
    "follow-redirect",
    "retry",
    "timeout",
    "tracing",
    "wasm-sync",
]

# Sync primitives
wasm-sync = ["parking_lot"]
```

### Phase 2: Create Sync Primitives Abstraction

1. **Create `src/internal/sync.rs`**
```rust
//! Synchronization primitives abstraction for cross-platform support

use std::sync::Once;

/// Mutex abstraction - uses parking_lot on both platforms
pub use parking_lot::Mutex;

/// RwLock abstraction - uses parking_lot on both platforms
pub use parking_lot::RwLock;

/// OnceLock abstraction
/// For native: parking_lot::OnceLock (if available) or std::sync::OnceLock
/// For WASM: parking_lot::OnceLock or custom implementation

#[cfg(not(target_arch = "wasm32"))]
pub use std::sync::OnceLock;

#[cfg(target_arch = "wasm32")]
pub use parking_lot::OnceLock;

/// Re-export common types
pub type Once<T> = OnceLock<T>;

/// Helper for one-time initialization
pub fn once_init<T, F>(init: F) -> T
where
    T: Send + Sync,
    F: FnOnce() -> T,
{
    // This is handled by OnceLock's get_or_init
}
```

### Phase 3: Create Concurrent Data Structure Abstraction

1. **Create `src/internal/concurrent.rs`**
```rust
//! Concurrent data structures for cross-platform support

use std::collections::HashMap;
use std::sync::Arc;
use crate::internal::sync::{RwLock, Mutex};

/// Concurrent map abstraction
/// For native: DashMap (better performance)
/// For WASM: RwLock<HashMap> (simpler, works in single-threaded context)

#[cfg(not(target_arch = "wasm32"))]
pub use dashmap::DashMap as ConcurrentMap;

#[cfg(target_arch = "wasm32")]
pub struct ConcurrentMap<K, V>
where
    K: Eq + std::hash::Hash + Send + Sync,
    V: Send + Sync,
{
    inner: Arc<RwLock<HashMap<K, V>>>,
}

#[cfg(target_arch = "wasm32")]
impl<K, V> ConcurrentMap<K, V>
where
    K: Eq + std::hash::Hash + Send + Sync,
    V: Send + Sync,
{
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn get<Q>(&self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: std::hash::Hash + Eq + ?Sized,
    {
        let map = self.inner.read();
        map.get(key).cloned()
    }

    pub fn insert(&self, key: K, value: V) {
        let mut map = self.inner.write();
        map.insert(key, value);
    }

    pub fn remove<Q>(&self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: std::hash::Hash + Eq + ?Sized,
    {
        let mut map = self.inner.write();
        map.remove(key)
    }

    pub fn iter(&self) -> Vec<(K, V)> {
        let map = self.inner.read();
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub fn clear(&self) {
        let mut map = self.inner.write();
        map.clear();
    }
}

#[cfg(target_arch = "wasm32")]
impl<K, V> Default for ConcurrentMap<K, V>
where
    K: Eq + std::hash::Hash + Send + Sync,
    V: Send + Sync,
{
    fn default() -> Self {
        Self::new()
    }
}
```

2. **Update `src/internal/mod.rs`**
```rust
pub mod sync;
pub mod concurrent;
```

### Phase 4: Refactor Static Instance

**File: `src/lib.rs`**

**Update static instance:**
```rust
use crate::internal::sync::OnceLock;

static STATIC_INSTANCE: OnceLock<Octocrab> = OnceLock::const_new();
```

No changes needed to usage, OnceLock interface is compatible.

### Phase 5: Refactor In-Memory Cache

**File: `src/service/middleware/cache.rs`**

**Update to use abstraction:**
```rust
use crate::internal::concurrent::ConcurrentMap;
use std::sync::Arc;

pub struct InMemoryCache {
    storage: Arc<ConcurrentMap<Uri, CacheEntry>>,
}

impl InMemoryCache {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(ConcurrentMap::new()),
        }
    }
}

impl CacheStorage for InMemoryCache {
    fn try_hit(&self, uri: &Uri) -> Option<CacheKey> {
        self.storage.get(uri).map(|entry| entry.key.clone())
    }

    fn load(&self, uri: &Uri) -> Option<CachedResponse> {
        self.storage.get(uri).map(|entry| CachedResponse {
            body: entry.body.clone(),
            headers: entry.headers.clone(),
        })
    }

    fn writer(&self, uri: &Uri, key: CacheKey, headers: HeaderMap) -> Box<dyn CacheWriter> {
        Box::new(InMemoryWriter {
            storage: Arc::clone(&self.storage),
            uri: uri.clone(),
            key,
            headers,
            buffer: Vec::new(),
        })
    }
}
```

**Update InMemoryWriter:**
```rust
struct InMemoryWriter {
    storage: Arc<ConcurrentMap<Uri, CacheEntry>>,
    uri: Uri,
    key: CacheKey,
    headers: HeaderMap,
    buffer: Vec<u8>,
}

impl CacheWriter for InMemoryWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.buffer.extend_from_slice(data);
        Ok(())
    }

    fn finish(self: Box<Self>) -> std::io::Result<()> {
        let entry = CacheEntry {
            key: self.key,
            headers: self.headers,
            body: self.buffer,
        };
        
        // Using the abstraction
        // Note: This may need to be adapted based on ConcurrentMap API
        // For now, assuming insert method exists
        drop(self.storage.insert(self.uri, entry));
        
        Ok(())
    }
}
```

### Phase 6: Update Token Caching (if needed)

**File: `src/lib.rs`**

**Ensure CachedToken is thread-safe:**
```rust
use crate::internal::sync::{Mutex, OnceLock};

// CachedToken already uses SecretString which is thread-safe
// May need Mutex if there's mutable state

pub struct CachedToken {
    #[cfg(not(target_arch = "wasm32"))]
    inner: Arc<Mutex<CachedTokenInner>>,
    #[cfg(target_arch = "wasm32")]
    inner: Arc<Mutex<CachedTokenInner>>,
}

impl CachedToken {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(CachedTokenInner {
                expiration: SystemTime::UNIX_EPOCH,
                secret: SecretString::new(String::new()),
            })),
        }
    }
}
```

### Phase 7: Review and Update Other Sync Usage

Search for and update any other sync primitive usage:

```rust
// Before:
use tokio::sync::{Mutex, RwLock, OnceLock};

// After:
use crate::internal::sync::{Mutex, RwLock, OnceLock};
```

### Phase 8: Update Cargo.toml Dependencies

Ensure parking_lot is properly configured:

```toml
[dependencies]
parking_lot = "0.12.
