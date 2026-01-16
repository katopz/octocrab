//! Internal modules for cross-platform compatibility
//!
//! This module contains abstraction layers that enable Octocrab to work
//! across both native platforms (Linux, macOS, Windows) and WASM platforms
//! (Cloudflare Workers).

pub mod concurrent;
pub mod jwt;
pub mod sync;

// Additional modules will be added as they are implemented:
pub mod async_runtime;
pub mod http_client;
// pub mod executor;
//
// #[cfg(target_arch = "wasm32")]
// pub mod wasm_http;
