use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::broadcast;
use tokio::time::timeout;

#[derive(Error, Debug, Clone)]
pub enum CoalescingError {
    #[error("request coalescing timed out after {0:?}")]
    Timeout(Duration),
    #[error("internal coalescing error: {0}")]
    Internal(String),
}

type ResultSender = broadcast::Sender<Result<Value, CoalescingError>>;

struct InFlight {
    tx: ResultSender,
}

/// Single-flight request coalescer.
///
/// When multiple identical GraphQL queries arrive concurrently, only the
/// first is forwarded to the backend. Subsequent callers subscribe to the
/// first request's result and are woken when it completes.
///
/// Identity is determined by normalizing the query string and hashing it
/// together with operation name and variable values.
#[derive(Clone)]
pub struct Coalescer {
    in_flight: Arc<DashMap<u64, InFlight>>,
    enabled: bool,
    timeout: Duration,
}

impl Coalescer {
    pub fn new(enabled: bool, timeout: Duration) -> Self {
        Self {
            in_flight: Arc::new(DashMap::new()),
            enabled,
            timeout,
        }
    }

    /// Build a cache key from the GraphQL request fields.
    pub fn make_key(
        query: Option<&str>,
        operation_name: Option<&str>,
        variables: Option<&Value>,
    ) -> u64 {
        let mut hasher = DefaultHasher::new();

        if let Some(q) = query {
            // Normalize: collapse all whitespace to single spaces
            let normalized: String = q
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            normalized.hash(&mut hasher);
        }

        if let Some(name) = operation_name {
            name.hash(&mut hasher);
        }

        if let Some(vars) = variables {
            // Hash the sorted JSON representation for deterministic ordering
            if let Ok(canonical) = serde_json::to_string(vars) {
                canonical.hash(&mut hasher);
            }
        }

        hasher.finish()
    }

