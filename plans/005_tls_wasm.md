# Plan: TLS Abstraction for WASM Support

## Overview
Create an abstraction layer for TLS/HTTPS connections that supports both native (hyper-rustls) and WASM (platform-handled) platforms, enabling Cloudflare Workers compatibility while maintaining advanced TLS configuration options for native platforms.

## Problem Statement
- Octocrab currently uses `hyper-rustls::HttpsConnectorBuilder` for TLS on native platforms
- WASM/Cloudflare Workers handle TLS automatically (no explicit TLS configuration needed)
- Need to abstract connector creation for both native and WASM platforms
- Must preserve advanced TLS features (native roots, custom certificates, HTTP/2) on native platform
- Multiple TLS backend options (rustls-ring, rustls-aws-lc-rs, rustls-webpki-tokio) need to be maintained

## Scope

**Affected Modules:**
- `src/lib.rs` - HTTP client construction and connector configuration
- `Cargo.toml` - TLS dependencies and feature flags
- Builder methods related to TLS/connection configuration

**In Scope:**
- HTTPS connector abstraction
- TLS configuration for native platform
- No-op TLS for WASM platform
- Connection pool configuration
- HTTP/1 and HTTP/2 support
- Certificate validation options

**Out of Scope:**
- Custom certificate loading (deferred - may need special handling in WASM)
- Client certificates (deferred - not typically needed for GitHub API)
- Advanced TLS session resumption
- Mutual TLS (mTLS) - not needed for GitHub API

## Current Architecture Analysis

### Native TLS Configuration
Octocrab currently builds HTTPS connector using hyper-rustls:
```rust
// In lib.rs - client construction
let connector = hyper_rustls::HttpsConnectorBuilder::new()
    .with_native_roots()
    .https_or_http()
    .enable_http1()
    .enable_http2()
    .build();

let client = hyper_util::client::legacy::Client::builder(TokioExecutor::new())
    .build::<_, http_body_util::Full<bytes::Bytes>>(connector);
```

### TLS Backend Features
Multiple TLS backend options via feature flags:
```toml
[features]
rustls = ["hyper-rustls"]
rustls-ring = ["hyper-rustls/ring"]
rustls-aws-lc-rs = ["hyper-rustls/aws-lc-rs"]
rustls-webpki-tokio = ["hyper-rustls/webpki-tokio"]
```

## Implementation Plan

### Phase 1: Feature Flag and Dependencies

1. **Update `Cargo.toml`**
```toml
[dependencies]
hyper-rustls = { version = "0.27.0", optional = true, default-features = false, features = [
    "http1",
    "logging",
    "native-tokio",
    "tls12",
] }

# No additional WASM dependencies needed
# TLS is handled by the platform

# Conditional dependencies
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
hyper-rustls = { version = "0.27.0", default-features = false, features = [
    "http1",
    "logging",
    "native-tokio",
    "tls12",
] }
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
]

# Native TLS backends
rustls = ["hyper-rustls"]
rustls-ring = ["hyper-rustls/ring"]
rustls-aws-lc-rs = ["hyper-rustls/aws-lc-rs"]
rustls-webpki-tokio = ["hyper-rustls/webpki-tokio"]
opentls = ["hyper-tls"]  # Alternative backend
```

### Phase 2: Create TLS/Connector Abstraction

