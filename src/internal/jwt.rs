//! JWT abstraction using jwt-compact for cross-platform support
//!
//! This module provides a unified interface for JWT encoding/decoding
//! using the jwt-compact library, which works on both native and WASM platforms.

use jwt_compact::{alg::Rsa, prelude::*};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::traits::PublicKeyParts;
use snafu::Snafu;

// RS256 algorithm (constant, works on all platforms)
const ALG: Rsa = Rsa::rs256();

/// JWT encoding key abstraction
#[derive(Clone)]
pub struct EncodingKey {
    inner: jwt_compact::alg::RsaPrivateKey,
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
    pub iat: i64, // Issued at (Unix timestamp)
    pub exp: i64, // Expiration (Unix timestamp, max 10 minutes)
}

// Re-export jwt-compact types with clearer names
pub use jwt_compact::TimeOptions;

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

    #[snafu(display("Key conversion failed: {message}"))]
    KeyConversion { message: String },
}

/// Result type for JWT operations
pub type Result<T> = std::result::Result<T, JwtError>;

/// Create encoding key from PEM format
pub fn encoding_key_from_pem(pem: &[u8]) -> Result<EncodingKey> {
    let pem_str = std::str::from_utf8(pem).map_err(|_| JwtError::InvalidPem)?;

    // Parse PEM and extract DER-encoded private key
    let private_key =
        rsa::RsaPrivateKey::from_pkcs1_pem(pem_str).map_err(|_e| JwtError::InvalidKey)?;

    // Ensure key meets minimum size requirement (2048 bits per RFC 7518)
    if private_key.size() < 256 {
        // 256 bytes = 2048 bits
        return Err(JwtError::InvalidKey);
    }

    // Use the private key directly (jwt-compact's RsaPrivateKey is compatible)
    Ok(EncodingKey { inner: private_key })
}

/// Encode JWT with claims (works on all platforms)
pub fn encode(_header: &Header, claims: &Claims, key: &EncodingKey) -> Result<String> {
    // Note: jwt-compact automatically sets the alg field in the header,
    // so we don't need to use the provided header's alg field directly.
    // The header parameter is kept for API compatibility.

    let header = jwt_compact::Header::empty();

    // Convert i64 timestamp to DateTime for jwt-compact
    let iat_datetime = chrono::DateTime::from_timestamp(claims.iat, 0).ok_or(JwtError::Time)?;

    // Create custom TimeOptions that uses our iat timestamp as "current time"
    // This ensures the token is issued at exactly the timestamp we specify
    let time_options = jwt_compact::TimeOptions::new(chrono::Duration::seconds(0), || iat_datetime);

    // Set duration from iat to exp
    let duration = chrono::Duration::seconds(claims.exp - claims.iat);

    // Create jwt-compact Claims using builder pattern
    let jwt_claims = jwt_compact::Claims::new(claims.clone())
        .set_duration_and_issuance(&time_options, duration)
        .set_not_before(iat_datetime);

    ALG.token(&header, &jwt_claims, &key.inner)
        .map_err(|e: jwt_compact::CreationError| JwtError::Encode {
            message: e.to_string(),
        })
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

    #[test]
    fn test_header_new() {
        let header = Header::new("RS256");
        assert_eq!(header.alg, "RS256");
        assert_eq!(header.typ, "JWT");
    }
}
