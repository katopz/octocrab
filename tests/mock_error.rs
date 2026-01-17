use serde_json::json;
use wiremock::{
    matchers::{method, path_regex},
    Mock, MockServer, ResponseTemplate,
};

// Global CryptoProvider initialization for rustls tests
#[cfg(all(feature = "rustls", not(target_arch = "wasm32")))]
pub fn ensure_crypto_provider_initialized() {
    use std::sync::OnceLock;

    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        #[cfg(feature = "rustls-ring")]
        let provider = rustls::crypto::ring::default_provider();
        #[cfg(all(feature = "rustls-aws-lc-rs", not(feature = "rustls-ring")))]
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        #[cfg(all(not(feature = "rustls-ring"), not(feature = "rustls-aws-lc-rs")))]
        compile_error!("Either rustls-ring or rustls-aws-lc-rs feature must be enabled");
        provider
            .install_default()
            .expect("Failed to install CryptoProvider");
    });
}

// Sets up a handler on the mock server which will return a 500 with the given message. This
// will be mapped internally into a GitHub json error, making it much easier to identify the cause
// of these test failures.
//
// This handler should always come after your real expectations as it will match any GET request.
pub async fn setup_error_handler(mock_server: &MockServer, message: &str) {
    Mock::given(method("GET"))
        .and(path_regex(".*"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!( {
            "documentation_url": "",
            "errors": None::<Vec<serde_json::Value>>,
            "message": message,
        })))
        .mount(mock_server)
        .await;
}