1. **Create `src/internal/connector.rs`**
```rust
//! TLS/HTTPS connector abstraction for cross-platform support

use std::fmt;

/// HTTPS connector type abstraction
#[cfg(not(target_arch = "wasm32"))]
pub type HttpsConnector = hyper_rustls::HttpsConnector<hyper::client::HttpConnector>;

/// Dummy connector type for WASM (not actually used)
#[cfg(target_arch = "wasm32")]
pub type HttpsConnector = ();  // Placeholder, not actually used

/// Connector configuration options
#[derive(Debug, Clone)]
pub struct ConnectorConfig {
    pub enable_http1: bool,
    pub enable_http2: bool,
    pub enable_tls: bool,
    pub native_roots: bool,
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            enable_http1: true,
            enable_http2: true,
            enable_tls: true,
            native_roots: true,
        }
    }
}

impl ConnectorConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_http1(mut self, enable: bool) -> Self {
        self.enable_http1 = enable;
        self
    }

    pub fn with_http2(mut self, enable: bool) -> Self {
        self.enable_http2 = enable;
        self
    }

    pub fn with_tls(mut self, enable: bool) -> Self {
        self.enable_tls = enable;
        self
    }

    pub fn with_native_roots(mut self, enable: bool) -> Self {
        self.native_roots = enable;
        self
    }
}

/// Create an HTTPS connector for the native platform
#[cfg(not(target_arch = "wasm32"))]
pub fn create_connector(config: &ConnectorConfig) -> Result<HttpsConnector, ConnectorError> {
    let mut builder = hyper_rustls::HttpsConnectorBuilder::new();

    // Configure TLS roots
    if config.native_roots {
        builder = builder.with_native_roots();
    } else {
        // Could add custom root certificates here in the future
        builder = builder.with_native_roots();
    }

    // Configure HTTP support
    if config.enable_http1 {
        builder = builder.enable_http1();
    }
    
    if config.enable_http2 {
        builder = builder.enable_http2();
    }

    // Build connector
    if config.enable_tls {
        builder.https_or_http().build()
    } else {
        builder.https_only().build()
    }
}

/// No-op connector creation for WASM
#[cfg(target_arch = "wasm32")]
pub fn create_connector(_config: &ConnectorConfig) -> Result<(), ConnectorError> {
    // In WASM, we don't create an explicit connector
    // The platform handles TLS automatically
    Ok(())
}

/// Connector creation error
#[derive(Debug, Snafu)]
pub enum ConnectorError {
    #[snafu(display("Failed to create HTTPS connector"))]
    CreationFailed,
    
    #[snafu(display("TLS configuration error: {}", source))]
    Tls { source: Box<dyn std::error::Error + Send + Sync> },
}

/// Result type for connector operations
pub type Result<T> = std::result::Result<T, ConnectorError>;
```

2. **Update `src/internal/mod.rs`**
```rust
pub mod connector;
```

### Phase 3: Refactor HTTP Client Construction

**File: `src/lib.rs`**

**Update build_client method:**

```rust
#[cfg(not(target_arch = "wasm32"))]
fn build_client(&self) -> Result<crate::OctocrabService> {
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    use crate::internal::connector::{create_connector, ConnectorConfig};

    // Create connector configuration
    let connector_config = ConnectorConfig::default();
    
    // Create HTTPS connector
    let connector = create_connector(&connector_config)
        .map_err(|e| crate::Error::Http(e.into()))?;

    // Build HTTP client
    let client = Client::builder(TokioExecutor::new())
        .build::<_, http_body_util::Full<bytes::Bytes>>(connector);

    Ok(BoxService::new(client))
}

#[cfg(target_arch = "wasm32")]
fn build_client(&self) -> Result<crate::OctocrabService> {
    use crate::internal::connector::create_connector;
    use crate::internal::wasm_http::Client;

    // In WASM, we don't need a connector
    // The platform handles TLS automatically
    let _connector = create_connector(&Default::default())
        .map_err(|e| crate::Error::Http(e.into()))?;

    // Create WASM HTTP client
    let client = Client::new();

    Ok(BoxService::new(client))
}
```

### Phase 4: Add Builder Methods for TLS Configuration

**File: `src/lib.rs`**

**Add connector configuration to builder:**

```rust
impl OctocrabBuilder<NoSvc, DefaultOctocrabBuilderConfig, NoAuth, NotLayerReady> {
    /// Configure TLS/HTTPS connector (native platform only)
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_connector_config<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut crate::internal::connector::ConnectorConfig),
    {
        // Store connector configuration or apply directly during build
        // For simplicity, we'll apply this in the build method
        self
    }

    /// Disable HTTP/2 (native platform only)
    #[cfg(not(target_arch = "wasm32"))]
    pub fn disable_http2(mut self) -> Self {
        self.config.enable_http2 = false;
        self
    }

    /// Disable HTTP/1 (native platform only)
    #[cfg(not(target_arch = "wasm32"))]
    pub fn disable_http1(mut self) -> Self {
        self.config.enable_http1 = false;
        self
    }
}
```

