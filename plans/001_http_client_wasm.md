# Plan: Hyper to WASM HTTP Client Abstraction

## Overview
Replace `hyper` with an abstraction layer that supports both native (hyper) and WASM (wasm-compatible HTTP client) platforms, enabling Cloudflare Workers compatibility for GitHub REST API interactions.

## Problem Statement
- `hyper` with `hyper-util` is not directly compatible with WASM/Cloudflare Workers
- Octocrab's HTTP client is built on `hyper_util::client::legacy::Client`
- The tower-based service architecture depends on HTTP client compatibility
- Need to maintain both native and WASM builds with feature flags
- Must preserve middleware functionality (retry, timeout, auth, etc.)

## Scope

**Affected Modules:**
- `src/lib.rs` - Main HTTP client construction and usage
- `src/service/` - Tower service layer
- `src/service/middleware/` - All middleware implementations
- `Cargo.toml` - Dependencies and feature flags

**In Scope:**
- REST API interactions (GET, POST, PATCH, DELETE, PUT)
- Tower middleware compatibility (retry, timeout, auth headers, base URI)
- Request/response handling (JSON, text)
- Error handling
- Header manipulation

**Out of Scope:**
- WebSocket connections (not applicable to GitHub API)
- Multipart file uploads with special WASM handling (deferred)
- Streaming responses (may need special handling for WASM)

## Current Architecture Analysis

### HTTP Client Construction
Octocrab currently uses `hyper_util::client::legacy::Client`:
```rust
// In lib.rs build method
let connector = hyper_rustls::HttpsConnectorBuilder::new()
    .with_native_roots()
    .https_or_http()
    .enable_http1()
    .enable_http2()
    .build();

let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
    .build(connector);
```

### Tower Service Layer
The client is wrapped in tower layers:
```rust
let service = ServiceBuilder::new()
    .layer(AuthHeaderLayer::new(...))
    .layer(BaseUriLayer::new(...))
    .layer(ExtraHeadersLayer::new(...))
    .layer(RetryLayer::new(...))  // if retry feature
    .layer(TimeoutLayer::new(...))  // if timeout feature
    .service(client);
```

## Implementation Plan

### Phase 1: Feature Flag and Dependencies

1. **Update `Cargo.toml`**
```toml
[dependencies]
hyper = { version = "1.1.0", optional = true }
hyper-util = { version = "0.1.3", features = ["http1"], optional = true }

# WASM dependencies (text-only, platform-specific)
[target.'cfg(target_arch = "wasm32")'.dependencies]
# Use WASM-compatible HTTP client
# Options: gloo-net, wasm-http, or custom fetch wrapper
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
js-sys = "0.3"
web-sys = { version = "0.3", features = ["Request", "Response", "Headers", "RequestInit"] }

# Conditional dependencies
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
hyper = "1.1.0"
hyper-util = { version = "0.1.3", features = ["http1"] }
```

2. **Update feature flags**
```toml
[features]
default = [
    "follow-redirect",
    "retry",
    "rustls",
    "timeout",
    "tracing",
    "default-client",
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

### Phase 2: Create HTTP Client Abstraction

1. **Create `src/internal/http_client.rs`**
```rust
//! HTTP client abstraction for cross-platform support

use http::{Request, Response};
use http_body::Body;
use std::future::Future;
use std::pin::Pin;

#[cfg(not(target_arch = "wasm32"))]
pub use hyper_util::client::legacy::Client as HyperClient;

#[cfg(not(target_arch = "wasm32"))]
pub use hyper::body::Incoming as HyperIncoming;

#[cfg(target_arch = "wasm32")]
pub use wasm_http::Client as HyperClient;

#[cfg(target_arch = "wasm32")]
pub type HyperIncoming = wasm_http::Incoming;

/// Generic HTTP client trait for tower compatibility
pub trait HttpClient<B>: Send + Sync + 'static
where
    B: Body + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    fn call(
        &self,
        req: Request<B>,
    ) -> Pin<Box<dyn Future<Output = Result<Response<HyperIncoming>, http::Error>> + Send>>;
}

#[cfg(not(target_arch = "wasm32"))]
impl<B> HttpClient<B> for HyperClient<hyper_rustls::HttpsConnector<hyper::client::HttpConnector>, B>
where
    B: Body + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    fn call(
        &self,
        req: Request<B>,
    ) -> Pin<Box<dyn Future<Output = Result<Response<HyperIncoming>, http::Error>> + Send>> {
        Box::pin(hyper_util::client::legacy::Client::request(self, req))
    }
}

#[cfg(target_arch = "wasm32")]
impl<B> HttpClient<B> for HyperClient
where
    B: Body + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    fn call(
        &self,
        req: Request<B>,
    ) -> Pin<Box<dyn Future<Output = Result<Response<HyperIncoming>, http::Error>> + Send>> {
        Box::pin(self.request(req))
    }
}
```

2. **Create WASM HTTP client wrapper**
```rust
// src/internal/wasm_http/mod.rs
// Minimal WASM HTTP client using web-sys Fetch API

