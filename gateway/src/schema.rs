use async_graphql::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Mock data store (Phase 1 — replaced by subgraph backends in Phase 3)
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct DataStore {
    articles: Arc<RwLock<HashMap<String, ArticleRecord>>>,
}

#[derive(Clone, Debug)]
struct ArticleRecord {
    ean: String,
    name: String,
    description: Option<String>,
    category_id: String,
    category_name: String,
    brand: String,
    image_url: Option<String>,
    price_amount: f64,
    price_currency: String,
}

impl DataStore {
    pub fn with_seed_data() -> Self {
        let mut map = HashMap::new();
        let records = vec![
            ArticleRecord {
                ean: "4012345678901".into(),
                name: "EDEKA Bio Vollmilch 3,8%".into(),
                description: Some("Frische Bio-Vollmilch, 1 Liter".into()),
                category_id: "cat-milk".into(),
                category_name: "Milchprodukte".into(),
                brand: "EDEKA Bio".into(),
                image_url: Some("https://cdn.edeka.de/images/articles/4012345678901.jpg".into()),
                price_amount: 1.29,
                price_currency: "EUR".into(),
            },
            ArticleRecord {
                ean: "4012345678902".into(),
                name: "GUT&GÜNSTIG Toastbrot".into(),
                description: Some("Toastbrot Weizen, 500g".into()),
                category_id: "cat-bread".into(),
                category_name: "Brot & Backwaren".into(),
                brand: "GUT&GÜNSTIG".into(),
                image_url: Some("https://cdn.edeka.de/images/articles/4012345678902.jpg".into()),
                price_amount: 0.99,
                price_currency: "EUR".into(),
            },
            ArticleRecord {
                ean: "4012345678903".into(),
                name: "Coca-Cola 1,5L".into(),
                description: Some("Erfrischungsgetränk, 1,5 Liter".into()),
                category_id: "cat-beverage".into(),
                category_name: "Getränke".into(),
                brand: "Coca-Cola".into(),
                image_url: Some("https://cdn.edeka.de/images/articles/4012345678903.jpg".into()),
                price_amount: 1.49,
                price_currency: "EUR".into(),
            },
            ArticleRecord {
                ean: "4012345678904".into(),
                name: "EDEKA Lachsfilet 200g".into(),
                description: Some("Tiefkühl-Lachsfilet, 200g".into()),
                category_id: "cat-fish".into(),
                category_name: "Fisch".into(),
                brand: "EDEKA".into(),
                image_url: Some("https://cdn.edeka.de/images/articles/4012345678904.jpg".into()),
                price_amount: 4.99,
                price_currency: "EUR".into(),
            },
            ArticleRecord {
                ean: "4012345678905".into(),
                name: "EDEKA Äpfel Jonagold 1kg".into(),
                description: Some("Frische Äpfel Jonagold, 1kg Beutel".into()),
                category_id: "cat-fruit".into(),
                category_name: "Obst & Gemüse".into(),
                brand: "EDEKA".into(),
                image_url: Some("https://cdn.edeka.de/images/articles/4012345678905.jpg".into()),
                price_amount: 2.49,
                price_currency: "EUR".into(),
            },
        ];

        for rec in records {
            map.insert(rec.ean.clone(), rec);
        }
        Self {
            articles: Arc::new(RwLock::new(map)),
        }
    }
}

// ---------------------------------------------------------------------------
// Federation Entity: Article
// ---------------------------------------------------------------------------

#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct Article {
    ean: String,
}

#[ComplexObject]
impl Article {
    async fn name(&self, ctx: &Context<'_>) -> String {
        let store = ctx.data_unchecked::<DataStore>();
        store
            .articles
            .read()
            .await
            .get(&self.ean)
            .map(|r| r.name.clone())
            .unwrap_or_default()
    }

    async fn description(&self, ctx: &Context<'_>) -> Option<String> {
        let store = ctx.data_unchecked::<DataStore>();
        store
            .articles
            .read()
            .await
            .get(&self.ean)
            .and_then(|r| r.description.clone())
    }