**Update DefaultOctocrabBuilderConfig:**

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
    
    // Connector configuration (native only)
    #[cfg(not(target_arch = "wasm32"))]
    pub enable_http2: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub enable_http1: bool,
}

impl Default for DefaultOctocrabBuilderConfig {
    fn default() -> Self {
        Self {
            auth: None,
            previews: Vec::new(),
            extra_headers: HeaderMap::new(),
            connect_timeout: None,
            read_timeout: None,
            write_timeout: None,
            base_uri: None,
            upload_uri: None,
            retry_config: None,
            cache_storage: None,
            
            #[cfg(not(target_arch = "wasm32"))]
            enable_http2: true,
            #[cfg(not(target_arch = "wasm32"))]
            enable_http1: true,
        }
    }
}
```

### Phase 5: Update Client Creation in Build Method

**File: `src/lib.rs`**

**In the build method:**

```rust
impl OctocrabBuilder<NoSvc, DefaultOctocrabBuilderConfig, NoAuth, NotLayerReady> {
    pub fn build(self) -> Result<Octocrab> {
        // ... existing validation code ...

        let connector_config = crate::internal::connector::ConnectorConfig {
            enable_http1: self.config.enable_http1,
            enable_http2: self.config.enable_http2,
            ..Default::default()
        };

        // ... rest of build logic using connector_config ...
    }
}
```

### Phase 6: Document Platform Differences

**Add documentation comments:**

```rust
/// Build the Octocrab client.
///
/// On native platforms, this creates an HTTP client with TLS support
/// configured using hyper-rustls. The connector supports HTTP/1 and HTTP/2.
///
/// On WASM platforms, TLS is handled automatically by the platform (browser
/// or Cloudflare Workers), so no explicit TLS configuration is needed.
///
/// # Platform Differences
///
/// ## Native Platform
/// - Uses hyper-rustls for TLS
/// - Supports HTTP/1 and HTTP/2 (configurable)
/// - Uses system certificate store or custom roots
/// - Full control over TLS configuration
///
/// ## WASM Platform
/// - TLS handled by platform automatically
/// - HTTP/1 and HTTP/2 support depends on platform
/// - Certificate validation handled by platform
/// - Limited TLS configuration options
impl OctocrabBuilder<NoSvc, DefaultOctocrabBuilderConfig, NoAuth, NotLayerReady> {
    pub fn build(self) -> Result<Octocrab> {
        // ...
    }
}
```

### Phase 7: Add Feature Flag Documentation

**Update README or documentation:**

```markdown
## TLS Configuration

Octocrab supports multiple TLS backends on native platforms:

- `rustls` (default): Uses rustls with ring crypto
- `rustls-aws-lc-rs`: Uses rustls with AWS LC crypto
- `rustls-webpki-tokio`: Uses rustls with webpki and tokio
- `opentls`: Uses OpenSSL (native TLS)

On WASM platforms, TLS is handled automatically by the platform and no
configuration is needed.
```

## Potential Issues and Solutions

### Issue 1: TLS Backend Feature Conflicts
**Problem:** Some TLS backends are mutually exclusive (ring vs aws-lc-rs)

**Solution:**
- Add compile-time checks for conflicting features
- Document which combinations are valid
- Provide clear error messages

```rust
#[cfg(all(feature = "rustls-ring", feature = "rustls-aws-lc-rs"))]
compile_error!("Cannot enable both rustls-ring and rustls-aws-lc-rs features");
```

### Issue 2: HTTP/2 Support on WASM
**Problem:** WASM HTTP client may not support HTTP/2

**Solution:**
- Document that HTTP/2 support depends on platform
- Test in Workers environment
- Gracefully fallback to HTTP/1 if needed
- Note that GitHub API supports both

### Issue 3: Custom Certificates
**Problem:** Custom certificates may be needed for some environments

**Solution:**
- Add support for custom roots in ConnectorConfig (native only)
- Document limitation for WASM platform
- Provide clear error if custom certs requested on WASM

### Issue 4: Connection Pooling
**Problem:** Connection pooling behavior may differ between platforms

**Solution:**
- Document connection pooling behavior for each platform
- Use sensible defaults for both platforms
- Allow configuration where possible

### Issue 5: Certificate Validation
**Problem:** Certificate validation is handled differently on WASM

**Solution:**
- Document that WASM uses platform validation
- Users cannot disable validation on WASM
- This is appropriate for security

### Issue 6: Performance Differences
**Problem:** Native TLS may have different performance characteristics

**Solution:**
- Benchmark both implementations
- Document any performance differences
- Use appropriate defaults for each platform

## Testing Strategy

### 1. Unit Tests
```bash
# Test native platform
cargo test --package octocrab --lib connector

