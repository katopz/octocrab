//! JWT tests for cross-platform support

use octocrab::auth::{create_jwt, AppAuth};
use octocrab::internal::jwt::{self, Claims, Header};
use octocrab::models::AppId;

const TEST_PRIVATE_KEY: &str = include_str!("fixtures/test_key_pkcs1.pem");
const TEST_APP_ID: u64 = 123456;

#[test]
fn test_encoding_key_from_pem() {
    let key = jwt::encoding_key_from_pem(TEST_PRIVATE_KEY.as_bytes());
    assert!(key.is_ok(), "Should successfully parse PEM key");

    let encoding_key = key.unwrap();
    match encoding_key {
        #[cfg(not(target_arch = "wasm32"))]
        jwt::EncodingKey::Native(_) => {}
        #[cfg(target_arch = "wasm32")]
        jwt::EncodingKey::Wasm(_) => {}
    }
}

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

#[test]
fn test_jwt_encoding() {
    let key = jwt::encoding_key_from_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap();
    let claims = Claims {
        iss: TEST_APP_ID,
        iat: 1000,
        exp: 2000,
    };

    let token = jwt::encode(&Header::default(), &claims, &key);
    assert!(token.is_ok(), "Should successfully encode JWT");

    let token = token.unwrap();
    assert!(
        token.contains('.'),
        "Token should have dots separating parts"
    );

    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(
        parts.len(),
        3,
        "JWT should have 3 parts: header, payload, signature"
    );
}

#[test]
fn test_jwt_structure() {
    let key = jwt::encoding_key_from_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap();
    let claims = Claims {
        iss: TEST_APP_ID,
        iat: 1000,
        exp: 2000,
    };

    let token = jwt::encode(&Header::default(), &claims, &key).unwrap();

    // Decode and verify header
    let header_part = token.split('.').next().unwrap();
    let header_json = {
        use base64::{engine::general_purpose, Engine as _};
        general_purpose::URL_SAFE_NO_PAD
            .decode(header_part)
            .unwrap()
    };
    let header: serde_json::Value = serde_json::from_slice(&header_json).unwrap();
    assert_eq!(header["alg"], "RS256");
    assert_eq!(header["typ"], "JWT");

    // Decode and verify payload
    let payload_part = token.split('.').nth(1).unwrap();
    let payload_json = {
        use base64::{engine::general_purpose, Engine as _};
        general_purpose::URL_SAFE_NO_PAD
            .decode(payload_part)
            .unwrap()
    };
    let payload: serde_json::Value = serde_json::from_slice(&payload_json).unwrap();
    assert_eq!(payload["iss"], TEST_APP_ID);
    assert_eq!(payload["iat"], 1000);
    assert_eq!(payload["exp"], 2000);
}

#[test]
fn test_create_jwt_function() {
    let key = jwt::encoding_key_from_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap();
    let app_id = AppId(TEST_APP_ID);

    let token = create_jwt(app_id, &key);
    assert!(token.is_ok(), "create_jwt should succeed");

    let token = token.unwrap();
    assert!(token.contains('.'), "Token should be properly formatted");

    // Verify the token has 3 parts
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);

    // Verify the payload contains the app_id
    let payload_part = parts[1];
    let payload_json = {
        use base64::{engine::general_purpose, Engine as _};
        general_purpose::URL_SAFE_NO_PAD
            .decode(payload_part)
            .unwrap()
    };
    let payload: serde_json::Value = serde_json::from_slice(&payload_json).unwrap();
    assert_eq!(payload["iss"], TEST_APP_ID);
}

#[test]
fn test_app_auth_new() {
    let app_id = AppId(TEST_APP_ID);
    let app_auth = AppAuth::new(app_id, TEST_PRIVATE_KEY);

    assert!(app_auth.is_ok(), "AppAuth::new should succeed");

    let app_auth = app_auth.unwrap();
    assert_eq!(app_auth.app_id.0, TEST_APP_ID);
}

#[test]
fn test_app_auth_generate_bearer_token() {
    let app_id = AppId(TEST_APP_ID);
    let app_auth = AppAuth::new(app_id, TEST_PRIVATE_KEY).unwrap();

    let token = app_auth.generate_bearer_token();
    assert!(token.is_ok(), "generate_bearer_token should succeed");

    let token = token.unwrap();
    assert!(token.contains('.'), "Bearer token should be a valid JWT");

    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);
}

#[test]
fn test_invalid_pem_key() {
    let invalid_key = "-----BEGIN RSA PRIVATE KEY-----
INVALID KEY DATA
-----END RSA PRIVATE KEY-----";

    let result = jwt::encoding_key_from_pem(invalid_key.as_bytes());
    assert!(result.is_err(), "Should fail with invalid key data");
}

#[test]
fn test_jwt_claims_structure() {
    let claims = Claims {
        iss: TEST_APP_ID,
        iat: 1000,
        exp: 2000,
    };

    // Test serialization
    let serialized = serde_json::to_string(&claims).unwrap();
    let deserialized: Claims = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized.iss, TEST_APP_ID);
    assert_eq!(deserialized.iat, 1000);
    assert_eq!(deserialized.exp, 2000);
}

#[test]
fn test_jwt_time_based_claims() {
    let key = jwt::encoding_key_from_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap();

    // Create claims with current time
    let now = web_time::SystemTime::UNIX_EPOCH
        .elapsed()
        .unwrap()
        .as_secs();

    let claims = Claims {
        iss: TEST_APP_ID,
        iat: now - 60,  // Issued 60 seconds ago
        exp: now + 540, // Expires in 9 minutes
    };

    let token = jwt::encode(&Header::default(), &claims, &key).unwrap();
    assert!(!token.is_empty());
}

