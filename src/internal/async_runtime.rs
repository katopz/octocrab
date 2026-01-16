//! Async runtime abstraction for cross-platform support
//!
//! This module provides unified async primitives that work on both native
//! platforms (using tokio) and WASM platforms (using wasm-bindgen-futures).

use std::future::Future;
use std::time::Duration;
use web_time::{Instant, SystemTime};

/// Sleep for a specified duration
///
/// On native platforms: Uses `tokio::time::sleep`
/// On WASM platforms: Uses `wasm_timer::Delay`
#[cfg(not(target_arch = "wasm32"))]
pub async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await
}

/// Sleep for a specified duration on WASM
#[cfg(target_arch = "wasm32")]
pub async fn sleep(duration: Duration) {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().expect("no global window exists");
    let promise = window.timeout_with_callback_and_duration_and_arguments_0(
        &Closure::once(|| {}),
        duration.as_millis() as i32,
    );

    let _ = JsFuture::from(promise).await;
}

/// Execute a future with a timeout
///
/// Returns `Err` if the future does not complete within the specified duration.
///
/// On native platforms: Uses `tokio::time::timeout`
/// On WASM platforms: Uses a custom timeout implementation with AbortController
#[cfg(not(target_arch = "wasm32"))]
pub async fn timeout<F, T>(duration: Duration, future: F) -> Result<T, TimeoutError>
where
    F: Future<Output = T>,
{
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| TimeoutError)
}

/// Execute a future with a timeout on WASM
#[cfg(target_arch = "wasm32")]
pub async fn timeout<F, T>(duration: Duration, future: F) -> Result<T, TimeoutError>
where
    F: Future<Output = T> + 'static,
{
    use js_sys::{Function, Promise};
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    // Simple timeout implementation using JavaScript Promise.race
    let window = web_sys::window().expect("no global window exists");

    // Use simple polling-based timeout implementation
    // For a production implementation, consider using gloo-timers or Promise.race
    let start = Instant::now();
    let mut pinned_future = std::pin::pin!(future);

    loop {
        // Check if timeout has elapsed
        if start.elapsed() >= duration {
            return Err(TimeoutError);
        }

        // Try to poll the future
        let waker = futures::task::noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);

        match pinned_future.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(val) => return Ok(val),
            std::task::Poll::Pending => {
                // Yield control and wait a bit before polling again
                // In a real implementation, this would use proper async notification
                sleep(Duration::from_millis(10)).await;
            }
        }
    }
}

/// Timeout error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutError;

impl std::fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "operation timed out")
    }
}

impl std::error::Error for TimeoutError {}

/// Executor for spawning futures
///
/// On native platforms: Uses `tokio::spawn`
/// On WASM platforms: Uses `wasm_bindgen_futures::spawn_local`
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(future);
}

/// Spawn a future locally on WASM
#[cfg(target_arch = "wasm32")]
pub fn spawn<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    wasm_bindgen_futures::spawn_local(future);
}

/// Current time utilities
///
/// These are platform-agnostic and use `web_time` for consistent behavior
pub mod time {
    use super::*;

    /// Returns the current time
    pub fn now() -> SystemTime {
        SystemTime::now()
    }

    /// Returns the current instant
    pub fn instant_now() -> Instant {
        Instant::now()
    }

    /// Returns the duration since the UNIX epoch
    pub fn unix_timestamp() -> Result<u64, web_time::SystemTimeError> {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_sleep_native() {
        let start = Instant::now();
        sleep(Duration::from_millis(100)).await;
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(90));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg(target_arch = "wasm32")]
    async fn test_sleep_wasm() {
        let start = Instant::now();
        sleep(Duration::from_millis(100)).await;
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(90));
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_timeout_success_native() {
        let result = timeout(Duration::from_millis(100), async {
            sleep(Duration::from_millis(50)).await;
            42
        })
        .await;
        assert_eq!(result.unwrap(), 42);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg(target_arch = "wasm32")]
    async fn test_timeout_success_wasm() {
        let result = timeout(Duration::from_millis(100), async {
            sleep(Duration::from_millis(50)).await;
            42
        })
        .await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_timeout_failure_native() {
        let result = timeout(Duration::from_millis(50), async {
            sleep(Duration::from_millis(100)).await;
            42
        })
        .await;
        assert_eq!(result, Err(TimeoutError));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg(target_arch = "wasm32")]
    async fn test_timeout_failure_wasm() {
        let result = timeout(Duration::from_millis(50), async {
            sleep(Duration::from_millis(100)).await;
            42
        })
        .await;
        assert_eq!(result, Err(TimeoutError));
    }

    #[test]
    fn test_time_utils() {
        let timestamp = time::unix_timestamp();

        assert!(timestamp.is_ok());
        let ts = timestamp.unwrap();
        // Timestamp should be recent (after year 2020)
        assert!(ts > 1_577_836_800);
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_spawn_native() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        spawn(async move {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Give the spawned task time to complete
        sleep(Duration::from_millis(10)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
