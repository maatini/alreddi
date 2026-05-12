use std::sync::Arc;

use dashmap::DashMap;
use serde::Deserialize;

use super::{GraphQlResponse, SubgraphConfig, SubgraphError};

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct PriceClient {
    http: reqwest::Client,
    config: SubgraphConfig,
    stub_store: Arc<DashMap<String, PriceData>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PriceData {
    pub matnr: String,
    pub amount: f64,
    pub currency: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
}

impl PriceClient {
    pub fn new(config: SubgraphConfig) -> Self {
        let stub_store = Arc::new(DashMap::new());
        for price in seed_prices() {
            stub_store.insert(price.matnr.clone(), price);
        }
        Self {
            http: reqwest::Client::builder()
                .timeout(config.timeout)
                .build()
                .expect("reqwest::Client::build"),
            config,
            stub_store,
        }
    }

    /// Fetch price by internal MATNR.
    pub async fn get_price(&self, matnr: &str) -> Result<PriceData, SubgraphError> {
        if self.config.stub_mode {
            return self
                .stub_store
                .get(matnr)
                .map(|r| r.clone())
                .ok_or(SubgraphError::NullData);
        }

        let query = serde_json::json!({
            "query": "query($matnr: String!) { price(matnr: $matnr) { matnr amount currency validFrom validTo } }",
            "variables": { "matnr": matnr }
        });

        let resp = self
            .http
            .post(&self.config.base_url)
            .json(&query)
            .send()
            .await?
            .json::<GraphQlResponse<PriceQueryData>>()
            .await?;

        match resp.data {
            Some(data) => Ok(data.price),
            None => Err(SubgraphError::NullData),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PriceQueryData {
    price: PriceData,
}

// ---------------------------------------------------------------------------
// Seed data
// ---------------------------------------------------------------------------

fn seed_prices() -> Vec<PriceData> {
    vec![
        PriceData {
            matnr: "000000000001000001".into(),
            amount: 1.29,
            currency: "EUR".into(),
            valid_from: Some("2025-01-01".into()),
            valid_to: None,
        },
        PriceData {
            matnr: "000000000001000002".into(),
            amount: 0.99,
            currency: "EUR".into(),
            valid_from: Some("2025-01-01".into()),
            valid_to: None,
        },
        PriceData {
            matnr: "000000000001000003".into(),
            amount: 1.49,
            currency: "EUR".into(),
            valid_from: Some("2025-01-01".into()),
            valid_to: None,
        },
        PriceData {
            matnr: "000000000001000004".into(),
            amount: 4.99,
            currency: "EUR".into(),
            valid_from: Some("2025-01-01".into()),
            valid_to: None,
        },
        PriceData {
            matnr: "000000000001000005".into(),
            amount: 2.49,
            currency: "EUR".into(),
            valid_from: Some("2025-01-01".into()),
            valid_to: None,
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_returns_seed_price() {
        let client = PriceClient::new(SubgraphConfig::default());
        let price = client.get_price("000000000001000001").await.unwrap();
        assert_eq!(price.amount, 1.29);
        assert_eq!(price.currency, "EUR");
    }

    #[tokio::test]
    async fn stub_unknown_matnr_returns_error() {
        let client = PriceClient::new(SubgraphConfig::default());
        let result = client.get_price("nonexistent").await;
        assert!(result.is_err());
    }
}
