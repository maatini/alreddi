use std::time::Duration;

use rand::Rng;
use rand::SeedableRng;
use tokio::time::sleep;
use tracing::info;

use super::cache::{PosArticle, PosCache};

/// Simulated REDDI (Solace) event consumer.
///
/// In production, this connects to the Solace message broker and consumes
/// `edeka/reddi/article/updated` events. For Phase 2, we generate synthetic
/// events on a configurable interval to populate and refresh the cache.
pub struct IngestionWorker {
    cache: PosCache,
    interval: Duration,
    seed_articles: Vec<PosArticle>,
}

impl IngestionWorker {
    /// Create a new ingestion worker with a predefined set of articles.
    pub fn new(cache: PosCache, interval_ms: u64) -> Self {
        let seed_articles = vec![
            PosArticle {
                ean: "4012345100000".into(),
                name: "EDEKA Bio Frischmilch 3,5% 1L".into(),
                price_amount: 1.29,
                price_currency: "EUR".into(),
                brand: "EDEKA Bio".into(),
                category: "Molkereiprodukte".into(),
                deposit_amount: None,
                age_restriction: None,
            },
            PosArticle {
                ean: "4012345100001".into(),
                name: "GUT&GÜNSTIG Toastbrot 500g".into(),
                price_amount: 0.89,
                price_currency: "EUR".into(),
                brand: "GUT&GÜNSTIG".into(),
                category: "Brot & Backwaren".into(),
                deposit_amount: None,
                age_restriction: None,
            },
            PosArticle {
                ean: "4012345100002".into(),
                name: "Coca-Cola 1,5L PET".into(),
                price_amount: 1.49,
                price_currency: "EUR".into(),
                brand: "Coca-Cola".into(),
                category: "Erfrischungsgetränke".into(),
                deposit_amount: Some(0.25),
                age_restriction: None,
            },
            PosArticle {
                ean: "4012345100003".into(),
                name: "EDEKA Lachsfilet tiefgefroren 200g".into(),
                price_amount: 4.99,
                price_currency: "EUR".into(),
                brand: "EDEKA".into(),
                category: "Tiefkühlkost".into(),
                deposit_amount: None,
                age_restriction: None,
            },
            PosArticle {
                ean: "4012345100004".into(),
                name: "EDEKA Äpfel Jonagold 1kg".into(),
                price_amount: 2.49,
                price_currency: "EUR".into(),
                brand: "EDEKA".into(),
                category: "Obst & Gemüse".into(),
                deposit_amount: None,
                age_restriction: None,
            },
            PosArticle {
                ean: "4012345100005".into(),
                name: "Becks Pilsner 6x0,33L".into(),
                price_amount: 4.49,
                price_currency: "EUR".into(),
                brand: "Beck's".into(),
                category: "Bier".into(),
                deposit_amount: Some(0.48),
                age_restriction: Some(16),
            },
            PosArticle {
                ean: "4012345100006".into(),
                name: "EDEKA Bio Eier 10er Bodenhaltung".into(),
                price_amount: 2.79,
                price_currency: "EUR".into(),
                brand: "EDEKA Bio".into(),
                category: "Eier".into(),
                deposit_amount: None,
                age_restriction: None,
            },
            PosArticle {
                ean: "4012345100007".into(),
                name: "Barilla Spaghetti No.5 500g".into(),
                price_amount: 1.19,
                price_currency: "EUR".into(),
                brand: "Barilla".into(),
                category: "Nudeln".into(),
                deposit_amount: None,
                age_restriction: None,
            },
            PosArticle {
                ean: "4012345100008".into(),
                name: "EDEKA Bio Joghurt Natur 500g".into(),
                price_amount: 1.09,
                price_currency: "EUR".into(),
                brand: "EDEKA Bio".into(),
                category: "Molkereiprodukte".into(),
                deposit_amount: None,
                age_restriction: None,
            },
            PosArticle {
                ean: "4012345100009".into(),
                name: "Nivea Creme 150ml".into(),
                price_amount: 2.29,
                price_currency: "EUR".into(),
                brand: "Nivea".into(),
                category: "Körperpflege".into(),
                deposit_amount: None,
                age_restriction: None,
            },
            // High-volume POS article — used for load tests
            PosArticle {
                ean: "4012345999999".into(),
                name: "EDEKA Pfandbon 0,15€".into(),
                price_amount: 0.15,
                price_currency: "EUR".into(),
                brand: "EDEKA".into(),
                category: "Pfand".into(),
                deposit_amount: None,
                age_restriction: None,
            },
            // Tobacco article (age restriction 18)
            PosArticle {
                ean: "4012345100010".into(),
                name: "Marlboro Gold 20er".into(),
                price_amount: 8.20,
                price_currency: "EUR".into(),
                brand: "Marlboro".into(),
                category: "Tabakwaren".into(),
                deposit_amount: None,
                age_restriction: Some(18),
            },
        ];

        Self {
            cache,
            interval: Duration::from_millis(interval_ms),
            seed_articles,
        }
    }

