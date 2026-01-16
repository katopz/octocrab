//! Concurrent data structures for cross-platform support
//!
//! This module provides thread-safe concurrent data structures that work
//! on both native platforms and WASM (Cloudflare Workers).

#[cfg(target_arch = "wasm32")]
use std::collections::HashMap;
#[cfg(target_arch = "wasm32")]
use std::hash::Hash;

#[cfg(not(target_arch = "wasm32"))]
use std::hash::Hash;
#[cfg(target_arch = "wasm32")]
use std::sync::Arc;

#[cfg(target_arch = "wasm32")]
use crate::internal::sync::{OnceLock, RwLock};

/// Concurrent map abstraction
///
/// On native platforms: Uses `DashMap` for better performance
/// On WASM platforms: Uses `RwLock<HashMap>` (simpler, works in single-threaded context)
#[cfg(not(target_arch = "wasm32"))]
pub struct ConcurrentMap<K, V>
where
    K: Eq + Hash + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    inner: dashmap::DashMap<K, V>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<K, V> ConcurrentMap<K, V>
where
    K: Eq + Hash + Send + Sync + Clone + 'static,
    V: Send + Sync + Clone + 'static,
{
    /// Creates a new empty `ConcurrentMap`
    pub fn new() -> Self {
        Self {
            inner: dashmap::DashMap::new(),
        }
    }

    /// Returns a clone of the value corresponding to the key
    pub fn get<Q>(&self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.inner.get(key).map(|r| r.value().clone())
    }

    /// Inserts a key-value pair into the map
    pub fn insert(&self, key: K, value: V) {
        self.inner.insert(key, value);
    }

    /// Removes a key from the map, returning the value at the key if it existed
    pub fn remove<Q>(&self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.inner.remove(key).map(|(_, v)| v)
    }

    /// Returns an iterator over all key-value pairs
    pub fn iter(&self) -> Vec<(K, V)> {
        self.inner
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Clears the map, removing all key-value pairs
    pub fn clear(&self) {
        self.inner.clear();
    }

    /// Returns the number of elements in the map
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the map contains no elements
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns true if the map contains a value for the specified key
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.inner.contains_key(key)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<K, V> Default for ConcurrentMap<K, V>
where
    K: Eq + Hash + Send + Sync + Clone + 'static,
    V: Send + Sync + Clone + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<K, V> Clone for ConcurrentMap<K, V>
where
    K: Eq + Hash + Send + Sync + Clone + 'static,
    V: Send + Sync + Clone + 'static,
{
    fn clone(&self) -> Self {
        let new_map = dashmap::DashMap::new();
        for r in self.inner.iter() {
            new_map.insert(r.key().clone(), r.value().clone());
        }
        Self { inner: new_map }
    }
}

/// Concurrent map for WASM platforms
///
/// Uses `RwLock<HashMap>` for concurrent access. While Workers is single-threaded,
/// this provides a compatible interface and ensures thread safety if the code
/// is ever used in multi-threaded WASM contexts.
#[cfg(target_arch = "wasm32")]
pub struct ConcurrentMap<K, V>
where
    K: Eq + Hash + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    inner: Arc<RwLock<HashMap<K, V>>>,
}

#[cfg(target_arch = "wasm32")]
impl<K, V> ConcurrentMap<K, V>
where
    K: Eq + Hash + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    /// Creates a new empty `ConcurrentMap`
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Returns a clone of the value corresponding to the key
    pub fn get<Q>(&self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q> + Eq + Hash,
        Q: Hash + Eq + ?Sized,
        V: Clone,
    {
        let map = self.inner.read();
        map.get(key).cloned()
    }

    /// Inserts a key-value pair into the map
    pub fn insert(&self, key: K, value: V) {
        let mut map = self.inner.write();
        map.insert(key, value);
    }

    /// Removes a key from the map, returning the value at the key if it existed
    pub fn remove<Q>(&self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let mut map = self.inner.write();
        map.remove(key)
    }

    /// Returns an iterator over all key-value pairs
    pub fn iter(&self) -> Vec<(K, V)> {
        let map = self.inner.read();
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Clears the map, removing all key-value pairs
    pub fn clear(&self) {
        let mut map = self.inner.write();
        map.clear();
    }

    /// Returns the number of elements in the map
    pub fn len(&self) -> usize {
        let map = self.inner.read();
        map.len()
    }

    /// Returns true if the map contains no elements
    pub fn is_empty(&self) -> bool {
        let map = self.inner.read();
        map.is_empty()
    }

    /// Returns true if the map contains a value for the specified key
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let map = self.inner.read();
        map.contains_key(key)
    }
}

#[cfg(target_arch = "wasm32")]
impl<K, V> Default for ConcurrentMap<K, V>
where
    K: Eq + Hash + Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_arch = "wasm32")]
