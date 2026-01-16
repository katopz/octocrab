use crate::internal::concurrent::ConcurrentMap;

use super::{CacheKey, CacheStorage, CacheWriter, CachedResponse};
use http::{HeaderMap, Uri};

pub struct InMemoryCache {
    keys: ConcurrentMap<Uri, CacheKey>,
    responses: ConcurrentMap<Uri, CachedResponse>,
}

impl InMemoryCache {
    pub fn new() -> Self {
        Self {
            keys: ConcurrentMap::new(),
            responses: ConcurrentMap::new(),
        }
    }
}

impl Default for InMemoryCache {
    fn default() -> Self {
        Self::new()
    }
}

struct InMemoryWriter {
    keys: ConcurrentMap<Uri, CacheKey>,
    responses: ConcurrentMap<Uri, CachedResponse>,
    uri: Uri,
    key: CacheKey,
    response: CachedResponse,
}

impl CacheStorage for InMemoryCache {
    fn try_hit(&self, uri: &Uri) -> Option<CacheKey> {
        self.keys.get(uri)
    }

    fn load(&self, uri: &Uri) -> Option<CachedResponse> {
        self.responses.get(uri)
    }

    fn writer(&self, uri: &Uri, key: CacheKey, headers: HeaderMap) -> Box<dyn CacheWriter> {
        Box::new(InMemoryWriter {
            keys: self.keys.clone(),
            responses: self.responses.clone(),
            uri: uri.clone(),
            key,
            response: CachedResponse {
                body: Vec::new(),
                headers,
            },
        })
    }
}

impl CacheWriter for InMemoryWriter {
    fn write_body(&mut self, data: &[u8]) {
        self.response.body.extend_from_slice(data);
    }
}

impl Drop for InMemoryWriter {
    fn drop(&mut self) {
        // The whole response was received, hence the writer is dropped. We need
        // to add the response body to the cache.
        let uri = self.uri.clone();
        let key = self.key.clone();
        let response = std::mem::take(&mut self.response);

        self.keys.insert(uri.clone(), key);
        self.responses.insert(uri, response);
    }
}
