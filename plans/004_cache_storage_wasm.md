# Plan: Cache Storage Abstraction for WASM Support

## Overview
Create an abstraction layer for cache storage that supports both native (filesystem/memory) and WASM (memory-only) platforms, enabling Cloudflare Workers compatibility while maintaining caching functionality.

## Problem Statement
- Octocrab currently uses in-memory cache storage via `service/middleware/cache.rs`
- The cache storage trait may have platform-specific assumptions
- WASM/Cloudflare Workers have no file system access
- Need to maintain both native and WASM builds with feature flags
- Must preserve conditional request functionality (ETag, Last-Modified)

## Scope

**Affected Modules:**
- `src/service/middleware/cache.rs` - Cache middleware implementation
- `src/service/middleware/cache/mem.rs` - In-memory cache implementation
- `src/lib.rs` - Cache configuration in builder
- `Cargo.toml` - Dependencies and feature flags

**In Scope:**
- Cache storage trait abstraction
- In-memory cache implementation (works on both platforms)
- Cache key management (ETag, Last-Modified)
- Cache hit/miss logic
- Cache entry storage and retrieval
- Cache invalidation

**Out of Scope:**
- Filesystem-based caching (not supported in Workers)
- Persistent caching across Workers invocations
- Distributed caching (future enhancement)
- Advanced cache eviction policies

## Current Architecture Analysis

### Cache Storage Trait
Octocrab defines a `CacheStorage` trait in `src/service/middleware/cache.rs`:
```rust
pub trait CacheStorage: Send + Sync {
    fn try_hit(&self, uri: &Uri) -> Option<CacheKey>;
    fn load(&self, uri: &Uri) -> Option<CachedResponse>;
    fn writer(&self, uri: &Uri, key: CacheKey, headers: HeaderMap) -> Box<dyn CacheWriter>;
}
```

### In-Memory Cache Implementation
Current implementation uses `DashMap` for concurrent access:
```rust
pub struct InMemoryCache {
    storage: Arc<DashMap<Uri, CacheEntry>>,
}
```

### Cache Key Types
```rust
pub enum CacheKey {
    ETag(String),
    LastModified(String),
}
```

### Cached Response Structure
```rust
pub struct CachedResponse {
    pub body: Vec<u8>,
    pub headers: HeaderMap,
}
```

## Implementation Plan

### Phase 1: Feature Flag and Dependencies

1. **Update `Cargo.toml`**
```toml
[dependencies]
# Cache dependencies
dashmap = { version = "5.5", optional = true }

# WASM dependencies - use different concurrent map
[target.'cfg(target_arch = "wasm32")'.dependencies]
# Option 1: Use simple RwLock + HashMap
# Option 2: Use WASM-specific concurrent map if available

# Conditional dependencies
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
dashmap = "5.5"
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
    "wasm-cache",
]

# Cache features
cache = ["dashmap"]
wasm-cache = []  # Uses built-in Rust collections
```

### Phase 2: Create Cache Storage Abstraction

1. **Create `src/internal/cache.rs`**
```rust
//! Cache storage abstraction for cross-platform support

use http::{HeaderMap, Uri};
use std::sync::Arc;

/// Re-export cache types
pub use crate::service::middleware::cache::{CacheKey, CachedResponse, CacheWriter};

/// Cache storage trait for platform abstraction
pub trait CacheStorage: Send + Sync {
    /// Returns the stored cache key for given URI if it's available
    fn try_hit(&self, uri: &Uri) -> Option<CacheKey>;

    /// Returns the cached response for given URI
    fn load(&self, uri: &Uri) -> Option<CachedResponse>;

    /// Returns a writer for caching a response
    fn writer(&self, uri: &Uri, key: CacheKey, headers: HeaderMap) -> Box<dyn CacheWriter>;
}

/// In-memory cache implementation using DashMap (native)
#[cfg(not(target_arch = "wasm32"))]
pub use crate::internal::cache_impl::InMemoryCache;

/// In-memory cache implementation using RwLock + HashMap (WASM)
#[cfg(target_arch = "wasm32")]
pub use crate::internal::cache_impl::InMemoryCache;

/// Create a new in-memory cache instance
pub fn new_in_memory_cache() -> Arc<dyn CacheStorage> {
    Arc::new(InMemoryCache::new())
}
```

