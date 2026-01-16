//! JWT abstraction for cross-platform support
//!
//! This module provides a platform-agnostic interface for JWT encoding/decoding:
//! - Native platforms (Linux, macOS, Windows): Uses `jsonwebtoken` crate
//! - WASM platforms (Cloudflare Workers): Uses Web Crypto API

use snafu::Snafu;

/// JWT encoding key abstraction for cross-platform support
#[derive(Clone)]
pub enum EncodingKey {
    #[cfg(not(target_arch = "wasm32"))]
    Native(jsonwebtoken::EncodingKey),
    #[cfg(target_arch = "wasm32")]
    Wasm(String), // PEM-encoded key as string (base64-encoded key without PEM headers)
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
}

impl Default for Header {
    fn default() -> Self {
        Self::new("RS256")
    }
}

/// JWT claims for GitHub Apps authentication
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Claims {
    pub iss: u64, // Issuer (GitHub App ID)
    pub iat: u64, // Issued at (Unix timestamp)
    pub exp: u64, // Expiration (Unix timestamp, max 10 minutes)
}

/// JWT encoding error types
#[derive(Debug, Snafu)]
pub enum JwtError {
    #[snafu(display("Failed to encode JWT: {message}"))]
    Encode { message: String },

    #[snafu(display("Invalid private key format"))]
    InvalidKey,

    #[snafu(display("Failed to parse PEM format"))]
    InvalidPem,

    #[snafu(display("Time operation failed"))]
    Time,

    #[snafu(display("Crypto API unavailable: {message}"))]
    CryptoUnavailable { message: String },

    #[snafu(display("Web Crypto operation failed: {message}"))]
    WebCryptoFailed { message: String },
}

/// Result type for JWT operations
pub type Result<T> = std::result::Result<T, JwtError>;

/// Create encoding key from PEM format
#[cfg(not(target_arch = "wasm32"))]
pub fn encoding_key_from_pem(pem: &[u8]) -> Result<EncodingKey> {
    jsonwebtoken::EncodingKey::from_rsa_pem(pem)
        .map(EncodingKey::Native)
        .map_err(|e| JwtError::Encode {
            message: e.to_string(),
        })
}

/// Create encoding key from PEM format for WASM
#[cfg(target_arch = "wasm32")]
pub fn encoding_key_from_pem(pem: &[u8]) -> Result<EncodingKey> {
    // For WASM, we parse the PEM and extract the base64-encoded key
    let pem_str = std::str::from_utf8(pem).map_err(|_| JwtError::InvalidPem)?;

    // Extract the base64-encoded key from PEM (remove headers and newlines)
    let key_base64 = pem_str
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");

    Ok(EncodingKey::Wasm(key_base64))
}

/// Encode JWT with claims for native platforms
#[cfg(not(target_arch = "wasm32"))]
pub fn encode(_header: &Header, claims: &Claims, key: &EncodingKey) -> Result<String> {
    let key = match key {
        EncodingKey::Native(k) => k,
    };

    let jwt_header = jsonwebtoken::Header {
        alg: jsonwebtoken::Algorithm::RS256,
        ..Default::default()
    };

    jsonwebtoken::encode(&jwt_header, claims, key).map_err(|e| JwtError::Encode {
        message: e.to_string(),
    })
}