pub struct Client;

impl Client {
    pub fn new() -> Self {
        Self
    }
}

pub type Incoming = IncomingBody;

pub struct IncomingBody {
    // Wrapper around web-sys Response body
}

impl Body for IncomingBody {
    type Data = bytes::Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        // Implementation using web-sys ReadableStream
        // ...
    }
}
```

3. **Update `src/internal/mod.rs`**
```rust
pub mod http_client;

#[cfg(target_arch = "wasm32")]
pub mod wasm_http;
```

### Phase 3: Refactor OctocrabBuilder HTTP Client

**File: `src/lib.rs`**

**Changes in build method:**

```rust
impl OctocrabBuilder<NoSvc, DefaultOctocrabBuilderConfig, NoAuth, NotLayerReady> {
    pub fn build(self) -> Result<Octocrab> {
        // ... existing code ...

        let client = self.build_client()?;

        // ... rest of build logic ...
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn build_client(&self) -> Result<crate::OctocrabService> {
        use hyper_rustls::HttpsConnectorBuilder;
        use hyper_util::client::legacy::Client;
        use hyper_util::rt::TokioExecutor;

        let connector = HttpsConnectorBuilder::new()
            .with_native_roots()
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build();

        let client = Client::builder(TokioExecutor::new())
            .build::<_, http_body_util::Full<bytes::Bytes>>(connector);

        Ok(client)
    }

    #[cfg(target_arch = "wasm32")]
    fn build_client(&self) -> Result<crate::OctocrabService> {
        use crate::internal::http_client::HyperClient;

        let client = HyperClient::new();

        Ok(client)
    }
}
```

### Phase 4: Update Service Layer Type

**File: `src/lib.rs`**

**Update type alias:**

```rust
// Before:
pub type OctocrabService = tower::util::BoxService<
    http::Request<http_body_util::Full<bytes::Bytes>>,
    http::Response<hyper::body::Incoming>,
    crate::error::Error,
>;

// After:
#[cfg(not(target_arch = "wasm32"))]
pub type OctocrabService = tower::util::BoxService<
    http::Request<http_body_util::Full<bytes::Bytes>>,
    http::Response<hyper::body::Incoming>,
    crate::error::Error,
>;

#[cfg(target_arch = "wasm32")]
pub type OctocrabService = tower::util::BoxService<
    http::Request<http_body_util::Full<bytes::Bytes>>,
    http::Response<crate::internal::wasm_http::Incoming>,
    crate::error::Error,
>;
```

### Phase 5: Update Request/Response Handling

**File: `src/lib.rs`**

**Update request body construction:**

```rust
// In execute method and related methods
#[cfg(not(target_arch = "wasm32"))]
fn create_request_body(body: Vec<u8>) -> http_body_util::Full<bytes::Bytes> {
    http_body_util::Full::new(bytes::Bytes::from(body))
}

#[cfg(target_arch = "wasm32")]
fn create_request_body(body: Vec<u8>) -> http_body_util::Full<bytes::Bytes> {
    http_body_util::Full::new(bytes::Bytes::from(body))
}
```

**Update response body handling:**

```rust
// In body_to_string method
pub async fn body_to_string<B>(response: &mut Response<B>) -> Result<String>
where
    B: Body + Unpin,
    B::Error: Into<crate::Error>,
{
    use http_body_util::BodyExt;

    let bytes = response
        .body_mut()
        .collect()
        .await
        .map_err(|e| crate::Error::Other(e.into()))?
        .to_bytes();

    String::from_utf8(bytes.to_vec())
        .map_err(|e| crate::Error::Other(e.into()))
}
```

### Phase 6: Update Middleware Compatibility

**File: `src/service/middleware/*.rs`**

**Ensure all middleware works with both body types:**

```rust
// In each middleware, use generic body types
use http_body::Body;

pub struct MyMiddleware<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for MyMiddleware<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    S::Error: Into<crate::Error>,
    ReqBody: Body,
    ResBody: Body,
{
    type Response = S::Response;
    type Error = crate::Error;
    type Future = /* ... */;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        // ... implementation ...
    }
}
```

### Phase 7: Update Error Handling

**File: `src/error.rs`**

**Ensure HTTP errors are compatible:**

```rust
#[cfg(not(target_arch = "wasm32"))]
impl From<hyper::Error> for Error {
    fn from(err: hyper::Error) -> Self {
        Error::Http(Box::new(err))
    }
}