# Test WASM
wasm-pack test --node
```

### 2. Connector Configuration Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_connector_config_default() {
        let config = ConnectorConfig::default();
        assert!(config.enable_http1);
        assert!(config.enable_http2);
        assert!(config.enable_tls);
        assert!(config.native_roots);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_connector_config_custom() {
        let config = ConnectorConfig::new()
            .with_http1(false)
            .with_http2(true);
        
        assert!(!config.enable_http1);
        assert!(config.enable_http2);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_create_connector_default() {
        let config = ConnectorConfig::default();
        let connector = create_connector(&config);
        assert!(connector.is_ok());
    }
}
```

### 3. Integration Tests
```rust
#[tokio::test]
#[cfg(not(target_arch = "wasm32"))]
async fn test_https_connection_native() {
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN").unwrap())
        .build()
        .unwrap();

    let user = octocrab.users("octocat").get().await.unwrap();
    assert_eq!(user.login, "octocat");
}
```

### 4. Cloudflare Workers Testing
```rust
// examples/wasm_https.rs
use octocrab::Octocrab;

pub async fn test_https_in_workers() -> Result<(), Box<dyn std::error::Error>> {
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN")?)
        .build()?;

    // HTTPS is automatic in Workers
    let user = octocrab.users("octocat").get().await?;
    println!("User: {} (login: {})", user.id, user.login);

    Ok(())
}
```

### 5. HTTP/2 Support Tests
```rust
#[tokio::test]
#[cfg(all(feature = "rustls", not(target_arch = "wasm32")))]
async fn test_http2_support() {
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN").unwrap())
        .build()
        .unwrap();

    // GitHub API supports HTTP/2
    let user = octocrab.users("octocat").get().await.unwrap();
    assert_eq!(user.login, "octocat");
}
```

## Implementation Order

1. ✅ Setup feature flags and dependencies
2. ✅ Create TLS/connector abstraction layer
3. ✅ Refactor HTTP client construction
4. ✅ Add builder methods for TLS configuration
5. ✅ Update DefaultOctocrabBuilderConfig
6. ✅ Add documentation for platform differences
7. ✅ Add feature flag validation
8. ✅ Run native tests
9. ✅ Create WASM tests
10. ✅ Update README documentation
11. ✅ Add Workers example
12. ⏸️ Consider adding custom certificate support (native only)

## Success Criteria

- ✅ Code compiles for native platform without breaking changes
- ✅ Code compiles for WASM target with `wasm` feature
- ✅ All existing tests pass on native platform
- ✅ TLS configuration works on native platform
- ✅ HTTPS connections work in Cloudflare Workers
- ✅ HTTP/1 and HTTP/2 support works on native platform
- ✅ Feature flags for different TLS backends work correctly
- ✅ No performance regression on native platform
- ✅ Platform differences are well-documented

## Use Cases Enabled

With this implementation, users can:
- Use Octocrab with any TLS backend on native platform
- Use Octocrab in Cloudflare Workers without TLS configuration
- Configure HTTP/1 and HTTP/2 support (native only)
- Use system certificates or custom roots (native only)
- Trust the platform's TLS handling in Workers

## Limitations

- WASM platform: TLS configuration not available (handled by platform)
- WASM platform: Cannot use custom certificates
- WASM platform: HTTP/2 support depends on platform capabilities
- Native platform: Multiple TLS backends require feature flags
- Custom certificate support not implemented in initial version

