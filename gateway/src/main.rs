mod apq;
mod coalescing;
mod config;
mod cost_analysis;
mod id_translation;
mod logging;
mod pos;
mod schema;
mod subgraph;

use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use cost_analysis::{CostAnalyzer, CostConfig, CostError};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

struct AppState {
    config: config::Config,
    coalescer: coalescing::Coalescer,
    apq_cache: apq::ApqCache,
    schema: schema::FederationSchema,
    #[allow(dead_code)]
    pos_cache: pos::PosCache,
    #[allow(dead_code)]
    pos_metrics: Arc<pos::metrics::LatencyTracker>,
    #[allow(dead_code)]
    id_translator: id_translation::IdTranslator,
    #[allow(dead_code)]
    pim_client: subgraph::pim::PimClient,
    #[allow(dead_code)]
    price_client: subgraph::price::PriceClient,
    #[allow(dead_code)]
    lunar_client: subgraph::lunar::LunarClient,
}

// ---------------------------------------------------------------------------
// GraphQL wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GraphQLRequest {
    query: Option<String>,
    #[serde(rename = "operationName")]
    operation_name: Option<String>,
    variables: Option<Value>,
    extensions: Option<GraphQLExtensions>,
}

#[derive(Debug, Deserialize)]
struct GraphQLExtensions {
    #[serde(rename = "persistedQuery")]
    persisted_query: Option<PersistedQuery>,
}