2. **Create `src/internal/cache_impl.rs`**
```rust
//! Platform-specific cache implementations

use http::{HeaderMap, Uri};
use std::sync::Arc;
use std::collections::HashMap;
use std::sync::RwLock;

use crate::internal::cache::{CacheKey, CachedResponse, CacheStorage, CacheWriter};
use crate::service::middleware::cache::InMemoryWriter;

/// Cache entry structure
#[derive(Clone)]
struct CacheEntry {
    key: CacheKey,
    headers: HeaderMap,
    body: Vec<u8>,
}

// Native implementation using DashMap
#[cfg(not(target_arch = "wasm32"))]
pub struct InMemoryCache {
    storage: Arc<dashmap::DashMap<Uri, CacheEntry>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl InMemoryCache {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(dashmap::DashMap::new()),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
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

// WASM implementation using RwLock + HashMap
#[cfg(target_arch = "wasm32")]
pub struct InMemoryCache {
    storage: Arc<RwLock<HashMap<Uri, CacheEntry>>>,
}

#[cfg(target_arch = "wasm32")]
impl InMemoryCache {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl CacheStorage for InMemoryCache {
    fn try_hit(&self, uri: &Uri) -> Option<CacheKey> {
        self.storage
            .read()
            .ok()
            .and_then(|storage| storage.get(uri).map(|entry| entry.key.clone()))
    }

    fn load(&self, uri: &Uri) -> Option<CachedResponse> {
        self.storage
            .read()
            .ok()
            .and_then(|storage| {
                storage.get(uri).map(|entry| CachedResponse {
                    body: entry.body.clone(),
                    headers: entry.headers.clone(),
                })
            })
    }

    fn writer(&self, uri: &Uri, key: CacheKey, headers: HeaderMap) -> Box<dyn CacheWriter> {
        Box::new(WasmInMemoryWriter {
            storage: Arc::clone(&self.storage),
            uri: uri.clone(),
            key,
            headers,
            buffer: Vec::new(),
        })
    }
}

#[cfg(target_arch = "wasm32")]
struct WasmInMemoryWriter {
    storage: Arc<RwLock<HashMap<Uri, CacheEntry>>>,
    uri: Uri,
    key: CacheKey,
    headers: HeaderMap,
    buffer: Vec<u8>,
}

#[cfg(target_arch = "wasm32")]
impl CacheWriter for WasmInMemoryWriter {
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
        
        if let Ok(mut storage) = self.storage.write() {
            storage.insert(self.uri, entry);
        }
        
        Ok(())
    }
}
```

3. **Update `src/internal/mod.rs`**
```rust
pub mod cache;
pub mod cache_impl;
```

### Phase 3: Refactor Existing Cache Middleware

**File: `src/service/middleware/cache.rs`**

**Update to use abstraction:**

```rust
use crate::internal::cache::{CacheStorage, CacheKey, CachedResponse};
use crate::internal::cache::new_in_memory_cache;

// Keep the existing CacheWriter trait
pub trait CacheWriter: Send {
    fn write(&mut self, data: &[u8]) -> std::io::Result<()>;
    fn finish(self: Box<Self>) -> std::io::Result<()>;
}

// Update CacheLayer to use the abstraction
pub struct CacheLayer {
    storage: Arc<dyn CacheStorage>,
}

impl CacheLayer {
    pub fn new(storage: Arc<dyn CacheStorage>) -> Self {
        Self { storage }
    }
    
    pub fn in_memory() -> Self {
        Self {
            storage: new_in_memory_cache(),
        }
    }
}

impl<S> Layer<S> for CacheLayer {
    type Service = Cache<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Cache {
            inner,
            storage: Arc::clone(&self.storage),
        }
    }
}
```

### Phase 4: Update OctocrabBuilder

**File: `src/lib.rs`**

**Update cache configuration:**

```rust
impl OctocrabBuilder<NoSvc, DefaultOctocrabBuilderConfig, NoAuth, NotLayerReady> {
    pub fn cache(mut self, cache: bool) -> Self {
        if cache {
            self.config.cache_storage = Some(crate::internal::cache::new_in_memory_cache());
        } else {
            self.config.cache_storage = None;
        }
        self
    }
}
```

