//! Built-in OTEL JSONL observation consumer.

mod config;
mod live_jsonl;

pub use config::{OTEL_JSONL_ROUTE_KIND, OtelJsonlExporterConfig};
pub use live_jsonl::{
    OTEL_JSONL_BUILTIN_PLUGIN_INSTANCE_ID, OtelJsonlObservationConsumer,
    build_otel_jsonl_observation_consumer, build_otel_jsonl_observation_consumer_instance,
    build_otel_jsonl_observation_consumer_instance_with_subscriptions,
    parse_otel_jsonl_plugin_config,
};
