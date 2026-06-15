//! Live OTEL JSONL export adaptor.

mod config;
mod live_jsonl;

pub use config::{OTEL_JSONL_ROUTE_KIND, OtelJsonlExporterConfig};
pub use live_jsonl::{
    OtelJsonlSemanticActionAdapter, OtelJsonlSemanticActionRoute,
    build_otel_jsonl_semantic_action_route,
};