#[cfg(target_arch = "wasm32")]
impl From<wasm_http::Error> for Error {
    fn from(err: wasm_http::Error) -> Self {
        Error::Http(Box::new(err))
    }
}
```

## Potential Issues and Solutions

### Issue 1: Body Type Compatibility
**Problem:** Different platforms use different body types

**Solution:**
- Use generic body types in middleware
- Use `http_body_util` for body operations
- Box body types when needed

### Issue 2: Tower Service Compatibility
**Problem:** WASM HTTP client may not implement tower::Service

**Solution:**
- Create wrapper implementing tower::Service for WASM client
- Ensure all methods use async traits
- Use BoxService to abstract differences

### Issue 3: Streaming Responses
**Problem:** WASM Fetch API may have different streaming semantics

**Solution:**
- Implement Body trait for WASM response
- Use ReadableStream API for streaming
- Provide buffered fallback if needed

### Issue 4: Executor Compatibility
**Problem:** `TokioExecutor` doesn't work in WASM

**Solution:**
- Use `wasm-bindgen-futures` executor for WASM
- Conditional compilation in client builder
- Keep tokio for native platform

### Issue 5: HTTP/2 Support
**Problem:** WASM Fetch API may not support HTTP/2

**Solution:**
- Use HTTP/1.1 for WASM (GitHub API supports both)
- Keep HTTP/2 for native platform
- Document difference if noticeable

### Issue 6: Connector Differences
**Problem:** TLS connector not needed in WASM (browser handles it)

**Solution:**
- Conditional compilation in client builder
- Skip TLS setup for WASM
- Document that Workers handle TLS automatically

## Testing Strategy

### 1. Unit Tests
```bash
# Test native platform
cargo test --package octocrab --lib

# Test WASM (requires wasm-pack)
wasm-pack test --node
```

### 2. Integration Tests
- Test all HTTP methods (GET, POST, PATCH, DELETE, PUT)
- Test authentication (personal token, OAuth, JWT)
- Test middleware (retry, timeout, auth headers, base URI)
- Test error handling
- Test request/response body handling

### 3. Cloudflare Workers Testing
```rust
// examples/wasm_github_api.rs
use octocrab::Octocrab;

async fn test_rest_api() -> Result<(), Box<dyn std::error::Error>> {
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN")?)
        .build()?;

    // Test getting a user
    let user = octocrab.users("octocat").get().await?;
    println!("User: {}", user.login);

    // Test listing repositories
    let repos = octocrab
        .repos("XAMPPRocky", "octocrab")
        .get()
        .await?;
    println!("Repo: {}", repos.name);

    Ok(())
}
```

### 4. Platform-Specific Tests
- Native: Test with actual hyper client
- WASM: Test with mock responses
- Both: Test API compatibility

## Implementation Order

1. ✅ Setup feature flags and dependencies
2. ✅ Create HTTP client abstraction layer
3. ✅ Create WASM HTTP client wrapper
4. ✅ Update OctocrabBuilder HTTP client construction
5. ✅ Update service layer type aliases
6. ✅ Update request/response handling
7. ✅ Update middleware compatibility
8. ✅ Update error handling
9. ⏸️ Run native tests
10. ⏸️ Create WASM test
11. ⏸️ Update documentation
12. ⏸️ Add Workers example

## Success Criteria

- ✅ Code compiles for native platform without breaking changes
- ✅ Code compiles for WASM target with `wasm` feature
- ✅ All existing tests pass on native platform
- ✅ Basic REST API operations work in Cloudflare Workers
- ✅ All middleware works on both platforms
- ✅ Request/response handling works correctly
- ✅ Error handling is consistent across platforms
- ✅ No performance regression on native platform

## Use Cases Enabled

With this implementation, users can:
- Make all GitHub REST API calls from Workers
- Use all authentication methods (personal token, OAuth, JWT)
- Benefit from retry and timeout middleware
- Use conditional requests (ETag, Last-Modified)
- Handle errors properly
- Send and receive JSON/text data

## Limitations

- HTTP/2 only on native platform (HTTP/1.1 on WASM)
- Some advanced hyper features may not work on WASM
- TLS handled by Workers platform, not configurable
- File uploads may have size limitations in Workers

## Related Plans

- `002_async_runtime_wasm.md` - Async runtime compatibility
- `003_jwt_wasm.md` - JWT authentication for WASM
- `004_cache_storage_wasm.md` - Cache storage abstraction
- `005_tls_wasm.md` - TLS abstraction
- `006_sync_wasm.md` - Synchronization primitives

## References

- [Hyper documentation](https://docs.rs/hyper)
- [Tower documentation](https://docs.rs/tower)
- [Cloudflare Workers Fetch API](https://developers.cloudflare.com/workers/runtime-apis/fetch/)
- [GitHub API documentation](https://docs.github.com/en/rest)
- [wasm-bindgen documentation](https://rustwasm.github.io/wasm-bindgen/)
- [web-sys documentation](https://rustwasm.github.io/wasm-bindgen/api/web_sys/)

## Notes

- Focus on API compatibility, not feature parity
- Maintain tower-based architecture
- Preserve middleware functionality
- Test thoroughly with actual GitHub API
- Document platform differences clearly