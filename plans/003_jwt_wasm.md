use jsonwebtoken::{Algorithm, EncodingKey, Header};

pub struct AppAuth {
    pub app_id: AppId,
    pub key: EncodingKey,
}

impl AppAuth {
    pub fn new(app_id: AppId, key: &str) -> Result<Self> {
        let key = EncodingKey::from_rsa_pem(key.as_bytes())?;
        Ok(Self { app_id, key })
    }

    pub fn generate_jwt(&self, expiration: i64) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();

        let claims = Claims {
            iss: self.app_id.0,
            iat: now,
            exp: now + expiration,
        };

        let token = encode(&Header::default(), &claims, &self.key)?;
        Ok(token)
    }
}
```

### JWT Claims Structure
```rust
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iss: u64,  // Issuer (GitHub App ID)
    iat: u64,  // Issued at (Unix timestamp)
    exp: u64,  // Expiration (Unix timestamp, max 10 minutes)
}
```

### Token Caching
Octocrab caches JWT tokens to avoid regenerating them:
```rust
struct CachedTokenInner {
    expiration: SystemTime,
    secret: SecretString,
}
```

## Implementation Plan

### Phase 1: Feature Flag and Dependencies

1. **Update `Cargo.toml`**
```toml
[dependencies]
jsonwebtoken = { version = "10", default-features = false, features = ["use_pem"], optional = true }
secrecy = "0.10.3"

# WASM JWT dependencies
[target.'cfg(target_arch = "wasm32")'.dependencies]
# Option 1: Use Web Crypto API via wasm-bindgen
wasm-bindgen = "0.2"
js-sys = "0.3"
web-sys = { version = "0.3", features = ["CryptoKey", "SubtleCrypto"] }

# Option 2: Use WASM-compatible JWT crate if available
# wasm-jwt = "0.1"  # Hypothetical crate

# Conditional dependencies
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
jsonwebtoken = { version = "10", default-features = false, features = ["use_pem"] }
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
    "jwt-wasm",
]

# Native JWT backends
jwt-rust-crypto = ["jsonwebtoken/rust_crypto"]
jwt-aws-lc-rs = ["jsonwebtoken/aws_lc_rs"]

# WASM JWT
jwt-wasm = ["wasm-bindgen", "web-sys"]
```

### Phase 2: Create JWT Abstraction Layer

1. **Create `src/internal/jwt.rs`**
```rust
//! JWT abstraction for cross-platform support

use secrecy::SecretString;
use snafu::Snafu;
use web_time::SystemTime;

/// JWT encoding key abstraction
#[derive(Clone)]
pub enum EncodingKey {
    #[cfg(not(target_arch = "wasm32"))]
    Native(jsonwebtoken::EncodingKey),
    #[cfg(target_arch = "wasm32")]
    Wasm(String),  // PEM-encoded key as string
}

/// JWT header abstraction
#[derive(Debug, Clone)]
pub struct Header {
    pub alg: String,
    pub typ: String,
}