### Phase 5: Update Cache Writer Implementation

**File: `src/service/middleware/cache.rs`**

**Ensure InMemoryWriter works with abstraction:**

```rust
pub struct InMemoryWriter {
    #[cfg(not(target_arch = "wasm32"))]
    storage: Arc<dashmap::DashMap<Uri, CacheEntry>>,
    #[cfg(target_arch = "wasm32")]
    storage: Arc<RwLock<HashMap<Uri, CacheEntry>>>,
    uri: Uri,
    key: CacheKey,
    headers: HeaderMap,
    buffer: Vec<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
impl InMemoryWriter {
    pub fn new(
        storage: Arc<dashmap::DashMap<Uri, CacheEntry>>,
        uri: Uri,
        key: CacheKey,
        headers: HeaderMap,
    ) -> Self {
        Self {
            storage,
            uri,
            key,
            headers,
            buffer: Vec::new(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
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
        
        self.storage.insert(self.uri, entry);
        Ok(())
    }
}
```

### Phase 6: Conditional Compilation for Cache Module

**File: `src/service/middleware/mod.rs`**

```rust
pub mod auth_header;
pub mod base_uri;
#[cfg(feature = "cache")]
#[cfg_attr(docsrs, doc(cfg(feature = "cache")))]
pub mod cache;
pub mod extra_headers;
#[cfg(feature = "retry")]
#[cfg_attr(docsrs, doc(cfg(feature = "retry")))]
pub mod retry;
```

### Phase 7: Update DefaultOctocrabBuilderConfig

**File: `src/lib.rs`**

```rust
pub struct DefaultOctocrabBuilderConfig {
    auth: Option<Auth>,
    previews: Vec<String>,
    extra_headers: HeaderMap,
    connect_timeout: Option<Duration>,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
    base_uri: Option<Uri>,
    upload_uri: Option<Uri>,
    retry_config: Option<RetryConfig>,
    cache_storage: Option<Arc<dyn crate::internal::cache::CacheStorage>>,
}
```

## Potential Issues and Solutions

### Issue 1: Concurrent Access in WASM
**Problem:** DashMap doesn't work in WASM, need alternative

**Solution:**
- Use `RwLock<HashMap<>>` for WASM (good enough for Workers single-threaded)
- Performance may be slightly lower but acceptable
- Workers run in single thread, so contention is minimal

### Issue 2: Cache Size Limits
**Problem:** In-memory cache can grow unbounded

**Solution:**
- Add cache size limits (max entries, max bytes)
- Implement LRU eviction policy
- Document cache behavior
- Provide configuration options

### Issue 3: Cache Persistence Across Workers Invocations
**Problem:** Workers memory is ephemeral, cache doesn't persist

**Solution:**
- Document this limitation clearly
- Consider using Workers KV for persistent caching (future)
- For now, focus on in-memory caching within single invocation
- Most GitHub API calls benefit from ETag even without persistence

### Issue 4: Memory Usage in Workers
**Problem:** Workers have memory limits (128MB default)

**Solution:**
- Implement cache size limits
- Provide configuration to disable cache if needed
- Monitor memory usage
- Document best practices

### Issue 5: Cache Key Collisions
**Problem:** Different URIs might have same cache key

**Solution:**
- Use full URI as cache key (current implementation)
- Ensure proper normalization
- Test with various URI formats

### Issue 6: Thread Safety in WASM
**Problem:** WASM is single-threaded, but code uses sync primitives

**Solution:**
- Use `send_wrapper` or similar if needed
- RwLock should work fine in single-threaded context
- Test thoroughly in Workers environment

## Testing Strategy

### 1. Unit Tests
```bash
# Test native platform
cargo test --package octocrab --lib cache

# Test WASM
wasm-pack test --node
```

