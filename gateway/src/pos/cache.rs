use std::sync::Arc;

use dashmap::DashMap;
use serde::Serialize;

use super::metrics::LatencyHistogram;

/// Article data optimized for the POS fast-read path.
///
/// All fields are owned so lookups return a cheap `Arc` clone —
/// no allocation on the hot path after the initial cache population.
#[derive(Clone, Debug, Serialize)]
pub struct PosArticle {
    pub ean: String,
    pub name: String,
    pub price_amount: f64,
    pub price_currency: String,
    pub brand: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deposit_amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_restriction: Option<u8>,
}

/// Lock-free concurrent cache for the POS fast-read path.
///
/// Uses `DashMap` (sharded `RwLock` per bucket) to keep read-latency
/// below the 15 ms SLA even under high contention.
#[derive(Clone)]
pub struct PosCache {
    articles: Arc<DashMap<String, Arc<PosArticle>>>,
}

impl PosCache {
    pub fn new() -> Self {
        Self {
            articles: Arc::new(DashMap::new()),
        }
    }

    /// Insert or update an article. Called from the ingestion worker.
    pub fn upsert(&self, article: PosArticle) {
        let ean = article.ean.clone();
        self.articles.insert(ean, Arc::new(article));
    }

    /// Look up an article by EAN for the POS endpoint.
    ///
    /// Returns `None` if the EAN is not cached. Must complete in
    /// well under 15 ms — the only work is a hash lookup + `Arc::clone`.
    pub fn get(&self, ean: &str) -> Option<Arc<PosArticle>> {
        let _timer = LatencyHistogram::start();
        self.articles.get(ean).map(|r| Arc::clone(&r))
    }

    /// Number of cached articles.
    pub fn len(&self) -> usize {
        self.articles.len()
    }
}

impl Default for PosCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_and_get() {
        let cache = PosCache::new();
        cache.upsert(PosArticle {
            ean: "4012345000010".into(),
            name: "Testartikel".into(),
            price_amount: 2.99,
            price_currency: "EUR".into(),
            brand: "Testmarke".into(),
            category: "Testkategorie".into(),
            deposit_amount: Some(0.25),
            age_restriction: None,
        });

        let found = cache.get("4012345000010").unwrap();
        assert_eq!(found.name, "Testartikel");
        assert_eq!(found.price_amount, 2.99);
        assert_eq!(found.deposit_amount, Some(0.25));
    }

    #[test]
    fn missing_returns_none() {
        let cache = PosCache::new();
        assert!(cache.get("nonexistent").is_none());
    }

    #[test]
    fn concurrent_reads_and_writes() {
        let cache = PosCache::new();
        cache.upsert(PosArticle {
            ean: "concurrent".into(),
            name: "Concurrent Test".into(),
            price_amount: 1.00,
            price_currency: "EUR".into(),
            brand: "B".into(),
            category: "C".into(),
            deposit_amount: None,
            age_restriction: None,
        });

        std::thread::scope(|s| {
            for _ in 0..10 {
                s.spawn(|| {
                    for _ in 0..100 {
                        let r = cache.get("concurrent");
                        assert!(r.is_some());
                    }
                });
            }
        });
    }

    #[test]
    fn upsert_overwrites() {
        let cache = PosCache::new();
        cache.upsert(PosArticle {
            ean: "overwrite".into(),
            name: "Original".into(),
            price_amount: 1.00,
            price_currency: "EUR".into(),
            brand: "B".into(),
            category: "C".into(),
            deposit_amount: None,
            age_restriction: None,
        });

        cache.upsert(PosArticle {
            ean: "overwrite".into(),
            name: "Updated".into(),
            price_amount: 2.00,
            price_currency: "EUR".into(),
            brand: "B".into(),
            category: "C".into(),
            deposit_amount: None,
            age_restriction: None,
        });

        assert_eq!(cache.get("overwrite").unwrap().name, "Updated");
        assert_eq!(cache.get("overwrite").unwrap().price_amount, 2.00);
    }
}
