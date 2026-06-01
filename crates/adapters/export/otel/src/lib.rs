//! OpenTelemetry OTLP JSON export for semantic actions.

pub mod serialize;
pub mod service;

pub use service::{OtelExportError, render_otlp_json, render_otlp_json_line};