    async fn category(&self, ctx: &Context<'_>) -> Option<Category> {
        let store = ctx.data_unchecked::<DataStore>();
        store.articles.read().await.get(&self.ean).map(|r| Category {
            id: r.category_id.clone().into(),
            name: r.category_name.clone(),
        })
    }

    async fn brand(&self, ctx: &Context<'_>) -> String {
        let store = ctx.data_unchecked::<DataStore>();
        store
            .articles
            .read()
            .await
            .get(&self.ean)
            .map(|r| r.brand.clone())
            .unwrap_or_default()
    }

    async fn image_url(&self, ctx: &Context<'_>) -> Option<String> {
        let store = ctx.data_unchecked::<DataStore>();
        store
            .articles
            .read()
            .await
            .get(&self.ean)
            .and_then(|r| r.image_url.clone())
    }

    async fn price(&self, ctx: &Context<'_>) -> Option<Price> {
        let store = ctx.data_unchecked::<DataStore>();
        store.articles.read().await.get(&self.ean).map(|r| Price {
            amount: r.price_amount,
            currency: r.price_currency.clone(),
            valid_from: Some("2025-01-01".into()),
            valid_to: None,
        })
    }
}

// Federation entity resolution (for use in Phase 3 when subgraphs split).
// When called via `_entities(representations: [{__typename: "Article", ean: "..."}])`,
// this resolver returns the full Article from the data store.
#[allow(dead_code)]
impl Article {
    pub async fn resolve_by_ean(ean: &str, ctx: &Context<'_>) -> Option<Article> {
        let store = ctx.data_unchecked::<DataStore>();
        if store.articles.read().await.contains_key(ean) {
            Some(Article {
                ean: ean.to_owned(),
            })
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Supporting types
// ---------------------------------------------------------------------------

#[derive(SimpleObject, Clone)]
pub struct Category {
    pub id: ID,
    pub name: String,
}

#[derive(SimpleObject, Clone)]
pub struct Price {
    pub amount: f64,
    pub currency: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
}

#[derive(SimpleObject)]
pub struct ArticleConnection {
    pub edges: Vec<ArticleEdge>,
    pub total_count: usize,
}

#[derive(SimpleObject)]
pub struct ArticleEdge {
    pub node: Article,
    pub cursor: String,
}

#[derive(InputObject)]
pub struct ArticleFilter {
    pub eans: Option<Vec<String>>,
    pub category: Option<String>,
    pub brand: Option<String>,
    pub search: Option<String>,
}

// ---------------------------------------------------------------------------
// Query root
// ---------------------------------------------------------------------------

pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn article(&self, ctx: &Context<'_>, ean: String) -> Option<Article> {
        let store = ctx.data_unchecked::<DataStore>();
        if store.articles.read().await.contains_key(&ean) {
            Some(Article { ean })
        } else {
            None
        }
    }

    async fn articles(
        &self,
        ctx: &Context<'_>,
        filter: Option<ArticleFilter>,
    ) -> Vec<Article> {
        let store = ctx.data_unchecked::<DataStore>();
        let articles = store.articles.read().await;

        articles
            .values()
            .filter(|r| {
                if let Some(ref f) = filter {
                    if let Some(ref eans) = f.eans {
                        if !eans.contains(&r.ean) {
                            return false;
                        }
                    }
                    if let Some(ref cat) = f.category {
                        if !r.category_name.to_lowercase().contains(&cat.to_lowercase()) {
                            return false;
                        }
                    }
                    if let Some(ref brand) = f.brand {
                        if !r.brand.to_lowercase().contains(&brand.to_lowercase()) {
                            return false;
                        }
                    }
                    if let Some(ref q) = f.search {
                        let lower = q.to_lowercase();
                        let name_match = r.name.to_lowercase().contains(&lower);
                        let desc_match = r
                            .description
                            .as_ref()
                            .map(|d| d.to_lowercase().contains(&lower))
                            .unwrap_or(false);
                        if !name_match && !desc_match {
                            return false;
                        }
                    }
                }
                true
            })
            .map(|r| Article {
                ean: r.ean.clone(),
            })
            .collect()
    }

    async fn search(
        &self,
        ctx: &Context<'_>,
        query: String,
        #[graphql(default = 20)] first: usize,
    ) -> ArticleConnection {
        let store = ctx.data_unchecked::<DataStore>();
        let lower = query.to_lowercase();
        let articles = store.articles.read().await;

        let results: Vec<_> = articles
            .values()
            .filter(|r| {
                r.name.to_lowercase().contains(&lower)
                    || r.description
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&lower))
                        .unwrap_or(false)
            })
            .take(first.max(1).min(100))
            .collect();

        let total_count = results.len();
        let edges: Vec<ArticleEdge> = results
            .into_iter()
            .enumerate()
            .map(|(i, r)| ArticleEdge {
                cursor: base64_encode(&format!("cursor:{}", i)),
                node: Article {
                    ean: r.ean.clone(),
                },
            })
            .collect();

        ArticleConnection {
            edges,
            total_count,
        }
    }
}

