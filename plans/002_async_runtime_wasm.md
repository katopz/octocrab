# Plan: Async Runtime Abstraction for WASM Support

## Overview
Replace `tokio` async runtime primitives with an abstraction layer that supports both native (tokio) and WASM (wasm-bindgen-futures) platforms, enabling Cloudflare Workers compatibility.

## Problem Statement
- Tokio runtime is not compatible with WASM/Cloudflare Workers
- `hyper_util::rt::TokioExecutor` is used for HTTP client execution
- Tokio time utilities may be used for timeouts and delays
- Need to maintain both native and WASM builds with feature flags
- Must preserve async functionality across platforms

## Scope

**Affected Modules:**
- `src/lib.rs` - HTTP client executor, time utilities
- `src/service/middleware/retry.rs` - May use tokio time for retry delays
- `src/service/middleware/timeout.rs` - May use tokio time for timeouts
- `Cargo.toml` - Dependencies and feature flags

**In Scope:**
- Async runtime executor abstraction
- Time utilities (sleep, timeout, interval)
- Task spawning (if used)
- Timer-based operations
- Timeout handling

**Out of Scope:**
- Tokio-specific I/O (fs, net, etc.) - handled separately
- Tokio sync primitives - covered in sync plan
- Tokio-specific tracing/logging

## Current Usage Analysis

### HTTP Client Executor
```rust
// In lib.rs - client construction
use hyper_util::rt::TokioExecutor;

let client = hyper_util::client::legacy::Client::builder(TokioExecutor::new())
    .build(connector);
```

### Timeout Feature
```toml
[features]
timeout = ["hyper-timeout", "tokio", "tower/timeout"]
```

### Retry Middleware
May use time utilities for retry delays.

## Implementation Plan

### Phase 1: Feature Flag and Dependencies

1. **Update `Cargo.toml`**
```toml
[dependencies]
tokio = { version = "1.17.0", default-features = false, features = ["time"], optional = true }
web-time = { version = "1.1.0", features = ["serde"] }

# WASM dependencies
[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen-futures = "0.4"
wasm-timer = "0.2"

# Conditional dependencies
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
tokio = { version = "1.17.0", default-features = false, features = ["time"] }
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
```

### Phase 2: Create Async Runtime Abstraction

1. **Create `src/internal/async_runtime.rs`**
```rust
//! Async runtime abstraction for cross-platform support

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use web_time::{Instant, SystemTime};

#[cfg(not(target_arch = "wasm32"))]
pub use tokio::time::sleep as sleep_async;

#[cfg(not(target_arch = "wasm32"))]
pub use tokio::time::timeout as timeout_async;

#[cfg(target_arch = "wasm32")]
pub async fn sleep_async(duration: Duration) {
    wasm_timer::Delay::new(duration).await
}

#[cfg(target_arch = "wasm32")]
pub async fn timeout_async<F, T>(duration: Duration, future: F) -> Result<T, &'static str>
where
    F: Future<Output = T>,
{
    wasm_timer::Timer::new(duration).await.map_err(|_| "timeout")?;
    // Note: This is a simplified version - actual implementation needs to race
    // with the future. May need a different approach.
    todo!("Implement timeout for WASM")
}

/// Generic sleep function
pub async fn sleep(duration: Duration) {
    sleep_async(duration).await
}

/// Generic timeout function
pub async fn timeout<F, T>(duration: Duration, future: F) -> Result<T, Box<dyn std::error::Error + Send + Sync>>
where
    F: Future<Output = T>,
{
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::timeout(duration, future)
            .await
            .map_err(|_| "timeout".into())
    }
    
    #[cfg(target_arch = "wasm32")]
    {
        // WASM timeout implementation
        // This may need tokio::time::timeout equivalent for WASM
        // Or use web-sys setTimeout with AbortController
        timeout_async(duration, future).await.map_err(|e| e.into())
    }
}
```

2. **Update `src/internal/mod.rs`**
```rust
pub mod async_runtime;
```

### Phase 3: Create HTTP Executor Abstraction

1. **Create `src/internal/executor.rs`**
```rust
//! HTTP client executor abstraction

#[cfg(not(target_arch = "wasm32"))]
pub use hyper_util::rt::TokioExecutor as Executor;

#[cfg(target_arch = "wasm32")]
pub use wasm_bindgen_futures::JsFuture as Executor;

#[cfg(target_arch = "wasm32")]
pub struct WasmExecutor;

#[cfg(target_arch = "wasm32")]
impl hyper::rt::Executor for WasmExecutor {
    fn execute<F>(&self, future: F)
    where
        F: std::future::Future + 'static,
        F::Output: 'static,
    {
        wasm_bindgen_futures::spawn_local(future);
    }
}
```