    /// Start the ingestion loop as a background tokio task.
    ///
    /// Phase 2 simulates events on a timer. Phase 3 will replace this
    /// with a real Solace consumer connected to `edeka/reddi/article/updated`.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // Initial bulk load — simulate startup catch-up
            for article in &self.seed_articles {
                info!(
                    ean = %article.ean,
                    name = %article.name,
                    price = article.price_amount,
                    event = "reddi.article.updated",
                    "initial load"
                );
                self.cache.upsert(article.clone());
            }
            info!(
                count = self.seed_articles.len(),
                "REDDI initial load complete"
            );

            // Ongoing simulated event stream
            let mut rng = rand::rngs::StdRng::from_entropy();
            loop {
                sleep(self.interval).await;

                // Pick a random article and simulate a price update
                let idx = rng.gen_range(0..self.seed_articles.len());
                let mut article = self.seed_articles[idx].clone();

                // Simulate a small price fluctuation (±5%)
                let delta = (rng.gen_range(-5.0..5.0) / 100.0) * article.price_amount;
                article.price_amount = (article.price_amount + delta * 100.0).round() / 100.0;
                article.price_amount = article.price_amount.max(0.01);

                info!(
                    ean = %article.ean,
                    price = article.price_amount,
                    event = "reddi.article.updated",
                    "event consumed"
                );

                self.cache.upsert(article);
            }
        })
    }
}

/// Start the REDDI ingestion worker and return the populated cache handle.
pub fn start_ingestion_worker(
    cache: PosCache,
    interval_ms: u64,
) -> tokio::task::JoinHandle<()> {
    let worker = IngestionWorker::new(cache, interval_ms);
    worker.spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initial_load_populates_cache() {
        let cache = PosCache::new();
        let worker = IngestionWorker::new(cache.clone(), 1000);
        let handle = worker.spawn();

        // Give the initial load time to complete
        sleep(Duration::from_millis(50)).await;

        // Should have all seed articles
        assert!(cache.len() >= 11);

        // Verify a few specific articles
        let milk = cache.get("4012345100000").unwrap();
        assert_eq!(milk.name, "EDEKA Bio Frischmilch 3,5% 1L");
        assert_eq!(milk.price_amount, 1.29);

        let cola = cache.get("4012345100002").unwrap();
        assert_eq!(cola.deposit_amount, Some(0.25));

        let tobacco = cache.get("4012345100010").unwrap();
        assert_eq!(tobacco.age_restriction, Some(18));

        handle.abort();
    }

    #[tokio::test]
    async fn missing_ean_returns_none() {
        let cache = PosCache::new();
        let worker = IngestionWorker::new(cache.clone(), 1000);
        let handle = worker.spawn();

        sleep(Duration::from_millis(50)).await;
        assert!(cache.get("0000000000000").is_none());

        handle.abort();
    }
}
