//! HTTP clients for downstream GraphQL subgraphs.
//!
//! Each subgraph client wraps a `reqwest::Client` with per-subgraph base URL,
//! timeout, and typed response parsing. Stub mode returns seed data for
//! development without running the actual subgraph services.

pub mod lunar;
pub mod pim;
pub mod price;

use std::time::Duration;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// Per-subgraph configuration.
#[derive(Clone, Debug)]
pub struct SubgraphConfig {
    pub base_url: String,
    pub timeout: Duration,
    /// When true, return seed data instead of making HTTP calls.
    pub stub_mode: bool,
}

impl Default for SubgraphConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:4000".into(),
            timeout: Duration::from_secs(5),
            stub_mode: true,
        }
    }
}

/// Generic GraphQL wire types for subgraph responses.
#[derive(Debug, Deserialize)]
pub struct GraphQlResponse<T> {
    pub data: Option<T>,
    #[allow(dead_code)]
    pub errors: Option<Vec<GraphQlError>>,
}

#[derive(Debug, Deserialize)]
pub struct GraphQlError {
    #[allow(dead_code)]
    pub message: String,
}

// ---------------------------------------------------------------------------
// Subgraph error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SubgraphError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("subgraph returned errors: {0:?}")]
    #[allow(dead_code)]
    GraphQl(Vec<GraphQlError>),

    #[error("subgraph returned null data")]
    NullData,

    #[error("timeout after {0:?}")]
    #[allow(dead_code)]
    Timeout(Duration),
}
