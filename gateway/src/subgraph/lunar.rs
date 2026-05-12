use std::sync::Arc;

use dashmap::DashMap;
use serde::Deserialize;

use super::{GraphQlResponse, SubgraphConfig, SubgraphError};

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
#[allow(dead_code)]
pub struct LunarClient {
    http: reqwest::Client,
    config: SubgraphConfig,
    stub_store: Arc<DashMap<String, LunarData>>,
}

/// ERP-enriched article data from LUNAR (SAP ERP via GraphQL wrapper).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct LunarData {
    pub matnr: String,
    pub plant: String,
    pub storage_location: String,
    pub available_stock: Option<f64>,
    pub base_unit: String,
    pub sales_org: String,
    pub distribution_channel: String,
}

impl LunarClient {
    pub fn new(config: SubgraphConfig) -> Self {
        let stub_store = Arc::new(DashMap::new());
        for data in seed_lunar_data() {
            stub_store.insert(data.matnr.clone(), data);
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

    /// Fetch ERP data by internal MATNR.
    #[allow(dead_code)]
    pub async fn get_erp_data(&self, matnr: &str) -> Result<LunarData, SubgraphError> {
        if self.config.stub_mode {
            return self
                .stub_store
                .get(matnr)
                .map(|r| r.clone())
                .ok_or(SubgraphError::NullData);
        }

        let query = serde_json::json!({
            "query": "query($matnr: String!) { material(matnr: $matnr) { matnr plant storageLocation availableStock baseUnit salesOrg distributionChannel } }",
            "variables": { "matnr": matnr }
        });

        let resp = self
            .http
            .post(&self.config.base_url)
            .json(&query)
            .send()
            .await?
            .json::<GraphQlResponse<LunarQueryData>>()
            .await?;

        match resp.data {
            Some(data) => Ok(data.material),
            None => Err(SubgraphError::NullData),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct LunarQueryData {
    material: LunarData,
}

// ---------------------------------------------------------------------------
// Seed data — typical EDEKA store / warehouse configuration
// ---------------------------------------------------------------------------

fn seed_lunar_data() -> Vec<LunarData> {
    vec![
        LunarData {
            matnr: "000000000001000001".into(),
            plant: "1000".into(),
            storage_location: "0001".into(),
            available_stock: Some(1250.0),
            base_unit: "ST".into(),
            sales_org: "1000".into(),
            distribution_channel: "10".into(),
        },
        LunarData {
            matnr: "000000000001000002".into(),
            plant: "1000".into(),
            storage_location: "0001".into(),
            available_stock: Some(3800.0),
            base_unit: "ST".into(),
            sales_org: "1000".into(),
            distribution_channel: "10".into(),
        },
        LunarData {
            matnr: "000000000001000003".into(),
            plant: "1000".into(),
            storage_location: "0001".into(),
            available_stock: Some(5400.0),
            base_unit: "ST".into(),
            sales_org: "1000".into(),
            distribution_channel: "10".into(),
        },
        LunarData {
            matnr: "000000000001000004".into(),
            plant: "1000".into(),
            storage_location: "0010".into(),
            available_stock: Some(350.0),
            base_unit: "ST".into(),
            sales_org: "1000".into(),
            distribution_channel: "10".into(),
        },
        LunarData {
            matnr: "000000000001000005".into(),
            plant: "1000".into(),
            storage_location: "0005".into(),
            available_stock: Some(820.0),
            base_unit: "KG".into(),
            sales_org: "1000".into(),
            distribution_channel: "10".into(),
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
    async fn stub_returns_seed_data() {
        let client = LunarClient::new(SubgraphConfig::default());
        let data = client.get_erp_data("000000000001000001").await.unwrap();
        assert_eq!(data.plant, "1000");
        assert_eq!(data.available_stock, Some(1250.0));
    }

    #[tokio::test]
    async fn stub_unknown_matnr_returns_error() {
        let client = LunarClient::new(SubgraphConfig::default());
        let result = client.get_erp_data("nonexistent").await;
        assert!(result.is_err());
    }
}