### 2. Cache Functionality Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::cache::{CacheStorage, CacheKey, new_in_memory_cache};
    use http::{HeaderMap, Uri};

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_cache_native() {
        let cache = new_in_memory_cache();
        let uri = Uri::from_static("https://api.github.com/users/octocat");
        
        // Test miss
        assert!(cache.try_hit(&uri).is_none());
        assert!(cache.load(&uri).is_none());
        
        // Test write
        let key = CacheKey::ETag("\"test-etag\"".to_string());
        let headers = HeaderMap::new();
        let mut writer = cache.writer(&uri, key, headers);
        writer.write(b"test data").unwrap();
        writer.finish().unwrap();
        
        // Test hit
        assert!(cache.try_hit(&uri).is_some());
        let response = cache.load(&uri).unwrap();
        assert_eq!(response.body, b"test data");
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    #[cfg(target_arch = "wasm32")]
    fn test_cache_wasm() {
        let cache = new_in_memory_cache();
        let uri = Uri::from_static("https://api.github.com/users/octocat");
        
        // Same tests as native
        assert!(cache.try_hit(&uri).is_none());
        
        let key = CacheKey::ETag("\"test-etag\"".to_string());
        let headers = HeaderMap::new();
        let mut writer = cache.writer(&uri, key, headers);
        writer.write(b"test data").unwrap();
        writer.finish().unwrap();
        
        assert!(cache.try_hit(&uri).is_some());
    }
}
```

### 3. Integration Tests
```rust
#[tokio::test]
#[cfg(not(target_arch = "wasm32"))]
async fn test_cache_with_real_requests() {
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN").unwrap())
        .cache(true)
        .build()
        .unwrap();

    // First request
    let user1 = octocrab.users("octocat").get().await.unwrap();
    
    // Second request (should use cache if ETag/Last-Modified)
    let user2 = octocrab.users("octocat").get().await.unwrap();
    
    assert_eq!(user1.id, user2.id);
}
```

### 4. Cloudflare Workers Testing
```rust
// examples/wasm_cache.rs
use octocrab::Octocrab;

pub async fn test_cache_in_workers() -> Result<(), Box<dyn std::error::Error>> {
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN")?)
        .cache(true)
        .build()?;
    
    // Multiple requests to test caching
    for _ in 0..3 {
        let user = octocrab.users("octocat").get().await?;
        println!("User: {} (login: {})", user.id, user.login);
    }
    
    Ok(())
}
```

### 5. Concurrency Tests
```rust
#[tokio::test]
#[cfg(not(target_arch = "wasm32"))]
async fn test_cache_concurrent_access() {
    let cache = new_in_memory_cache();
    let handles: Vec<_> = (0..100)
        .map(|i| {
            let cache = Arc::clone(&cache);
            tokio::spawn(async move {
                let uri = Uri::from_static(&format!("https://api.github.com/test/{}", i));
                let key = CacheKey::ETag(format!("\"etag-{}\"", i));
                let headers = HeaderMap::new();
                let mut writer = cache.writer(&uri, key, headers);
                writer.write(format!("data-{}", i).as_bytes()).unwrap();
                writer.finish().unwrap();
            })
        })
        .collect();
    
    for handle in handles {
        handle.await.unwrap();
    }
    
    // Verify all entries
    for i in 0..100 {
        let uri = Uri::from_static(&format!("https://api.github.com/test/{}", i));
        assert!(cache.try_hit(&uri).is_some());
        let response = cache.load(&uri).unwrap();
        assert_eq!(response.body, format!("data-{}", i).as_bytes());
    }
}
```

## Implementation Order

1. ✅ Setup feature flags and dependencies
2. ✅ Create cache storage abstraction layer
3. ✅ Create platform-specific cache implementations
4. ✅ Refactor existing cache middleware
5. ✅ Update OctocrabBuilder cache configuration
6. ✅ Update cache writer implementation
7. ✅ Update conditional compilation
8. ✅ Run native tests
9. ✅ Create WASM tests
10. ✅ Update documentation
11. ✅ Add Workers example
12. ⏸️ Consider adding cache size limits
13. ⏸️ Consider adding LRU eviction

## Success Criteria

- ✅ Code compiles for native platform without breaking changes
- ✅ Code compiles for WASM target with `wasm` feature
- ✅ All existing tests pass on native platform
- ✅ Cache functionality works on WASM platform
- ✅ Cache hit/miss logic works correctly on both platforms
- ✅ Concurrent access works safely on both platforms
- ✅ Conditional requests (ETag, Last-Modified) work in Workers
- ✅ No memory leaks in cache implementations
- ✅ No performance regression on native platform
- ✅ Cache can be disabled when needed

## Use Cases Enabled

With this implementation, users can:
- Use cache in Cloudflare Workers
- Reduce GitHub API calls via conditional requests
- Cache responses within a Workers invocation
- Improve performance for repeated requests
- Respect GitHub rate limits more effectively
- Disable cache if needed (memory constraints)

## Limitations

- Cache is in-memory only (no persistence across Workers invocations)
- Workers have memory limits (consider cache size)
- No LRU eviction in initial implementation
- No distributed caching (all requests independent)
- Cache invalidation only on conditional request failure

## Future Enhancements

- Integrate with Workers KV for persistent caching
- Implement LRU eviction policy
- Add cache statistics and metrics
- Support cache size limits
- Add cache warming strategies
- Support custom cache backends

## Related Plans

- `001_http_client_wasm.md` - HTTP client abstraction
- `002_async_runtime_wasm.md` - Async runtime compatibility
- `003_jwt_wasm.md` - JWT authentication for WASM

## References

- [GitHub API Conditional Requests](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api?apiVersion=2022-11-28#use-conditional-requests-if-appropriate)
- [HTTP Caching](https://developer.mozilla.org/en-US/docs/Web/HTTP/Caching)
- [DashMap documentation](https://docs.rs/dashmap)
- [Cloudflare Workers KV](https://developers.cloudflare.com/workers/wrangler/configuration/#kv-namespaces)
- [Cache Storage API](https://developer.mozilla.org/en-US/docs/Web/API/CacheStorage)

## Notes

- Workers memory is ephemeral, cache won't persist across invocations
- Conditional requests (ETag/Last-Modified) still provide value even without persistence
- In-memory cache is appropriate for single Workers invocation scenarios
- Consider using Workers KV for persistent caching if needed
- Test cache behavior with actual GitHub API to ensure ETag/Last-Modified handling
- Monitor memory usage in Workers, disable cache if needed

## Example Usage

### Native Platform
```rust
use octocrab::Octocrab;

