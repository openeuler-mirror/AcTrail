//! Realtime OTLP/HTTP export adapter. Transport is std sockets, optionally
//! wrapped in system-OpenSSL TLS (with mutual TLS) for `https://` endpoints.

mod config;
mod live_http;

pub use config::{
    Endpoint, OTEL_HTTP_ROUTE_KIND, OtelAttributeMode, OtelCompression, OtelEncoding,
    OtelHttpExporterConfig, OtelHttpTlsConfig, parse_otel_http_plugin_config,
};
pub use live_http::{
    OTEL_HTTP_BUILTIN_PLUGIN_INSTANCE_ID, OtelHttpObservationConsumer,
    build_otel_http_observation_consumer, build_otel_http_observation_consumer_instance,
    build_otel_http_observation_consumer_instance_with_subscriptions,
};
