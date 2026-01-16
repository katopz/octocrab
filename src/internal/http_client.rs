//! HTTP client abstraction for cross-platform support
//!
//! This module provides a unified HTTP client interface that works on both native
//! platforms (using hyper) and WASM platforms (using Fetch API).

use crate::body::OctoBody;
use hyper_util::client::legacy::connect::HttpConnector;

#[cfg(target_arch = "wasm32")]
use crate::body::BoxBody;
#[cfg(target_arch = "wasm32")]
use bytes::Bytes;
#[cfg(target_arch = "wasm32")]
use http_body_util::Full;

/// HTTP client incoming response body type for native platforms
#[cfg(not(target_arch = "wasm32"))]
pub use hyper::body::Incoming;

/// HTTP client incoming response body type for WASM
#[cfg(target_arch = "wasm32")]
pub type Incoming = BoxBody;

/// Platform-specific HTTP client service type
#[cfg(not(target_arch = "wasm32"))]
pub type HttpClient =
    hyper_util::client::legacy::Client<hyper_rustls::HttpsConnector<HttpConnector>, OctoBody>;

/// Platform-specific HTTP client service type for WASM
#[cfg(target_arch = "wasm32")]
pub type HttpClient = WasmClient;

/// Creates a new HTTP client appropriate for the current platform
#[cfg(not(target_arch = "wasm32"))]
pub fn create_client() -> Result<HttpClient, String> {
    let connector = {
        let builder = hyper_rustls::HttpsConnectorBuilder::new();
        let builder = builder
            .with_native_roots()
            .map_err(|e| format!("Failed to create TLS connector: {}", e))?;

        builder.https_or_http().enable_http1().build()
    };

    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build::<_, OctoBody>(connector);

    Ok(client)
}

/// WASM HTTP client using Fetch API
#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
pub struct WasmClient;

#[cfg(target_arch = "wasm32")]
impl WasmClient {
    pub fn new() -> Self {
        Self
    }

    async fn execute_request(
        &self,
        req: http::Request<OctoBody>,
    ) -> Result<http::Response<Incoming>, crate::Error> {
        use http_body::Body;
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;
        use wasm_bindgen_futures::JsFuture;

        // Extract request data
        let (parts, body) = req.into_parts();
        let method = parts.method.as_str();
        let url = parts.uri.to_string();

        let mut opts = web_sys::RequestInit::new();
        opts.method(method);

        // Set headers
        if let Some(headers) = web_sys::Headers::new().ok() {
            for (name, value) in parts.headers.iter() {
                if let Ok(val_str) = value.to_str() {
                    let _ = headers.set(name.as_str(), val_str);
                }
            }
            opts.headers(&headers);
        }

        // Set body - collect the OctoBody into bytes
        let body_bytes = self.collect_body(body).await?;
        if !body_bytes.is_empty() {
            if let Some(array) = js_sys::Uint8Array::view(&body_bytes) {
                opts.body(Some(&array));
            }
        }

        // Execute fetch request
        let window = web_sys::window().ok_or_else(|| crate::Error::Other {
            source: Box::from("No global window exists"),
            backtrace: snafu::Backtrace::capture(),
        })?;

        let fetch_promise = window.fetch_with_str_and_init(&url, &opts);

        let response_result =
            JsFuture::from(fetch_promise)
                .await
                .map_err(|e| crate::Error::Other {
                    source: Box::from(format!("Fetch failed: {:?}", e)),
                    backtrace: snafu::Backtrace::capture(),
                })?;

        let response = response_result
            .dyn_into::<web_sys::Response>()
            .map_err(|e| crate::Error::Other {
                source: Box::from(format!("Response conversion failed: {:?}", e)),
                backtrace: snafu::Backtrace::capture(),
            })?;

        // Read response body
        let body_promise = response.array_buffer().map_err(|e| crate::Error::Other {
            source: Box::from(format!("Failed to get array buffer: {:?}", e)),
            backtrace: snafu::Backtrace::capture(),
        })?;

        let array_buffer = JsFuture::from(body_promise)
            .await
            .map_err(|e| crate::Error::Other {
                source: Box::from(format!("Array buffer read failed: {:?}", e)),
                backtrace: snafu::Backtrace::capture(),
            })?;

        let uint8_array = js_sys::Uint8Array::new(&array_buffer);
        let mut buffer = Vec::with_capacity(uint8_array.length() as usize);
        uint8_array.copy_to(&mut buffer);

        // Build HTTP response
        let mut builder = http::Response::builder().status(response.status() as u16);

        // Copy headers
        if let Some(headers) = builder.headers_mut() {
            for (name, value) in response.headers().entries().into_iter() {
                if let Ok(entry) = value.dyn_into::<js_sys::Array>() {
                    if entry.length() == 2 {
                        let key = entry.get(0).as_string().unwrap_or_default();
                        let val = entry.get(1).as_string().unwrap_or_default();
                        if let Ok(name) = http::header::HeaderName::from_bytes(key.as_bytes()) {
                            if let Ok(value) = http::header::HeaderValue::from_bytes(val.as_bytes())
                            {
                                let _ = headers.append(name, value);
                            }
                        }
                    }
                }
            }
        }

        let body = Full::new(Bytes::from(buffer))
            .map_err(|e| crate::Error::Other {
                source: e.into(),
                backtrace: snafu::Backtrace::capture(),
            })
            .boxed();

        builder.body(body).map_err(|e| crate::Error::Other {
            source: Box::from(format!("Failed to build response: {}", e)),
            backtrace: snafu::Backtrace::capture(),
        })
    }

    /// Collect OctoBody into a Vec<u8>
    async fn collect_body(&self, mut body: OctoBody) -> Result<Vec<u8>, crate::Error> {
        use http_body::Body;
        use std::pin::Pin;
        use std::task::{Context, Poll};

        let mut buffer = Vec::new();
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        loop {
            match Pin::new(&mut body).poll_frame(&mut cx) {
                Poll::Ready(Some(Ok(frame))) => {
                    if let Some(data) = frame.data_ref() {
                        buffer.extend_from_slice(data);
                    }
                }
                Poll::Ready(None) => break,
                Poll::Ready(Some(Err(e))) => {
                    return Err(crate::Error::Other {
                        source: e.into(),
                        backtrace: snafu::Backtrace::capture(),
                    });
                }
                Poll::Pending => {
                    // For synchronous body collection, we need to await
                    // But since we're in a non-async context here, this is tricky
                    // In practice, OctoBody with buffered data should work immediately
                    return Err(crate::Error::Other {
                        source: Box::from("Body not ready - buffered data expected"),
                        backtrace: snafu::Backtrace::capture(),
                    });
                }
            }
        }

        Ok(buffer)
    }
}

#[cfg(target_arch = "wasm32")]
impl tower::Service<http::Request<OctoBody>> for WasmClient {
    type Response = http::Response<Incoming>;
    type Error = crate::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<OctoBody>) -> Self::Future {
        Box::pin(self.execute_request(req))
    }
}

/// Creates a new HTTP client appropriate for the current platform
#[cfg(target_arch = "wasm32")]
pub fn create_client() -> Result<HttpClient, String> {
    Ok(WasmClient::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_create_native_client() {
        let client = create_client();
        assert!(client.is_ok());
    }

    #[test]
    #[cfg(target_arch = "wasm32")]
    fn test_create_wasm_client() {
        let client = WasmClient::new();
        // Just verify it can be created
        let _ = client;
    }
}