#[derive(Debug, Deserialize)]
struct PersistedQuery {
    #[serde(rename = "sha256Hash")]
    sha256_hash: String,
    #[allow(dead_code)]
    version: u32,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

enum AppError {
    BadRequest(String),
    CostExceeded(CostError),
    ApqNotFound,
    Coalescing(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::BadRequest(msg) => {
                let body = json!({"errors": [{"message": msg}]});
                (StatusCode::BAD_REQUEST, Json(body)).into_response()
            }
            Self::CostExceeded(e) => {
                let body = json!({
                    "errors": [{
                        "message": e.to_string(),
                        "extensions": {"code": "COST_LIMIT_EXCEEDED"}
                    }]
                });
                (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response()
            }
            Self::ApqNotFound => {
                let resp = apq::ApqErrorResponse::not_found();
                (StatusCode::OK, Json(serde_json::to_value(&resp).unwrap_or_default())).into_response()
            }
            Self::Coalescing(msg) => {
                let body = json!({"errors": [{"message": msg}]});
                (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
            }
        }
    }
}

impl From<CostError> for AppError {
    fn from(e: CostError) -> Self {
        Self::CostExceeded(e)
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    logging::init();
    let config = config::Config::from_env();
    tracing::info!(?config, "edeka-gateway starting");

    let addr = config.bind_addr;

    // --- POS Fast-Read (Pfad B, CQRS) ---
    let pos_cache = pos::PosCache::new();
    let pos_metrics = Arc::new(pos::metrics::LatencyTracker::new());
    let _ingestion_handle = pos::start_ingestion_worker(pos_cache.clone(), 5000);

    // --- ID Translation & Subgraph clients (Phase 3) ---
    let id_translator = id_translation::IdTranslator::with_seed_data();
    let subgraph_cfg = subgraph::SubgraphConfig::default();
    let pim_client = subgraph::pim::PimClient::new(subgraph_cfg.clone());
    let price_client = subgraph::price::PriceClient::new(subgraph_cfg.clone());
    let lunar_client = subgraph::lunar::LunarClient::new(subgraph_cfg);

    let id_mapping_count = id_translator.len();

    // Build schema with subgraph clients injected into async-graphql context
    let schema = schema::build_schema(
        id_translator.clone(),
        pim_client.clone(),
        price_client.clone(),
    );

    let state = Arc::new(AppState {
        coalescer: coalescing::Coalescer::new(
            config.coalescing_enabled,
            Duration::from_secs(config.request_timeout_secs),
        ),
        apq_cache: apq::ApqCache::new(config.apq_cache_size),
        schema,
        pos_cache: pos_cache.clone(),
        pos_metrics: pos_metrics.clone(),
        id_translator,
        pim_client,
        price_client,
        lunar_client,
        config,
    });

    tracing::info!(
        cache_entries = pos_cache.len(),
        id_mappings = id_mapping_count,
        "POS cache initialised, REDDI ingestion started"
    );

    // POS handler wrappers — extract state and delegate to pure impl functions
    let pos_cache_clone = pos_cache.clone();
    let pos_metrics_clone = pos_metrics.clone();

    let app = Router::new()
        .route("/graphql", axum::routing::post(graphql_handler))
        .route("/health", get(health_handler))
        .route("/healthz", get(health_handler))
        .route(
            "/api/v1/pos/article/:ean",
            get({
                let cache = pos_cache_clone.clone();
                let metrics = pos_metrics_clone.clone();
                move |axum::extract::Path(ean): axum::extract::Path<String>| {
                    let cache = cache.clone();
                    let metrics = metrics.clone();
                    async move {
                        pos::handler::get_article_impl(&cache, &metrics, &ean).await
                    }
                }
            }),
        )
        .route(
            "/api/v1/pos/metrics",
            get({
                let metrics = pos_metrics_clone.clone();
                move || {
                    let metrics = metrics.clone();
                    async move { pos::handler::get_metrics_impl(&metrics).await }
                }
            }),
        )
        .route(
            "/api/v1/pos/health",
            get(|| async { pos::handler::health_impl().await }),
        )
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    tracing::info!(%addr, "listening");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health_handler() -> Json<Value> {
    Json(json!({"status": "ok", "service": "edeka-gateway"}))
}

async fn graphql_handler(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<Value>, AppError> {
    // 1. Parse the incoming GraphQL request
    let mut req: GraphQLRequest = serde_json::from_slice(&body)
        .map_err(|e| AppError::BadRequest(format!("invalid JSON: {}", e)))?;

    // 2. APQ resolution
    let query = resolve_apq(&mut req, &state.apq_cache)?;

    // 3. Cost analysis
    let cost_cfg = CostConfig {
        max_cost: state.config.max_cost,
        max_depth: state.config.max_depth,
        ..Default::default()
    };
    let analyzer = CostAnalyzer::new(cost_cfg);
    let cost = analyzer.analyze(&query)?;
    tracing::info!(cost, operation = ?req.operation_name, "cost accepted");

    // 4. Execute via single-flight coalescer
    let key = coalescing::Coalescer::make_key(
        Some(&query),
        req.operation_name.as_deref(),
        req.variables.as_ref(),
    );

    let schema = state.schema.clone();
    let result = state
        .coalescer
        .execute(key, || {
            let schema = schema.clone();
            let q = query.clone();
            async move {
                let request = async_graphql::Request::new(q).variables(
                    async_graphql::Variables::from_json(
                        req.variables.unwrap_or(Value::Null),
                    ),
                );
                let response = schema.execute(request).await;
                let json = serde_json::to_value(response)
                    .map_err(|e| coalescing::CoalescingError::Internal(e.to_string()))?;
                Ok(json)
            }
        })
        .await
        .map_err(|e| AppError::Coalescing(e.to_string()))?;

    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// APQ resolution logic
// ---------------------------------------------------------------------------

fn resolve_apq(req: &mut GraphQLRequest, cache: &apq::ApqCache) -> Result<String, AppError> {
    if let Some(ref ext) = req.extensions {
        if let Some(ref pq) = ext.persisted_query {
            let hash = &pq.sha256_hash;

            return if req.query.is_none() {
                // Hash only — must be in cache or return PERSISTED_QUERY_NOT_FOUND
                cache.lookup(hash).ok_or(AppError::ApqNotFound)
            } else {
                // Hash + query — cache and use the query
                let query = req.query.take().unwrap();
                cache.store(&query);
                Ok(query)
            };
        }
    }

    req.query
        .take()
        .ok_or_else(|| AppError::BadRequest("missing query or persistedQuery extension".into()))
}