## Related Plans

- `001_http_client_wasm.md` - HTTP client abstraction
- `002_async_runtime_wasm.md` - Async runtime compatibility
- `003_jwt_wasm.md` - JWT authentication for WASM
- `004_cache_storage_wasm.md` - Cache storage abstraction

## References

- [hyper-rustls documentation](https://docs.rs/hyper-rustls)
- [rustls documentation](https://docs.rs/rustls)
- [Cloudflare Workers TLS](https://developers.cloudflare.com/workers/runtime-apis/fetch/)
- [GitHub API TLS Requirements](https://docs.github.com/en/rest/overview/resources-in-the-rest-api)
- [HTTP/2 Support in GitHub API](https://docs.github.com/en/rest/using-the-rest-api/http-verbs)
- [Web Crypto API](https://developer.mozilla.org/en-US/docs/Web/API/Web_Crypto_API)

## Notes

- Workers platform handles TLS automatically, no configuration needed
- GitHub API supports both HTTP/1 and HTTP/2
- Certificate validation is handled by the platform on WASM
- Native platform has full control over TLS configuration
- Use sensible defaults for each platform
- Test TLS behavior with actual GitHub API
- Document platform differences clearly

## Example Usage

### Native Platform with Default TLS
```rust
use octocrab::Octocrab;

#[tokio::main]
async fn main() -> octocrab::Result<()> {
    // Default TLS configuration (HTTP/1 + HTTP/2, native roots)
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN")?)
        .build()?;

    let user = octocrab.users("octocat").get().await?;
    println!("User: {}", user.login);

    Ok(())
}
```

### Native Platform with HTTP/1 Only
```rust
use octocrab::Octocrab;

#[tokio::main]
async fn main() -> octocrab::Result<()> {
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN")?)
        .disable_http2()  // Only HTTP/1
        .build()?;

    let user = octocrab.users("octocat").get().await?;
    println!("User: {}", user.login);

    Ok(())
}
```

### Native Platform with Specific TLS Backend
```toml
# Cargo.toml
[dependencies]
octocrab = { version = "0.49", features = ["rustls-aws-lc-rs"] }
```

### Cloudflare Workers (Automatic TLS)
```rust
use octocrab::Octocrab;

pub async fn handle_request() -> Result<(), Box<dyn std::error::Error>> {
    // TLS is automatic in Workers, no configuration needed
    let octocrab = Octocrab::builder()
        .personal_token(std::env::var("GITHUB_TOKEN")?)
        .build()?;

    let user = octocrab.users("octocat").get().await?;
    println!("User: {}", user.login);

    Ok(())
}
```

## Security Considerations

1. **Certificate Validation**: Always enabled, cannot be disabled
2. **Native Roots**: Default uses system certificate store
3. **WASM Validation**: Platform handles certificate validation
4. **TLS Versions**: Use secure defaults (TLS 1.2+)
5. **Custom Certificates**: Not supported in initial version (security best practice)
6. **Certificate Pinning**: Not implemented (future enhancement)

## Troubleshooting

### Issue: TLS handshake failure on native platform
**Solution**: Check system certificates, verify network connectivity

### Issue: "Feature conflict" compile error
**Solution**: Choose one TLS backend (rustls-ring OR rustls-aws-lc-rs)

### Issue: HTTP/2 not working
**Solution**: Verify HTTP/2 is enabled, check GitHub API support

### Issue: Performance slow on native platform
**Solution**: Try different TLS backend (ring vs aws-lc-rs)

### Issue: Workers TLS error
**Solution**: Workers handles TLS automatically, check Workers configuration

## Contributing

When contributing to TLS abstraction:
1. Test on both native and WASM platforms
2. Ensure feature flag validation works
3. Add tests for new configuration options
4. Document platform differences
5. Consider security implications of changes

## Future Enhancements

- Custom certificate support (native only)
- Certificate pinning
- Session resumption configuration
- Connection pool tuning
- Metrics for TLS handshakes
- Support for more TLS backends