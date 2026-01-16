// Auto-generated test key for JWT testing
// Generated with: openssl genrsa -traditional -out test_key_pkcs1.pem 2048
// Then: openssl pkcs8 -topk8 -nocrypt -in test_key_pkcs1.pem -out test_key.pem

#[allow(dead_code)]
pub const TEST_PRIVATE_KEY: &str = include_str!("test_key.pem");

#[allow(dead_code)]
pub const TEST_APP_ID: u64 = 123456;