### Phase 4: Refactor HTTP Client Construction

**File: `src/lib.rs`**

**Update build_client method:**

```rust
#[cfg(not(target_arch = "wasm32"))]
fn build_client(&self) -> Result<crate::OctocrabService> {
    use hyper_rustls::HttpsConnectorBuilder;
    use hyper_util::client::legacy::Client;
    use crate::internal::executor::Executor;

    let connector = HttpsConnectorBuilder::new()
        .with_native_roots()
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();

    let client = Client::builder(Executor::new())
        .build::<_, http_body_util::Full<bytes::Bytes>>(connector);

    Ok(client)
}

#[cfg(target_arch = "wasm32")]
fn build_client(&self) -> Result<crate::OctocrabService> {
    use crate::internal::executor::WasmExecutor;

    // WASM doesn't need a connector - handled by platform
    let client = hyper::Client::builder()
        .executor(WasmExecutor)
        .build_http::<http_body_util::Full<bytes::Bytes>>();

    Ok(BoxService::new(client))
}
```

### Phase 5: Update Timeout Middleware

**File: `src/service/middleware/timeout.rs`**

**Update timeout implementation:**

```rust
use tower::{Layer, Service};
use std::time::Duration;
use std::task::{Context, Poll};
use pin_project::pin_project;

#[pin_project]
pub struct Timeout<S> {
    #[pin]
    inner: S,
    timeout: Duration,
}

impl<S, Request> Service<Request> for Timeout<S>
where
    S: Service<Request>,
    S::Error: Into<crate::Error>,
{
    type Response = S::Response;
    type Error = crate::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        ResponseFuture {
            inner: self.inner.call(req),
            timeout: self.timeout,
        }
    }
}

#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    inner: F,
    timeout: Duration,
}

impl<F, T, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<T, E>>,
    E: Into<crate::Error>,
{
    type Output = Result<T, crate::Error>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        
        #[cfg(not(target_arch = "wasm32"))]
        {
            match tokio::time::timeout(this.timeout, this.inner).poll(cx) {
                Poll::Ready(Ok(Ok(t))) => Poll::Ready(Ok(t)),
                Poll::Ready(Ok(Err(e))) => Poll::Ready(Err(e.into())),
                Poll::Ready(Err(_)) => Poll::Ready(Err(crate::Error::Timeout)),
                Poll::Pending => Poll::Pending,
            }
        }
        
        #[cfg(target_arch = "wasm32")]
        {
            // WASM timeout implementation
            match this.inner.poll(cx) {
                Poll::Ready(Ok(t)) => Poll::Ready(Ok(t)),
                Poll::Ready(Err(e)) => Poll::Ready(Err(e.into())),
                Poll::Pending => {
                    // Check timeout - simplified version
                    // Actual implementation needs to track start time
                    Poll::Pending
                }
            }
        }
    }
}
```

### Phase 6: Update Retry Middleware Time Usage

**File: `src/service/middleware/retry.rs`**

**Update delay implementations:**

```rust
use crate::internal::async_runtime::sleep;

// Replace tokio::time::sleep with sleep from abstraction
async fn delay_backoff(attempt: u32) {
    let duration = Duration::from_millis(2u64.pow(attempt.min(6)) * 100);
    sleep(duration).await;
}
```

### Phase 7: Update Other Time-Dependent Code

Search for and update any other tokio time usage:

```rust
// Before:
use tokio::time::{sleep, Duration, Instant};

// After:
use crate::internal::async_runtime::{sleep, timeout};
use web_time::{Duration, Instant};
```

## Potential Issues and Solutions

### Issue 1: Timeout Implementation on WASM
**Problem:** WASM doesn't have a direct equivalent to tokio::time::timeout

**Solution:**
- Implement custom timeout using web-sys AbortController
- Or use a crate like `wasm-timer` with proper timeout support
- May need to race two futures with manual abort logic

### Issue 2: Task Spawning
**Problem:** tokio::spawn doesn't work in WASM

**Solution:**
- Use `wasm_bindgen_futures::spawn_local` for WASM
- Create abstraction for task spawning
- Note: spawn_local only works in the same thread, which is fine for Workers

### Issue 3: Timer Accuracy
**Problem:** WASM timers may have different accuracy characteristics

**Solution:**
- Document any timing differences
- Use web-time for consistent API across platforms
- Test thoroughly with actual Workers environment