    /// Execute a query through the coalescer.
    ///
    /// If an identical query is already in-flight, this call waits for its
    /// result. Otherwise, it executes `f` and broadcasts the result to any
    /// concurrent callers that arrive while it runs.
    pub async fn execute<F, Fut>(
        &self,
        key: u64,
        f: F,
    ) -> Result<Value, CoalescingError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Value, CoalescingError>>,
    {
        if !self.enabled {
            return f().await;
        }

        // Check for in-flight request — subscribe if exists
        if let Some(entry) = self.in_flight.get(&key) {
            let mut rx = entry.tx.subscribe();
            drop(entry);

            let recv_future = rx.recv();
            match timeout(self.timeout, recv_future).await {
                Ok(Ok(result)) => result.map_err(|e| e),
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                    // Too slow — fall back to executing ourselves
                    f().await
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    Err(CoalescingError::Internal("broadcast channel closed".into()))
                }
                Err(_elapsed) => Err(CoalescingError::Timeout(self.timeout)),
            }
        } else {
            // No in-flight request — we are the leader
            let (tx, _) = broadcast::channel(1);
            self.in_flight.insert(key, InFlight { tx: tx.clone() });

            let result = f().await;

            // Broadcast to waiters (ignore send errors — no receivers is fine)
            let _ = tx.send(result.clone());

            // Cleanup
            self.in_flight.remove(&key);

            result
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use tokio::time::sleep;

    #[tokio::test(flavor = "multi_thread")]
    async fn single_request_executes_normally() {
        let c = Coalescer::new(true, Duration::from_secs(5));
        let key = Coalescer::make_key(Some("{ test }"), None, None);
        let result = c
            .execute(key, || async {
                Ok(serde_json::json!({"data": {"test": "ok"}}))
            })
            .await
            .unwrap();

        assert_eq!(result["data"]["test"], "ok");
        assert!(c.in_flight.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_identical_queries_coalesce() {
        let c = Coalescer::new(true, Duration::from_secs(5));
        let call_count = Arc::new(AtomicU32::new(0));
        let key = Coalescer::make_key(Some("{ article(ean:\"42\") { ean } }"), None, None);

        let c1 = c.clone();
        let c2 = c.clone();
        let cnt1 = call_count.clone();
        let cnt2 = call_count.clone();

        let (r1, r2) = tokio::join!(
            tokio::spawn(async move {
                c1.execute(key, || {
                    let cnt = cnt1.clone();
                    async move {
                        cnt.fetch_add(1, Ordering::SeqCst);
                        sleep(Duration::from_millis(50)).await;
                        Ok(serde_json::json!({"data": {"article": {"ean": "42"}}}))
                    }
                })
                .await
            }),
            tokio::spawn(async move {
                // Brief delay so the first request is already in-flight
                sleep(Duration::from_millis(10)).await;
                c2.execute(key, || {
                    let cnt = cnt2.clone();
                    async move {
                        cnt.fetch_add(1, Ordering::SeqCst);
                        Ok(serde_json::json!({"should_not_execute": true}))
                    }
                })
                .await
            }),
        );

        let result1 = r1.unwrap().unwrap();
        let result2 = r2.unwrap().unwrap();

        // Both see the same result (from the leader)
        assert_eq!(result1["data"]["article"]["ean"], "42");
        assert_eq!(result2["data"]["article"]["ean"], "42");

        // Only ONE backend call was made
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        assert!(c.in_flight.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn different_keys_execute_independently() {
        let c = Coalescer::new(true, Duration::from_secs(5));
        let key1 = Coalescer::make_key(Some("{ article(ean:\"1\") { ean } }"), None, None);
        let key2 = Coalescer::make_key(Some("{ article(ean:\"2\") { ean } }"), None, None);

        let c1 = c.clone();
        let c2 = c.clone();

        let (r1, r2) = tokio::join!(
            tokio::spawn(async move {
                c1.execute(key1, || async {
                    sleep(Duration::from_millis(30)).await;
                    Ok(serde_json::json!({"data": {"article": {"ean": "1"}}}))
                })
                .await
            }),
            tokio::spawn(async move {
                c2.execute(key2, || async {
                    sleep(Duration::from_millis(10)).await;
                    Ok(serde_json::json!({"data": {"article": {"ean": "2"}}}))
                })
                .await
            }),
        );

        assert_eq!(r1.unwrap().unwrap()["data"]["article"]["ean"], "1");
        assert_eq!(r2.unwrap().unwrap()["data"]["article"]["ean"], "2");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disabled_coalescing_bypasses() {
        let c = Coalescer::new(false, Duration::from_secs(5));
        let call_count = Arc::new(AtomicU32::new(0));
        let key = Coalescer::make_key(Some("{ test }"), None, None);

        let cnt = call_count.clone();
        let result = c
            .execute(key, || {
                let cnt = cnt.clone();
                async move {
                    cnt.fetch_add(1, Ordering::SeqCst);
                    Ok(serde_json::json!({"ok": true}))
                }
            })
            .await
            .unwrap();

        assert_eq!(result["ok"], true);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        assert!(c.in_flight.is_empty());
    }

    #[test]
    fn make_key_normalizes_whitespace() {
        let key1 = Coalescer::make_key(
            Some("query { article(ean:\"42\") { ean name } }"),
            None,
            None,
        );
        let key2 = Coalescer::make_key(
            Some("query   {   article(ean:\"42\") { ean   name }   }"),
            None,
            None,
        );
        assert_eq!(key1, key2);
    }

    #[test]
    fn make_key_distinguishes_variables() {
        let key1 = Coalescer::make_key(
            Some("{ article(ean: $ean) { name } }"),
            None,
            Some(&serde_json::json!({"ean": "42"})),
        );
        let key2 = Coalescer::make_key(
            Some("{ article(ean: $ean) { name } }"),
            None,
            Some(&serde_json::json!({"ean": "99"})),
        );
        assert_ne!(key1, key2);
    }
}
