use tracing_subscriber::{
    EnvFilter, Registry,
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

pub fn init() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let json_layer = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_current_span(false)
        .with_span_list(false)
        .with_target(false)
        .with_file(false)
        .with_line_number(false)
        .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339());

    Registry::default()
        .with(env_filter)
        .with(json_layer)
        .init();
}