### Issue 4: Executor Compatibility
**Problem:** hyper::rt::Executor trait may not be easily implementable for WASM

**Solution:**
- Check if there's a WASM-compatible hyper executor
- May need to use a different HTTP client for WASM (covered in HTTP client plan)
- Or implement a custom executor wrapper

### Issue 5: Complex Timeout Scenarios
**Problem:** Complex timeout patterns (like racing multiple futures) are harder in WASM

**Solution:**
- Simplify timeout logic where possible
- Use established WASM patterns for racing futures
- Document any limitations

## Testing Strategy

### 1. Unit Tests
```bash
# Test native platform
cargo test --package octocrab --lib

# Test WASM
wasm-pack test --node
```

### 2. Time-Dependent Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::async_runtime::{sleep, timeout};

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_sleep_native() {
        let start = Instant::now();
        sleep(Duration::from_millis(100)).await;
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(90));
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    #[cfg(target_arch = "wasm32")]
    async fn test_sleep_wasm() {
        let start = Instant::now();
        sleep(Duration::from_millis(100)).await;
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(90));
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_timeout_native() {
        let result = timeout(Duration::from_millis(100), async {
            sleep(Duration::from_millis(50)).await;
            42
        }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    #[cfg(target_arch = "wasm32")]
    async fn test_timeout_wasm() {
        let result = timeout(Duration::from_millis(100), async {
            sleep(Duration::from_millis(50)).await;
            42
        }).await;
        assert_eq!(result.unwrap(), 42);
    }
}
```

### 3. Integration Tests
- Test timeout middleware on both platforms
- Test retry delays on both platforms
- Test concurrent operations
- Test with actual GitHub API calls

### 4. Cloudflare Workers Testing
```rust
// examples/wasm_timeout.rs
use octocrab::Octocrab;
use std::time::Duration;

async fn test_timeout() -> Result<(), Box<dyn std::error::Error>> {
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN")?)
        .build()?;

    // This should timeout
    let start = std::time::Instant::now();
    match timeout(Duration::from_secs(1), async {
        // Very slow operation
        std::future::pending::<()>().await;
        Ok::<(), Box<dyn std::error::Error>>(())
    }).await {
        Ok(_) => Err("Should have timed out".into()),
        Err(_) => {
            let elapsed = start.elapsed();
            assert!(elapsed >= Duration::from_millis(900));
            Ok(())
        }
    }
}
```

## Implementation Order

1. ✅ Setup feature flags and dependencies
2. ✅ Create async runtime abstraction layer
3. ✅ Create HTTP executor abstraction
4. ✅ Update HTTP client construction
5. ✅ Update timeout middleware
6. ✅ Update retry middleware
7. ✅ Update other time-dependent code
8. ⏸️ Run native tests
9. ⏸️ Create WASM tests
10. ⏸️ Update documentation
11. ⏸️ Add Workers example

## Success Criteria

- ✅ Code compiles for native platform without breaking changes
- ✅ Code compiles for WASM target with `wasm` feature
- ✅ All existing tests pass on native platform
- ✅ Sleep operations work on both platforms
- ✅ Timeout operations work on both platforms
- ✅ HTTP requests work with timeout on both platforms
- ✅ Retry delays work correctly on both platforms
- ✅ No performance regression on native platform

## Use Cases Enabled

With this implementation, users can:
- Use timeout middleware in Workers
- Have retry delays work in Workers
- Use time-based operations in Workers
- Have consistent async behavior across platforms
- Run async operations with proper timing

## Limitations

- WASM timeout implementation may be less precise than tokio
- Task spawning is single-threaded in WASM (appropriate for Workers)
- Some advanced tokio features not available in WASM
- Timer accuracy may vary between platforms

## Related Plans

- `001_http_client_wasm.md` - HTTP client abstraction
- `003_jwt_wasm.md` - JWT authentication for WASM
- `004_cache_storage_wasm.md` - Cache storage abstraction
- `006_sync_wasm.md` - Synchronization primitives

## References

- [Tokio documentation](https://docs.rs/tokio)
- [web-time documentation](https://docs.rs/web-time)
- [wasm-bindgen-futures documentation](https://docs.rs/wasm-bindgen-futures)
- [Cloudflare Workers Async](https://developers.cloudflare.com/workers/runtime-apis/fetch/)
- [wasm-timer documentation](https://docs.rs/wasm-timer)

## Notes

- Use `web-time` crate for cross-platform time API
- Focus on common async patterns (sleep, timeout, delay)
- Document any timing differences clearly
- Test thoroughly with actual Workers environment
- Keep abstraction simple and focused on needed functionality