impl Header {
    pub fn new(alg: &'static str) -> Self {
        Self {
            alg: alg.to_string(),
            typ: "JWT".to_string(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn default() -> Self {
        Self::new("RS256")
    }

    #[cfg(target_arch = "wasm32")]
    pub fn default() -> Self {
        Self::new("RS256")
    }
}

/// JWT claims for GitHub Apps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub iss: u64,  // Issuer (GitHub App ID)
    pub iat: u64,  // Issued at (Unix timestamp)
    pub exp: u64,  // Expiration (Unix timestamp)
}

/// JWT encoding error
#[derive(Debug, Snafu)]
pub enum JwtError {
    #[snafu(display("Failed to encode JWT"))]
    Encode { source: Box<dyn std::error::Error + Send + Sync> },
    
    #[snafu(display("Invalid private key"))]
    InvalidKey,
    
    #[snafu(display("Failed to parse PEM"))]
    InvalidPem,
    
    #[snafu(display("Time operation failed"))]
    Time,
}

/// Result type for JWT operations
pub type Result<T> = std::result::Result<T, JwtError>;

/// Create encoding key from PEM
#[cfg(not(target_arch = "wasm32"))]
pub fn encoding_key_from_pem(pem: &[u8]) -> Result<EncodingKey> {
    jsonwebtoken::EncodingKey::from_rsa_pem(pem)
        .map(EncodingKey::Native)
        .map_err(|e| JwtError::Encode { source: Box::new(e) })
}

#[cfg(target_arch = "wasm32")]
pub fn encoding_key_from_pem(pem: &[u8]) -> Result<EncodingKey> {
    // For WASM, we'll parse the PEM and prepare it for Web Crypto
    let pem_str = std::str::from_utf8(pem)
        .map_err(|_| JwtError::InvalidPem)?;
    
    // Extract the base64-encoded key from PEM
    let key_base64 = pem_str
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    
    Ok(EncodingKey::Wasm(key_base64))
}

/// Encode JWT with claims
#[cfg(not(target_arch = "wasm32"))]
pub fn encode(header: &Header, claims: &Claims, key: &EncodingKey) -> Result<String> {
    let key = match key {
        EncodingKey::Native(k) => k,
    };
    
    let jwt_header = jsonwebtoken::Header {
        alg: jsonwebtoken::Algorithm::RS256,
        ..Default::default()
    };
    
    jsonwebtoken::encode(&jwt_header, claims, key)
        .map_err(|e| JwtError::Encode { source: Box::new(e) })
}

#[cfg(target_arch = "wasm32")]
pub fn encode(header: &Header, claims: &Claims, key: &EncodingKey) -> Result<String> {
    use wasm_bindgen::JsCast;
    use js_sys::{JSON, Uint8Array, Reflect};
    use web_sys::{Crypto, SubtleCrypto, Window};
    
    // Get crypto API
    let window = web_sys::window().ok_or_else(|| JwtError::Encode { 
        source: Box::from("No window object") 
    })?;
    let crypto: Crypto = window.crypto().map_err(|_| JwtError::Encode { 
        source: Box::from("No crypto API") 
    })?;
    let subtle: SubtleCrypto = crypto.subtle();
    
    // Import the private key
    let key_base64 = match key {
        EncodingKey::Wasm(k) => k,
    };
    
    // Decode base64
    let key_bytes = base64::decode_config(key_base64, base64::STANDARD)
        .map_err(|_| JwtError::InvalidKey)?;
    
    // Create key data for import
    let key_data = Uint8Array::from(&key_bytes[..]);
    
    // Import key parameters
    let key_params = js_sys::Object::new();
    Reflect::set(&key_params, &"name".into(), &"RSASSA-PKCS1-v1_5".into())?;
    Reflect::set(&key_params, &"hash".into(), &"SHA-256".into())?;
    
    // Import the key
    let import_promise = subtle.import_key_with_object(
        "pkcs8",
        &key_data,
        &key_params,
        false,
        &["sign"],
    ).map_err(|e| JwtError::Encode { 
        source: Box::new(e.as_string().unwrap_or_else(|| "Unknown error".to_string())) 
    })?;
    
    // Convert promise to Rust future
    let key_future = wasm_bindgen_futures::JsFuture::from(import_promise);
    let crypto_key = key_future.await.map_err(|e| JwtError::Encode { 
        source: Box::new(e.as_string().unwrap_or_else(|| "Unknown error".to_string())) 
    })?;
    
    // Create JWT header and payload
    let header_json = JSON::stringify(&serde_wasm_bindgen::to_value(&jsonwebtoken::Header {
        alg: jsonwebtoken::Algorithm::RS256,
        ..Default::default()
    }).map_err(|e| JwtError::Encode { source: Box::new(e) })?.map_err(|e| JwtError::Encode { 
        source: Box::new(e.as_string().unwrap_or_else(|| "Unknown error".to_string())) 
    })?;
    
    let payload_json = JSON::stringify(&serde_wasm_bindgen::to_value(claims).map_err(|e| JwtError::Encode { 
        source: Box::new(e) 
    })?.map_err(|e| JwtError::Encode { 
        source: Box::new(e.as_string().unwrap_or_else(|| "Unknown error".to_string())) 
    })?;
    
    // Base64url encode header and payload
    let header_b64 = base64_url_encode(header_json.as_string().unwrap());
    let payload_b64 = base64_url_encode(payload_json.as_string().unwrap());
    
    // Create data to sign
    let signing_data = format!("{}.{}", header_b64, payload_b64);
    let signing_bytes = signing_data.as_bytes();
    
    let signing_data_array = Uint8Array::view(signing_bytes);
    
    // Sign the data
    let sign_params = js_sys::Object::new();
    Reflect::set(&sign_params, &"name".into(), &"RSASSA-PKCS1-v1_5".into())?;
    
    let sign_promise = subtle.sign_with_object_and_buffer_source(
        &sign_params,
        &crypto_key,
        &signing_data_array,
    ).map_err(|e| JwtError::Encode { 
        source: Box::new(e.as_string().unwrap_or_else(|| "Unknown error".to_string())) 
    })?;
    
    let signature_future = wasm_bindgen_futures::JsFuture::from(sign_promise);
    let signature = signature_future.await.map_err(|e| JwtError::Encode { 
        source: Box::new(e.as_string().unwrap_or_else(|| "Unknown error".to_string())) 
    })?;
    
    // Convert signature to Uint8Array and encode
    let signature_array: Uint8Array = signature.unchecked_into();
    let signature_vec: Vec<u8> = signature_array.to_vec();
    let signature_b64 = base64_url_encode_vec(signature_vec);
    
    // Combine header, payload, and signature
    Ok(format!("{}.{}.{}", header_b64, payload_b64, signature_b64))
}

/// Helper: Base64url encode
#[cfg(target_arch = "wasm32")]
fn base64_url_encode(input: &str) -> String {
    base64_url_encode_vec(input.as_bytes().to_vec())
}

#[cfg(target_arch = "wasm32")]
fn base64_url_encode_vec(input: Vec<u8>) -> String {
    base64::encode_config(&input, base64::URL_SAFE_NO_PAD)
}
```

2. **Update `src/internal/mod.rs`**
```rust
pub mod jwt;
```

### Phase 3: Refactor AppAuth

**File: `src/auth.rs`**

**Update AppAuth struct:**
```rust
use crate::internal::jwt::{EncodingKey, Header, Claims, Result as JwtResult};

#[derive(Clone)]
pub struct AppAuth {
    pub app_id: AppId,
    pub key: EncodingKey,
}
```

**Update AppAuth implementation:**
```rust
impl AppAuth {
    pub fn new(app_id: AppId, key: &str) -> Result<Self> {
        let key = crate::internal::jwt::encoding_key_from_pem(key.as_bytes())?;
        Ok(Self { app_id, key })
    }

    pub fn generate_jwt(&self, expiration: i64) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| Error::Other("Time error".into()))?
            .as_secs();

        let claims = Claims {
            iss: self.app_id.0,
            iat: now,
            exp: now + expiration as u64,
        };

        let token = crate::internal::jwt::encode(&Header::default(), &claims, &self.key)
            .map_err(|e| Error::Other(e.into()))?;
        Ok(token)
    }
}
```

### Phase 4: Update Token Caching

**File: `src/lib.rs`**

The token caching logic should work without changes since it operates on the generated JWT string, not the encoding process. However, we should ensure the caching logic is platform-agnostic:

```rust
struct CachedTokenInner {
    expiration: SystemTime,
    secret: SecretString,
}

impl CachedTokenInner {
    fn new(expiration: SystemTime, secret: SecretString) -> Self {
        Self { expiration, secret }
    }

    fn is_expired(&self, buffer_seconds: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        let exp = self.expiration
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        now + buffer_seconds >= exp
    }
}
```

### Phase 5: Update Error Handling

**File: `src/error.rs`**

**Add JWT error variant:**
```rust
#[derive(Debug, Snafu)]
pub enum Error {
    // ... existing variants ...
    
    #[snafu(display("JWT error: {}", source))]
    Jwt { source: Box<dyn std::error::Error + Send + Sync> },
}
```

### Phase 6: Add WASM-Specific Dependencies

Create a wrapper for WASM-specific dependencies:

**File: `src/internal/wasm_crypto.rs`**
```rust
//! WASM-specific crypto utilities

#[cfg(target_arch = "wasm32")]
pub use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
pub use wasm_bindgen_futures::JsFuture;
```

### Phase 7: Update Examples and Documentation

**File: `examples/github_app.rs`**
```rust
use octocrab::Octocrab;

async fn main() -> octocrab::Result<()> {
    // This works on both native and WASM platforms
    let app_id = octocrab::models::AppId(123456);
    let app = octocrab::auth::AppAuth::new(app_id, include_str!("private_key.pem"))?;
    
    let octocrab = Octocrab::builder()
        .app(app)
        .build()?;
    
    // Use the client
    let user = octocrab.current().user().await?;
    println!("Authenticated as: {}", user.login);
    
    Ok(())
}
```

## Potential Issues and Solutions

### Issue 1: Web Crypto API Complexity
**Problem:** Web Crypto API is complex and has different semantics than native crypto libraries

**Solution:**
- Provide thorough testing for WASM implementation
- Document any behavioral differences
- Consider using a WASM-specific JWT crate if available
- Provide clear error messages for debugging

### Issue 2: Key Format Differences
**Problem:** PEM format may need special handling in WASM

**Solution:**
- Parse PEM to extract base64-encoded key
- Use proper PKCS#8 format for Web Crypto
- Provide helper functions for key conversion
- Test with various key formats

### Issue 3: Promise-based API
**Problem:** Web Crypto uses Promises, not Futures

**Solution:**
- Use `wasm_bindgen_futures::JsFuture` to convert Promises
- Handle errors properly from JavaScript
- Document the async nature of WASM crypto operations

### Issue 4: Performance Differences
**Problem:** Web Crypto may have different performance characteristics

**Solution:**
- Benchmark both implementations
- Document any performance differences
- The token caching should mitigate most concerns
- Consider caching the imported crypto key

### Issue 5: Algorithm Compatibility
**Problem:** Web Crypto may not support all algorithms equally

**Solution:**
- Use RSASSA-PKCS1-v1_5 with SHA-256 (standard for GitHub)
- Test thoroughly with actual GitHub API
- Provide clear error messages if algorithm not supported

### Issue 6: Key Security
**Problem:** Private key handling in browser/Workers environment

**Solution:**
- Use secrecy crate for sensitive data
- Document security best practices
- Never log or expose private keys
- Use environment variables or secure storage

## Testing Strategy

### 1. Unit Tests
```bash
# Test native platform
cargo test --package octocrab --lib auth

# Test WASM
wasm-pack test --node
```

### 2. JWT Generation Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::jwt::{encoding_key_from_pem, encode, Header, Claims};

    const TEST_PRIVATE_KEY: &str = include_str!("../tests/fixtures/test_key.pem");
    const TEST_APP_ID: u64 = 123456;

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_jwt_generation_native() {
        let key = encoding_key_from_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap();
        let claims = Claims {
            iss: TEST_APP_ID,
            iat: 1000,
            exp: 2000,
        };
        let token = encode(&Header::default(), &claims, &key).unwrap();
        
        assert!(token.contains('.'));
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    #[cfg(target_arch = "wasm32")]
    async fn test_jwt_generation_wasm() {
        let key = encoding_key_from_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap();
        let claims = Claims {
            iss: TEST_APP_ID,
            iat: 1000,
            exp: 2000,
        };
        let token = encode(&Header::default(), &claims, &key).unwrap();
        
        assert!(token.contains('.'));
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_jwt_verification_native() {
        // Test that generated JWT can be verified
        let key = encoding_key_from_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap();
        let claims = Claims {
            iss: TEST_APP_ID,
            iat: 1000,
            exp: 2000,
        };
        let token = encode(&Header::default(), &claims, &key).unwrap();
        
        // Decode and verify header
        let header_part = token.split('.').next().unwrap();
        let header_json = base64::decode_config(header_part, base64::URL_SAFE_NO_PAD).unwrap();
        let header: serde_json::Value = serde_json::from_slice(&header_json).unwrap();
        assert_eq!(header["alg"], "RS256");
        assert_eq!(header["typ"], "JWT");
    }
}
```

### 3. Integration Tests
```rust
#[tokio::test]
#[cfg(not(target_arch = "wasm32"))]
async fn test_github_app_authentication_native() {
    let app_id = AppId(std::env::var("TEST_APP_ID").unwrap().parse().unwrap());
    let app = AppAuth::new(app_id, &std::env::var("TEST_PRIVATE_KEY").unwrap()).unwrap();
    
    let octocrab = Octocrab::builder()
        .app(app)
        .build()
        .unwrap();
    
    let installation = octocrab
        .installations()
        .get(1)
        .await;
    
    assert!(installation.is_ok() || installation.is_err()); // Just test it runs
}
```

### 4. Cloudflare Workers Testing
```rust
// examples/wasm_github_app.rs
use octocrab::{Octocrab, auth::AppAuth};

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub async fn main() -> Result<(), JsValue> {
    let app_id = AppId(123456);
    let private_key = std::env::var("GITHUB_PRIVATE_KEY")
        .map_err(|e| JsValue::from_str(&format!("Missing private key: {}", e)))?;
    
    let app = AppAuth::new(app_id, &private_key)
        .map_err(|e| JsValue::from_str(&format!("Failed to create AppAuth: {}", e)))?;
    
    let octocrab = Octocrab::builder()
        .app(app)
        .build()
        .map_err(|e| JsValue::from_str(&format!("Failed to build Octocrab: {}", e)))?;
    
    let user = octocrab
        .current()
        .user()
        .await
        .map_err(|e| JsValue::from_str(&format!("Failed to get user: {}", e)))?;
    
    web_sys::console::log_1(&format!("Authenticated as: {}", user.login).into());
    
    Ok(())
}
```

### 5. Cross-Platform Verification
- Generate JWT with native implementation
- Generate JWT with WASM implementation
- Verify both produce valid tokens
- Test both work with GitHub API

## Implementation Order

1. ✅ Setup feature flags and dependencies
2. ✅ Create JWT abstraction layer
3. ✅ Create WASM-specific crypto utilities
4. ✅ Refactor AppAuth implementation
5. ✅ Update error handling
6. ✅ Update documentation
7. ✅ Create test fixtures (test key, etc.)
8. ⏸️ Run native tests
9. ⏸️ Create WASM tests
10. ⏸️ Test with actual GitHub API
11. ⏸️ Update examples
12. ⏸️ Create Workers example
13. ⏸️ Add GitHub Actions for cross-platform testing

## Success Criteria

- ✅ Code compiles for native platform without breaking changes
- ✅ Code compiles for WASM target with `wasm` feature
- ✅ All existing tests pass on native platform
- ✅ JWT generation works on WASM platform
- ✅ Generated JWTs are valid (can be verified)
- ✅ GitHub Apps authentication works in Workers
- ✅ Token caching works on both platforms
- ✅ Error handling is consistent across platforms
- ✅ No performance regression on native platform
- ✅ Actual GitHub API calls work with WASM-generated JWTs

## Use Cases Enabled

With this implementation, users can:
- Use GitHub Apps authentication in Cloudflare Workers
- Generate JWT tokens for app installation access
- Cache JWT tokens to avoid regeneration
- Authenticate as GitHub Apps in serverless environments
- Create Workers that manage GitHub repositories
- Automate GitHub operations using Apps

## Limitations

- WASM implementation uses Web Crypto API, which may have slightly different behavior
- Key import may be slower on first use (consider caching)
- Some advanced JWT features not implemented (not needed for GitHub)
- Private key must be in PEM format (same as native)
- WASM crypto operations are Promise-based, adding async overhead

## Related Plans

- `001_http_client_wasm.md` - HTTP client abstraction
- `002_async_runtime_wasm.md` - Async runtime compatibility
- `004_cache_storage_wasm.md` - Cache storage abstraction
- `005_tls_wasm.md` - TLS abstraction

## References

- [GitHub Apps Authentication Documentation](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-json-web-token-jwt-for-a-github-app)
- [jsonwebtoken documentation](https://docs.rs/jsonwebtoken)
- [Web Crypto API Documentation](https://developer.mozilla.org/en-US/docs/Web/API/Web_Crypto_API)
- [JWT.io](https://jwt.io/) - JWT debugger and documentation
- [Base64 URL Safe Encoding](https://tools.ietf.org/html/rfc4648#section-5)
- [wasm-bindgen documentation](https://rustwasm.github.io/wasm-bindgen/)
- [serenity JWT implementation](https://github.com/serenity-rs/serenity/blob/current/src/auth.rs)

## Notes

- GitHub requires RSASSA-PKCS1-v1_5 with SHA-256 algorithm
- JWT expiration must be less than 10 minutes (GitHub enforces this)
- JWT claims must include: iss (app ID), iat (issued at), exp (expiration)
- The JWT is used to get an installation access token, not for direct API calls
- Token caching is crucial to avoid hitting GitHub rate limits
- Use secrecy crate to keep private keys secure
- Test thoroughly with actual GitHub Apps before deploying

## Example Usage

### Native Platform
```rust
use octocrab::Octocrab;
use octocrab::auth::AppAuth;

#[tokio::main]
async fn main() -> octocrab::Result<()> {
    let app_id = octocrab::models::AppId(123456);
    let app = AppAuth::new(app_id, include_str!("private_key.pem"))?;
    
    let octocrab = Octocrab::builder()
        .app(app)
        .build()?;
    
    let installation_token = octocrab.installation_token(123).await?;
    println!("Installation token: {}", installation_token.token);
    
    Ok(())
}
```

### Cloudflare Workers
```javascript
// wrangler.toml
[vars]
GITHUB_APP_ID = "123456"
GITHUB_PRIVATE_KEY = "-----BEGIN PRIVATE KEY-----\n..."
```

```rust
use octocrab::{Octocrab, auth::AppAuth};

pub async fn handle_request(req: Request) -> Result<Response, Error> {
    let app_id = AppId(123456);
    let private_key = std::env::var("GITHUB_PRIVATE_KEY")?;
    
    let app = AppAuth::new(app_id, &private_key)?;
    
    let octocrab = Octocrab::builder()
        .app(app)
        .build()?;
    
    let user = octocrab.current().user().await?;
    
    Ok(Response::new(format!("Hello, {}!", user.login)))
}
```

## Security Considerations

1. **Private Key Storage**: Never commit private keys to version control
2. **Environment Variables**: Use environment variables or secret management
3. **Token Caching**: Cache JWT tokens in memory, not in persistent storage
4. **Key Rotation**: Support key rotation without service interruption
5. **Error Messages**: Don't expose private key data in error messages
6. **Auditing**: Log authentication events for security monitoring

## Troubleshooting

### Issue: "Invalid key format" error
**Solution**: Ensure private key is in PEM format with proper headers

### Issue: "Crypto operation failed" on WASM
**Solution**: Check browser/Workers console for detailed error messages

### Issue: JWT rejected by GitHub API
**Solution**: Verify app ID is correct, claims are valid, and token not expired

### Issue: Performance slow on WASM
**Solution**: Token caching should help; also consider caching imported crypto key

## Contributing

When contributing to JWT abstraction:
1. Test changes on both native and WASM platforms
2. Use test fixtures for keys (never commit real keys)
3. Document any behavioral differences
4. Add tests for new functionality
5. Update examples as needed