impl<K, V> Clone for ConcurrentMap<K, V>
where
    K: Eq + Hash + Send + Sync + Clone + 'static,
    V: Send + Sync + Clone + 'static,
{
    fn clone(&self) -> Self {
        let map = self.inner.read();
        let new_map = map.clone();
        Self {
            inner: Arc::new(RwLock::new(new_map)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_dashmap_basic() {
        let map = ConcurrentMap::new();

        assert!(map.is_empty());
        assert_eq!(map.len(), 0);

        map.insert(1, "one");
        map.insert(2, "two");
        map.insert(3, "three");

        assert_eq!(map.len(), 3);
        assert!(!map.is_empty());
        assert_eq!(map.get(&1), Some("one"));
        assert_eq!(map.get(&2), Some("two"));
        assert_eq!(map.get(&3), Some("three"));
        assert_eq!(map.get(&4), None);

        assert!(map.contains_key(&2));
        assert!(!map.contains_key(&4));

        let removed = map.remove(&2);
        assert_eq!(removed, Some("two"));
        assert_eq!(map.get(&2), None);
        assert_eq!(map.len(), 2);
    }

    #[test]
    #[cfg(target_arch = "wasm32")]
    fn test_rwlock_map_basic() {
        let map = ConcurrentMap::new();

        assert!(map.is_empty());
        assert_eq!(map.len(), 0);

        map.insert(1, "one");
        map.insert(2, "two");
        map.insert(3, "three");

        assert_eq!(map.len(), 3);
        assert!(!map.is_empty());
        assert_eq!(map.get(&1), Some("one"));
        assert_eq!(map.get(&2), Some("two"));
        assert_eq!(map.get(&3), Some("three"));
        assert_eq!(map.get(&4), None);

        assert!(map.contains_key(&2));
        assert!(!map.contains_key(&4));

        let removed = map.remove(&2);
        assert_eq!(removed, Some("two"));
        assert_eq!(map.get(&2), None);
        assert_eq!(map.len(), 2);
    }

    #[test]
    #[cfg(target_arch = "wasm32")]
    fn test_rwlock_map_iter() {
        let map = ConcurrentMap::new();

        map.insert(1, "one");
        map.insert(2, "two");
        map.insert(3, "three");

        let mut items = map.iter();
        items.sort_by_key(|(k, _)| *k);

        assert_eq!(items.len(), 3);
        assert_eq!(items[0], (1, "one"));
        assert_eq!(items[1], (2, "two"));
        assert_eq!(items[2], (3, "three"));
    }

    #[test]
    #[cfg(target_arch = "wasm32")]
    fn test_rwlock_map_clear() {
        let map = ConcurrentMap::new();

        map.insert(1, "one");
        map.insert(2, "two");

        assert_eq!(map.len(), 2);

        map.clear();

        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
        assert_eq!(map.get(&1), None);
    }

    #[test]
    #[cfg(target_arch = "wasm32")]
    fn test_rwlock_map_clone() {
        let map1 = ConcurrentMap::new();

        map1.insert(1, "one");
        map1.insert(2, "two");

        let map2 = map1.clone();

        assert_eq!(map1.len(), map2.len());
        assert_eq!(map1.get(&1), map2.get(&1));
        assert_eq!(map1.get(&2), map2.get(&2));

        // Modify original
        map1.insert(3, "three");

        assert_eq!(map1.get(&3), Some("three"));
        assert_eq!(map2.get(&3), None);
    }
}