#[tokio::main]
async fn main() -> octocrab::Result<()> {
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN")?)
        .cache(true)  // Enable cache
        .build()?;
    
    // First request (not cached)
    let user1 = octocrab.users("octocat").get().await?;
    
    // Second request (uses cache if ETag matches)
    let user2 = octocrab.users("octocat").get().await?;
    
    Ok(())
}
```

### Cloudflare Workers
```rust
use octocrab::Octocrab;

pub async fn handle_request() -> Result<(), Box<dyn std::error::Error>> {
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN")?)
        .cache(true)  // Enable cache (works in Workers)
        .build()?;
    
    // Multiple requests benefit from cache
    let user = octocrab.users("octocat").get().await?;
    println!("User: {}", user.login);
    
    Ok(())
}
```

## Performance Considerations

- In-memory cache reduces GitHub API calls
- Cache hit latency is negligible (memory lookup)
- Cache miss still makes HTTP request (with conditional headers)
- DashMap provides high concurrency on native platform
- RwLock+HashMap is adequate for Workers (single-threaded)
- Monitor cache hit ratio to optimize usage

## Best Practices

1. **Enable caching** for most use cases to reduce API calls
2. **Monitor memory usage** in Workers, disable if needed
3. **Use conditional requests** (ETag, Last-Modified) effectively
4. **Clear cache periodically** if needed (restart Workers)
5. **Test cache behavior** with actual GitHub API
6. **Document cache usage** for debugging

## Troubleshooting

### Issue: Cache not working
**Solution**: Verify cache is enabled in builder, check logs for cache hits/misses

### Issue: Memory high in Workers
**Solution**: Disable cache or consider using Workers KV for persistent caching

### Issue: Cache hit but stale data
**Solution**: Verify ETag/Last-Modified headers are being used correctly

### Issue: Concurrent access issues
**Solution**: Test with concurrent requests, check for race conditions

## Contributing

When contributing to cache abstraction:
1. Test on both native and WASM platforms
2. Ensure thread safety
3. Add tests for new functionality
4. Document cache behavior
5. Consider performance implications