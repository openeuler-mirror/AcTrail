//! Built-in OTEL JSONL observation consumer.

mod config;
mod live_jsonl;

pub use config::{OtelJsonlExporterConfig, parse_otel_jsonl_plugin_config};
pub use live_jsonl::{
    OtelJsonlObservationConsumer, build_otel_jsonl_observation_consumer_instance_with_subscriptions,
};
