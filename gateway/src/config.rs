use std::net::SocketAddr;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub max_cost: u32,
    pub max_depth: u32,
    pub apq_cache_size: usize,
    pub coalescing_enabled: bool,
    pub request_timeout_secs: u64,
}

impl Config {
    pub fn from_env() -> Self {
        let host = std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "0.0.0.0".into());
        let port: u16 = std::env::var("GATEWAY_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4000);

        Self {
            bind_addr: SocketAddr::new(host.parse().expect("invalid bind host"), port),
            max_cost: std::env::var("GATEWAY_MAX_COST")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
            max_depth: std::env::var("GATEWAY_MAX_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            apq_cache_size: std::env::var("GATEWAY_APQ_CACHE_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10000),
            coalescing_enabled: std::env::var("GATEWAY_COALESCING_ENABLED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(true),
            request_timeout_secs: std::env::var("GATEWAY_REQUEST_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
        }
    }
}
