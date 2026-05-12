use std::sync::Arc;

use dashmap::DashMap;
use sha2::{Digest, Sha256};
use tokio::time::Instant;

/// Automated Persisted Queries (APQ) cache.
///
/// Clients send a SHA-256 hash instead of the full query text.
/// Flow:
///   1. Client sends `{ extensions: { persistedQuery: { sha256Hash, version: 1 } } }`
///      with no `query` field.
///   2. If the hash is known → execute the cached query.
///   3. If unknown → return `PERSISTED_QUERY_NOT_FOUND` error.
///   4. Client re-sends with BOTH the hash AND the full query text.
///   5. Server caches and executes.
#[allow(dead_code)]
#[derive(Clone)]
pub struct ApqCache {
    store: Arc<DashMap<String, CachedQuery>>,
    max_entries: usize,
}

struct CachedQuery {
    query: String,
    inserted_at: Instant,
}

#[allow(dead_code)]
impl ApqCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            store: Arc::new(DashMap::new()),
            max_entries,
        }
    }

    /// Store a query under its SHA-256 hash.
    pub fn store(&self, query: &str) -> String {
        let hash = hex::encode(Sha256::digest(query.as_bytes()));

        // Evict oldest if at capacity.
        // Drop the iterator guard before calling remove() — otherwise
        // DashMap's RwLock-per-shard deadlocks: the RefMulti from iter()
        // holds a read guard on the shard that remove() needs to write-lock.
        if self.store.len() >= self.max_entries {
            let key_to_evict = self
                .store
                .iter()
                .min_by_key(|entry| entry.inserted_at)
                .map(|entry| entry.key().clone());

            if let Some(key) = key_to_evict {
                self.store.remove(&key);
            }
        }

        self.store.insert(
            hash.clone(),
            CachedQuery {
                query: query.to_owned(),
                inserted_at: Instant::now(),
            },
        );

        hash
    }

    /// Look up a query by its SHA-256 hash.
    pub fn lookup(&self, hash: &str) -> Option<String> {
        self.store.get(hash).map(|entry| entry.query.clone())
    }

    /// Look up and re-register (for APQ v1 flow: second request with hash + query).
    pub fn lookup_and_store(&self, hash: &str, query: &str) -> Option<String> {
        // Verify the hash matches
        let expected = hex::encode(Sha256::digest(query.as_bytes()));
        if !constant_time_eq(expected.as_bytes(), hash.as_bytes()) {
            return None;
        }

        self.store(query);
        self.lookup(hash)
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.store.len()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}

/// Constant-time byte comparison to prevent timing side-channels on hash matching.
#[allow(dead_code)]
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[allow(dead_code)]
#[derive(serde::Serialize)]
pub struct ApqErrorResponse {
    pub errors: Vec<ApqError>,
}

#[derive(serde::Serialize)]
pub struct ApqError {
    pub message: String,
    pub extensions: ApqErrorExtensions,
}

#[derive(serde::Serialize)]
pub struct ApqErrorExtensions {
    pub code: String,
}

#[allow(dead_code)]
impl ApqErrorResponse {
    pub fn not_found() -> Self {
        Self {
            errors: vec![ApqError {
                message: "PersistedQueryNotFound".into(),
                extensions: ApqErrorExtensions {
                    code: "PERSISTED_QUERY_NOT_FOUND".into(),
                },
            }],
        }
    }

    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_and_lookup() {
        let cache = ApqCache::new(10);
        let hash = cache.store("{ article(ean: \"42\") { ean } }");
        assert_eq!(hash.len(), 64); // SHA-256 hex
        assert!(cache.lookup(&hash).is_some());
        assert!(cache.lookup("deadbeef").is_none());
    }

    #[test]
    fn same_query_produces_same_hash() {
        let cache = ApqCache::new(10);
        let h1 = cache.store("query { test }");
        let h2 = cache.store("query { test }");
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_mismatch_rejected() {
        let cache = ApqCache::new(10);
        let hash = hex::encode(Sha256::digest(b"query { valid }"));
        assert!(cache.lookup_and_store(&hash, "query { different }").is_none());
    }

    #[test]
    fn eviction_on_full_cache() {
        let cache = ApqCache::new(3);
        cache.store("query { a }");
        cache.store("query { b }");
        cache.store("query { c }");
        assert_eq!(cache.len(), 3);

        cache.store("query { d }");
        assert_eq!(cache.len(), 3); // oldest evicted
    }

    #[test]
    fn constant_time_eq_safety() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