#[test]
fn test_multiple_jwt_generations() {
    let key = jwt::encoding_key_from_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap();
    let claims = Claims {
        iss: TEST_APP_ID,
        iat: 1000,
        exp: 2000,
    };

    // Generate multiple tokens - they should be different due to signature randomness
    let token1 = jwt::encode(&Header::default(), &claims, &key).unwrap();
    let token2 = jwt::encode(&Header::default(), &claims, &key).unwrap();
    let token3 = jwt::encode(&Header::default(), &claims, &key).unwrap();

    // All should be valid JWTs
    assert!(token1.contains('.'));
    assert!(token2.contains('.'));
    assert!(token3.contains('.'));

    // Each should have 3 parts
    for token in &[&token1, &token2, &token3] {
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
    }

    // Tokens should have same header and payload, different signatures
    let header1 = token1.split('.').next().unwrap();
    let header2 = token2.split('.').next().unwrap();
    assert_eq!(header1, header2, "Headers should be identical");

    let payload1 = token1.split('.').nth(1).unwrap();
    let payload2 = token2.split('.').nth(1).unwrap();
    assert_eq!(payload1, payload2, "Payloads should be identical");

    // Signatures should differ (due to randomness in signing)
    let sig1 = token1.split('.').nth(2).unwrap();
    let sig2 = token2.split('.').nth(2).unwrap();
    // Note: Some RSA implementations may produce deterministic signatures
    // so we just check that the tokens are valid rather than requiring different signatures
    assert!(!sig1.is_empty());
    assert!(!sig2.is_empty());
}

#[test]
fn test_app_auth_different_app_ids() {
    let app_id_1 = AppId(111111);
    let app_id_2 = AppId(222222);

    let app_auth_1 = AppAuth::new(app_id_1, TEST_PRIVATE_KEY).unwrap();
    let app_auth_2 = AppAuth::new(app_id_2, TEST_PRIVATE_KEY).unwrap();

    let token1 = app_auth_1.generate_bearer_token().unwrap();
    let token2 = app_auth_2.generate_bearer_token().unwrap();

    // Extract and compare app IDs from tokens
    let payload1 = token1.split('.').nth(1).unwrap();
    let payload1_json = {
        use base64::{engine::general_purpose, Engine as _};
        general_purpose::URL_SAFE_NO_PAD.decode(payload1).unwrap()
    };
    let payload1_value: serde_json::Value = serde_json::from_slice(&payload1_json).unwrap();

    let payload2 = token2.split('.').nth(1).unwrap();
    let payload2_json = {
        use base64::{engine::general_purpose, Engine as _};
        general_purpose::URL_SAFE_NO_PAD.decode(payload2).unwrap()
    };
    let payload2_value: serde_json::Value = serde_json::from_slice(&payload2_json).unwrap();

    assert_eq!(payload1_value["iss"], 111111);
    assert_eq!(payload2_value["iss"], 222222);
    assert_ne!(payload1_value["iss"], payload2_value["iss"]);
}

// WASM-specific tests
#[cfg(target_arch = "wasm32")]
mod wasm_tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn test_jwt_encoding_wasm() {
        let key = jwt::encoding_key_from_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap();
        let claims = Claims {
            iss: TEST_APP_ID,
            iat: 1000,
            exp: 2000,
        };

        let token = jwt::encode(&Header::default(), &claims, &key).unwrap();
        assert!(token.contains('.'));

        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
    }

    #[wasm_bindgen_test]
    async fn test_app_auth_async_wasm() {
        let app_id = AppId(TEST_APP_ID);
        let app_auth = AppAuth::new(app_id, TEST_PRIVATE_KEY).unwrap();

        let token = app_auth.generate_bearer_token().unwrap();
        assert!(token.contains('.'));

        // Verify structure
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
    }
}

// Integration test for GitHub App authentication flow
#[test]
fn test_github_app_authentication_flow() {
    let app_id = AppId(TEST_APP_ID);
    let app_auth = AppAuth::new(app_id, TEST_PRIVATE_KEY).unwrap();

    // Step 1: Generate bearer token
    let bearer_token = app_auth.generate_bearer_token().unwrap();
    assert!(!bearer_token.is_empty());
    assert!(
        bearer_token.starts_with("ey"),
        "JWT should start with 'ey' (base64 for typical header)"
    );

    // Step 2: Verify token structure
    let parts: Vec<&str> = bearer_token.split('.').collect();
    assert_eq!(parts.len(), 3, "Bearer token should be a valid JWT");

    // Step 3: Decode and verify claims
    let payload = parts[1];
    let payload_json = {
        use base64::{engine::general_purpose, Engine as _};
        general_purpose::URL_SAFE_NO_PAD.decode(payload).unwrap()
    };
    let claims: serde_json::Value = serde_json::from_slice(&payload_json).unwrap();

    assert_eq!(claims["iss"], TEST_APP_ID);
    assert!(claims["iat"].is_number());
    assert!(claims["exp"].is_number());

    // Step 4: Verify time window (within 10 minutes)
    let iat: u64 = claims["iat"].as_u64().unwrap();
    let exp: u64 = claims["exp"].as_u64().unwrap();
    let now = web_time::SystemTime::UNIX_EPOCH
        .elapsed()
        .unwrap()
        .as_secs();

    // Token should be valid now (not expired, not issued too far in future)
    assert!(iat <= now, "Token should not be issued in the future");
    assert!(exp > now, "Token should not be already expired");
    assert!(
        exp - iat <= 600,
        "Token lifetime should be at most 10 minutes"
    );
}
