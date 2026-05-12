use async_graphql::*;

use crate::id_translation::IdTranslator;
use crate::subgraph::pim::{PimArticle, PimClient};
use crate::subgraph::price::{PriceClient, PriceData};

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
        resolve_pim(self.ean.clone(), ctx)
            .await
            .map(|a| a.name)
            .unwrap_or_default()
    }

    async fn description(&self, ctx: &Context<'_>) -> Option<String> {
        resolve_pim(self.ean.clone(), ctx).await.and_then(|a| a.description)
    }

    async fn category(&self, ctx: &Context<'_>) -> Option<Category> {
        resolve_pim(self.ean.clone(), ctx).await.map(|a| Category {
            id: a.category_id.into(),
            name: a.category_name,
        })
    }

    async fn brand(&self, ctx: &Context<'_>) -> String {
        resolve_pim(self.ean.clone(), ctx)
            .await
            .map(|a| a.brand)
            .unwrap_or_default()
    }

    async fn image_url(&self, ctx: &Context<'_>) -> Option<String> {
        resolve_pim(self.ean.clone(), ctx).await.and_then(|a| a.image_url)
    }

    async fn price(&self, ctx: &Context<'_>) -> Option<Price> {
        resolve_price(self.ean.clone(), ctx).await.map(|p| Price {
            amount: p.amount,
            currency: p.currency.clone(),
            valid_from: p.valid_from,
            valid_to: p.valid_to,
        })
    }
}

// ---------------------------------------------------------------------------
// Federation entity resolution
// ---------------------------------------------------------------------------

impl Article {
    pub async fn resolve_by_ean(ean: &str, ctx: &Context<'_>) -> Option<Article> {
        let translator = ctx.data_unchecked::<IdTranslator>();
        if translator.is_whitelisted(ean) {
            Some(Article {
                ean: ean.to_owned(),
            })
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Resolver helpers — called from ComplexObject resolvers above
// ---------------------------------------------------------------------------

async fn resolve_pim(ean: String, ctx: &Context<'_>) -> Option<PimArticle> {
    let translator = ctx.data_unchecked::<IdTranslator>();
    let entry = translator.translate(&ean)?;

    let pim = ctx.data_unchecked::<PimClient>();
    pim.get_article(&entry.matnr).await.ok()
}

async fn resolve_price(ean: String, ctx: &Context<'_>) -> Option<PriceData> {
    let translator = ctx.data_unchecked::<IdTranslator>();
    let entry = translator.translate(&ean)?;

    let price = ctx.data_unchecked::<PriceClient>();
    price.get_price(&entry.matnr).await.ok()
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
        Article::resolve_by_ean(&ean, ctx).await
    }

    async fn articles(
        &self,
        ctx: &Context<'_>,
        filter: Option<ArticleFilter>,
    ) -> Vec<Article> {
        // Collect known EANs from the translator
        let translator = ctx.data_unchecked::<IdTranslator>();
        let translator = translator.clone();

        // For the filter-based query, iterate over known EANs.
        // In production, this would delegate to a search-capable subgraph.
        let mut results: Vec<Article> = Vec::new();

        // Use a static list of known EANs for the stub phase.
        // Production: query PIM search endpoint.
        let known_eans = translator.known_eans();

        // Apply filters to the known EAN set
        for ean in &known_eans {
            if let Some(ref f) = filter {
                if let Some(ref eans) = f.eans {
                    if !eans.contains(ean) {
                        continue;
                    }
                }
            }
            if let Some(ref _cat) = filter.as_ref().and_then(|f| f.category.as_ref()) {
                // Category filter requires resolving the article first (expensive).
                // Production: push category filter into PIM query.
                if let Some(pim) = resolve_pim(ean.clone(), ctx).await {
                    if !pim
                        .category_name
                        .to_lowercase()
                        .contains(&_cat.to_lowercase())
                    {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            if let Some(ref _brand) = filter.as_ref().and_then(|f| f.brand.as_ref()) {
                if let Some(pim) = resolve_pim(ean.clone(), ctx).await {
                    if !pim.brand.to_lowercase().contains(&_brand.to_lowercase()) {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            if let Some(ref _q) = filter.as_ref().and_then(|f| f.search.as_ref()) {
                if let Some(pim) = resolve_pim(ean.clone(), ctx).await {
                    let lower = _q.to_lowercase();
                    let name_match = pim.name.to_lowercase().contains(&lower);
                    let desc_match = pim
                        .description
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&lower))
                        .unwrap_or(false);
                    if !name_match && !desc_match {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            results.push(Article { ean: ean.clone() });
        }

        results
    }

    async fn search(
        &self,
        ctx: &Context<'_>,
        query: String,
        #[graphql(default = 20)] first: usize,
    ) -> ArticleConnection {
        let translator = ctx.data_unchecked::<IdTranslator>();
        let known_eans = translator.known_eans();
        let lower = query.to_lowercase();

        let mut results: Vec<String> = Vec::new();
        let limit = first.max(1).min(100);

        for ean in &known_eans {
            if results.len() >= limit {
                break;
            }
            if let Some(pim) = resolve_pim(ean.clone(), ctx).await {
                let name_match = pim.name.to_lowercase().contains(&lower);
                let desc_match = pim
                    .description
                    .as_ref()
                    .map(|d| d.to_lowercase().contains(&lower))
                    .unwrap_or(false);
                if name_match || desc_match {
                    results.push(ean.clone());
                }
            }
        }

        let total_count = results.len();
        let edges: Vec<ArticleEdge> = results
            .into_iter()
            .enumerate()
            .map(|(i, ean)| ArticleEdge {
                cursor: base64_encode(&format!("cursor:{}", i)),
                node: Article { ean },
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

pub fn build_schema(
    id_translator: IdTranslator,
    pim_client: PimClient,
    price_client: PriceClient,
) -> FederationSchema {
    Schema::build(QueryRoot, EmptyMutation, EmptySubscription)
        .data(id_translator)
        .data(pim_client)
        .data(price_client)
        .finish()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn base64_encode(s: &str) -> String {
    let mut buf = Vec::new();
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
    use crate::id_translation::IdTranslator;
    use crate::subgraph::{pim::PimClient, price::PriceClient, SubgraphConfig};

    fn test_schema() -> FederationSchema {
        build_schema(
            IdTranslator::with_seed_data(),
            PimClient::new(SubgraphConfig::default()),
            PriceClient::new(SubgraphConfig::default()),
        )
    }

    #[tokio::test]
    async fn article_by_ean() {
        let schema = test_schema();
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
        let schema = test_schema();
        let query = r#"
            query { article(ean: "nonexistent") { ean } }
        "#;
        let res = schema.execute(query).await;
        let data = res.data.into_json().unwrap();
        assert!(data["article"].is_null());
    }

    #[tokio::test]
    async fn articles_with_filter() {
        let schema = test_schema();
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
        let schema = test_schema();
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
        let schema = test_schema();
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