// ---------------------------------------------------------------------------
// Schema builder
// ---------------------------------------------------------------------------

pub type FederationSchema = Schema<QueryRoot, EmptyMutation, EmptySubscription>;

pub fn build_schema() -> FederationSchema {
    Schema::build(QueryRoot, EmptyMutation, EmptySubscription)
        .data(DataStore::with_seed_data())
        .finish()
}

fn base64_encode(s: &str) -> String {
    let mut buf = Vec::new();
    // Simple base64 encoding without external crates
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        buf.push(CHARS[((triple >> 18) & 0x3F) as usize]);
        buf.push(CHARS[((triple >> 12) & 0x3F) as usize]);

        if chunk.len() > 1 {
            buf.push(CHARS[((triple >> 6) & 0x3F) as usize]);
        } else {
            buf.push(b'=');
        }

        if chunk.len() > 2 {
            buf.push(CHARS[(triple & 0x3F) as usize]);
        } else {
            buf.push(b'=');
        }
    }
    String::from_utf8(buf).unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn article_by_ean() {
        let schema = build_schema();
        let query = r#"
            query {
                article(ean: "4012345678901") {
                    ean
                    name
                    brand
                    price { amount currency }
                }
            }
        "#;
        let res = schema.execute(query).await;
        assert!(res.is_ok(), "{:?}", res.errors);

        let data = res.data.into_json().unwrap();
        let article = &data["article"];
        assert_eq!(article["ean"], "4012345678901");
        assert!(article["name"].as_str().unwrap().contains("Vollmilch"));
        assert_eq!(article["brand"], "EDEKA Bio");
        assert_eq!(article["price"]["amount"], 1.29);
    }

    #[tokio::test]
    async fn article_not_found_returns_null() {
        let schema = build_schema();
        let query = r#"
            query { article(ean: "nonexistent") { ean } }
        "#;
        let res = schema.execute(query).await;
        let data = res.data.into_json().unwrap();
        assert!(data["article"].is_null());
    }

    #[tokio::test]
    async fn articles_with_filter() {
        let schema = build_schema();
        let query = r#"
            query {
                articles(filter: { brand: "EDEKA Bio" }) {
                    ean
                    brand
                }
            }
        "#;
        let res = schema.execute(query).await;
        assert!(res.is_ok());
        let data = res.data.into_json().unwrap();
        let articles = data["articles"].as_array().unwrap();
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0]["ean"], "4012345678901");
    }

    #[tokio::test]
    async fn search_by_query() {
        let schema = build_schema();
        let query = r#"
            query { search(query: "EDEKA") { edges { node { ean name } } totalCount } }
        "#;
        let res = schema.execute(query).await;
        let data = res.data.into_json().unwrap();
        let search = &data["search"];
        assert!(search["totalCount"].as_u64().unwrap() >= 3);
    }

    #[tokio::test]
    async fn nested_category_in_query() {
        let schema = build_schema();
        let query = r#"
            query {
                article(ean: "4012345678901") {
                    ean
                    category { id name }
                }
            }
        "#;
        let res = schema.execute(query).await;
        let data = res.data.into_json().unwrap();
        assert_eq!(data["article"]["category"]["name"], "Milchprodukte");
    }
}
