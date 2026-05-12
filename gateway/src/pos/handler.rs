use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use super::cache::PosCache;
use super::metrics::LatencyTracker;

// ---------------------------------------------------------------------------
// Handler implementations — called from main.rs with app state extraction
// ---------------------------------------------------------------------------

/// Core logic for the POS article lookup. Does NOT extract axum state
/// itself — callers pass the cache and metrics references explicitly so
/// the function stays decoupled from the app state type.
pub async fn get_article_impl(
    cache: &PosCache,
    metrics: &LatencyTracker,
    ean: &str,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let start = Instant::now();
    let result = cache.get(ean);
    let elapsed_us = start.elapsed().as_micros() as u64;
    metrics.record(elapsed_us);

    match result {
        Some(article) => {
            let body = json!({
                "ean": article.ean,
                "name": article.name,
                "price": {
                    "amount": article.price_amount,
                    "currency": article.price_currency,
                },
                "brand": article.brand,
                "category": article.category,
                "deposit": article.deposit_amount,
                "ageRestriction": article.age_restriction,
            });
            Ok(Json(body))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn get_metrics_impl(metrics: &LatencyTracker) -> impl IntoResponse {
    let snap = metrics.snapshot();
    Json(serde_json::to_value(&snap).unwrap_or_default())
}

pub async fn health_impl() -> Json<serde_json::Value> {
    Json(json!({"status": "ok", "path": "pos"}))
}

// ---------------------------------------------------------------------------
// Shared state (used in tests and for potential future Axum 0.8 migration)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct PosState {
    pub cache: PosCache,
    pub metrics: Arc<LatencyTracker>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use crate::pos::cache::PosArticle;

    fn test_state() -> PosState {
        let cache = PosCache::new();
        cache.upsert(PosArticle {
            ean: "4012345999999".into(),
            name: "Test Pfandbon".into(),
            price_amount: 0.15,
            price_currency: "EUR".into(),
            brand: "EDEKA".into(),
            category: "Pfand".into(),
            deposit_amount: None,
            age_restriction: None,
        });
        PosState {
            cache,
            metrics: Arc::new(LatencyTracker::new()),
        }
    }

    fn test_app(state: PosState) -> Router {
        let cache = state.cache.clone();
        let metrics = state.metrics.clone();

        let article_cache = cache.clone();
        let article_metrics = metrics.clone();
        let metrics_metrics = metrics.clone();

        Router::new()
            .route(
                "/article/:ean",
                get(move |axum::extract::Path(ean): axum::extract::Path<String>| {
                    let c = article_cache.clone();
                    let m = article_metrics.clone();
                    async move { get_article_impl(&c, &m, &ean).await }
                }),
            )
            .route(
                "/metrics",
                get(move || {
                    let m = metrics_metrics.clone();
                    async move { get_metrics_impl(&m).await }
                }),
            )
            .route("/health", get(|| async { health_impl().await }))
            .with_state(())
    }

    #[tokio::test]
    async fn get_article_200() {
        let app = test_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/article/4012345999999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ean"], "4012345999999");
        assert_eq!(json["name"], "Test Pfandbon");
        assert_eq!(json["price"]["amount"].as_f64().unwrap(), 0.15);
    }

    #[tokio::test]
    async fn get_article_404() {
        let app = test_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/article/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn metrics_endpoint() {
        let state = test_state();
        state.metrics.record(500);

        let app = test_app(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total_requests"], 1);
        assert_eq!(json["avg_latency_us"], 500);
    }

    #[tokio::test]
    async fn health_check() {
        let app = test_app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