/// Encode JWT with claims for WASM platforms using Web Crypto API
#[cfg(target_arch = "wasm32")]
pub fn encode(header: &Header, claims: &Claims, key: &EncodingKey) -> Result<String> {
    use js_sys::{Object, Reflect, Uint8Array, JSON};
    use wasm_bindgen::JsCast;
    use web_sys::{Crypto, SubtleCrypto, Window};

    // Get crypto API
    let window = web_sys::window().ok_or_else(|| JwtError::CryptoUnavailable {
        message: "No window object".to_string(),
    })?;
    let crypto: Crypto = window.crypto().map_err(|_| JwtError::CryptoUnavailable {
        message: "No crypto API".to_string(),
    })?;
    let subtle: SubtleCrypto = crypto.subtle();

    // Extract key data
    let key_base64 = match key {
        EncodingKey::Wasm(k) => k,
    };

    // Decode base64 to get raw key bytes
    let key_bytes = {
        use base64::{engine::general_purpose, Engine as _};
        general_purpose::STANDARD
            .decode(key_base64)
            .map_err(|_| JwtError::InvalidKey)?
    };

    // Create key data for import
    let key_data = Uint8Array::from(&key_bytes[..]);

    // Import key parameters for RSASSA-PKCS1-v1_5 with SHA-256
    let key_params = Object::new();
    Reflect::set(&key_params, &"name".into(), &"RSASSA-PKCS1-v1_5".into()).map_err(|e| {
        JwtError::Encode {
            message: format!("Failed to set algorithm name: {:?}", e),
        }
    })?;
    Reflect::set(&key_params, &"hash".into(), &"SHA-256".into()).map_err(|e| JwtError::Encode {
        message: format!("Failed to set hash algorithm: {:?}", e),
    })?;

    // Import the private key
    let import_promise = subtle
        .import_key_with_object("pkcs8", &key_data, &key_params, false, &["sign"])
        .map_err(|e| JwtError::WebCryptoFailed {
            message: format!("Failed to import key: {:?}", e),
        })?;

    // Convert promise to Rust future
    let key_future = wasm_bindgen_futures::JsFuture::from(import_promise);
    let crypto_key = key_future.await.map_err(|e| {
        let js_string = e.as_string().unwrap_or_else(|| "Unknown error".to_string());
        JwtError::WebCryptoFailed {
            message: format!("Key import failed: {}", js_string),
        }
    })?;

    // Create JWT header
    let header_value = serde_json::json!({
        "alg": "RS256",
        "typ": "JWT"
    });
    let header_json = JSON::stringify(&header_value)
        .map_err(|e| JwtError::Encode {
            message: format!("Failed to stringify header: {:?}", e),
        })?
        .as_string()
        .ok_or_else(|| JwtError::Encode {
            message: "Header JSON is null".to_string(),
        })?;

    // Create JWT payload from claims
    let payload_json = JSON::stringify(claims)
        .map_err(|e| JwtError::Encode {
            message: format!("Failed to stringify claims: {:?}", e),
        })?
        .as_string()
        .ok_or_else(|| JwtError::Encode {
            message: "Claims JSON is null".to_string(),
        })?;

    // Base64url encode header and payload
    let header_b64 = base64_url_encode(&header_json);

    // Create data to sign (header.payload)
    let signing_data = format!("{}.{}", header_b64, claims_json);
    let signing_bytes = signing_data.as_bytes();
    let signing_data_array = Uint8Array::view(signing_bytes);

    // Sign the data
    let sign_params = Object::new();
    Reflect::set(&sign_params, &"name".into(), &"RSASSA-PKCS1-v1_5".into()).map_err(|e| {
        JwtError::Encode {
            message: format!("Failed to set sign algorithm: {:?}", e),
        }
    })?;

    // Import the private key for signing
    let import_params = Object::new();
    Reflect::set(&import_params, &"name".into(), &"RSASSA-PKCS1-v1_5".into()).map_err(|e| {
        JwtError::Encode {
            message: format!("Failed to set import algorithm: {:?}", e),
        }
    })?;
    Reflect::set(&import_params, &"hash".into(), &"SHA-256".into()).map_err(|e| {
        JwtError::Encode {
            message: format!("Failed to set hash: {:?}", e),
        }
    })?;

    let key_format = "pkcs8";
    let key_usages = JsValue::from_serde(&vec!["sign"]).map_err(|e| JwtError::Encode {
        message: format!("Failed to create key usages: {}", e),
    })?;

    let import_promise = subtle
        .import_key(
            &key_format.into(),
            &key_data,
            &import_params,
            false,
            &key_usages,
        )
        .map_err(|e| JwtError::WebCryptoFailed {
            message: format!("Failed to import key: {:?}", e),
        })?;

    let crypto_key = wasm_bindgen_futures::JsFuture::from(import_promise)
        .await
        .map_err(|e: JsValue| {
            let js_string = e.as_string().unwrap_or_else(|| "Unknown error".to_string());
            JwtError::WebCryptoFailed {
                message: format!("Key import failed: {}", js_string),
            }
        })?;

    let sign_promise = subtle
        .sign(&sign_params, &crypto_key, &signing_data_array)
        .map_err(|e| JwtError::WebCryptoFailed {
            message: format!("Failed to sign: {:?}", e),
        })?;

    // Wait for signature
    let signature_future = wasm_bindgen_futures::JsFuture::from(sign_promise);
    let signature = signature_future.await.map_err(|e: JsValue| {
        let js_string = e.as_string().unwrap_or_else(|| "Unknown error".to_string());
        JwtError::WebCryptoFailed {
            message: format!("Signing failed: {}", js_string),
        }
    })?;

    // Convert signature to Uint8Array and encode as base64url
    let signature_array: Uint8Array = signature.unchecked_into();
    let signature_vec: Vec<u8> = signature_array.to_vec();
    let signature_b64 = base64_url_encode_vec(signature_vec);

    // Combine header, payload, and signature
    Ok(format!("{}.{}.{}", header_b64, payload_b64, signature_b64))
}

/// Base64url encode a string for WASM
#[cfg(target_arch = "wasm32")]
fn base64_url_encode(input: &str) -> String {
    base64_url_encode_vec(input.as_bytes().to_vec())
}

/// Base64url encode bytes for WASM
#[cfg(target_arch = "wasm32")]
fn base64_url_encode_vec(input: Vec<u8>) -> String {
    use base64::{engine::general_purpose, Engine as _};
    general_purpose::URL_SAFE_NO_PAD.encode(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_default() {
        let header = Header::default();
        assert_eq!(header.alg, "RS256");
        assert_eq!(header.typ, "JWT");
    }

    // JWT encoding tests are moved to tests/jwt_test.rs
    // to use proper test fixtures with real RSA keys
}
