//! Synchronization primitives abstraction for cross-platform support
//!
//! This module provides a unified interface for synchronization primitives
//! that work on both native platforms and WASM (Cloudflare Workers).

/// Mutex abstraction - uses parking_lot on both platforms
pub use parking_lot::Mutex;

/// RwLock abstraction - uses parking_lot on both platforms
pub use parking_lot::RwLock;

/// OnceLock abstraction for one-time initialization
///
/// On native platforms, this uses `std::sync::OnceLock`.
/// On WASM platforms, this uses `parking_lot::OnceLock`.
#[cfg(not(target_arch = "wasm32"))]
pub use std::sync::OnceLock;

#[cfg(target_arch = "wasm32")]
pub use parking_lot::OnceLock;

/// Type alias for OnceLock for convenience
pub type Once<T> = OnceLock<T>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mutex() {
        let mutex = Mutex::new(42);
        {
            let mut data = mutex.lock();
            *data = 100;
        }
        assert_eq!(*mutex.lock(), 100);
    }

    #[test]
    fn test_rwlock() {
        let rwlock = RwLock::new(42);

        // Test read lock
        {
            let data = rwlock.read();
            assert_eq!(*data, 42);
        }

        // Test write lock
        {
            let mut data = rwlock.write();
            *data = 100;
        }

        assert_eq!(*rwlock.read(), 100);
    }

    #[test]
    fn test_once_lock() {
        let once = OnceLock::new();

        assert!(once.get().is_none());

        once.set(42).unwrap();
        assert_eq!(*once.get().unwrap(), 42);

        // Setting again should fail
        assert!(once.set(100).is_err());
        assert_eq!(*once.get().unwrap(), 42);
    }

    #[test]
    fn test_once_lock_get_or_init() {
        let once = OnceLock::new();
        let value = once.get_or_init(|| 42);

        assert_eq!(*value, 42);

        // Should not reinitialize
        let value2 = once.get_or_init(|| 100);
        assert_eq!(*value2, 42);
    }
}
