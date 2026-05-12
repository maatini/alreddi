use std::sync::Arc;

use dashmap::DashMap;
use serde::Deserialize;

use super::{GraphQlResponse, SubgraphConfig, SubgraphError};

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct PimClient {
    http: reqwest::Client,
    config: SubgraphConfig,
    /// In-memory stub data for fast development without a running PIM subgraph.
    stub_store: Arc<DashMap<String, PimArticle>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PimArticle {
    pub ean: String,
    pub matnr: String,
    pub name: String,
    pub description: Option<String>,
    pub category_id: String,
    pub category_name: String,
    pub brand: String,
    pub image_url: Option<String>,
}

impl PimClient {
    pub fn new(config: SubgraphConfig) -> Self {
        let stub_store = Arc::new(DashMap::new());
        for article in seed_articles() {
            stub_store.insert(article.matnr.clone(), article);
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

    /// Fetch article data by internal MATNR. Uses stub data when
    /// `stub_mode` is enabled in config.
    pub async fn get_article(&self, matnr: &str) -> Result<PimArticle, SubgraphError> {
        if self.config.stub_mode {
            return self
                .stub_store
                .get(matnr)
                .map(|r| r.clone())
                .ok_or(SubgraphError::NullData);
        }

        let query = serde_json::json!({
            "query": "query($matnr: String!) { article(matnr: $matnr) { ean matnr name description categoryId categoryName brand imageUrl } }",
            "variables": { "matnr": matnr }
        });

        let resp = self
            .http
            .post(&self.config.base_url)
            .json(&query)
            .send()
            .await?
            .json::<GraphQlResponse<PimQueryData>>()
            .await?;

        match resp.data {
            Some(data) => Ok(data.article),
            None => Err(SubgraphError::NullData),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PimQueryData {
    article: PimArticle,
}

// ---------------------------------------------------------------------------
// Seed data — mirrors the gateway mock DataStore for Phase 3 transition.
// ---------------------------------------------------------------------------

fn seed_articles() -> Vec<PimArticle> {
    vec![
        PimArticle {
            ean: "4012345678901".into(),
            matnr: "000000000001000001".into(),
            name: "EDEKA Bio Vollmilch 3,8%".into(),
            description: Some("Frische Bio-Vollmilch, 1 Liter".into()),
            category_id: "cat-milk".into(),
            category_name: "Milchprodukte".into(),
            brand: "EDEKA Bio".into(),
            image_url: Some("https://cdn.edeka.de/images/articles/4012345678901.jpg".into()),
        },
        PimArticle {
            ean: "4012345678902".into(),
            matnr: "000000000001000002".into(),
            name: "GUT&GÜNSTIG Toastbrot".into(),
            description: Some("Toastbrot Weizen, 500g".into()),
            category_id: "cat-bread".into(),
            category_name: "Brot & Backwaren".into(),
            brand: "GUT&GÜNSTIG".into(),
            image_url: Some("https://cdn.edeka.de/images/articles/4012345678902.jpg".into()),
        },
        PimArticle {
            ean: "4012345678903".into(),
            matnr: "000000000001000003".into(),
            name: "Coca-Cola 1,5L".into(),
            description: Some("Erfrischungsgetränk, 1,5 Liter".into()),
            category_id: "cat-beverage".into(),
            category_name: "Getränke".into(),
            brand: "Coca-Cola".into(),
            image_url: Some("https://cdn.edeka.de/images/articles/4012345678903.jpg".into()),
        },
        PimArticle {
            ean: "4012345678904".into(),
            matnr: "000000000001000004".into(),
            name: "EDEKA Lachsfilet 200g".into(),
            description: Some("Tiefkühl-Lachsfilet, 200g".into()),
            category_id: "cat-fish".into(),
            category_name: "Fisch".into(),
            brand: "EDEKA".into(),
            image_url: Some("https://cdn.edeka.de/images/articles/4012345678904.jpg".into()),
        },
        PimArticle {
            ean: "4012345678905".into(),
            matnr: "000000000001000005".into(),
            name: "EDEKA Äpfel Jonagold 1kg".into(),
            description: Some("Frische Äpfel Jonagold, 1kg Beutel".into()),
            category_id: "cat-fruit".into(),
            category_name: "Obst & Gemüse".into(),
            brand: "EDEKA".into(),
            image_url: Some("https://cdn.edeka.de/images/articles/4012345678905.jpg".into()),
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
    async fn stub_returns_seed_article() {
        let client = PimClient::new(SubgraphConfig::default());
        let article = client
            .get_article("000000000001000001")
            .await
            .unwrap();
        assert_eq!(article.name, "EDEKA Bio Vollmilch 3,8%");
        assert_eq!(article.brand, "EDEKA Bio");
    }

    #[tokio::test]
    async fn stub_unknown_matnr_returns_error() {
        let client = PimClient::new(SubgraphConfig::default());
        let result = client.get_article("nonexistent").await;
        assert!(result.is_err());
    }
}
