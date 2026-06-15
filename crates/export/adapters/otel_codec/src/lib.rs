//! OTLP JSON rendering for semantic action export.

mod serialize;
mod service;

pub use service::{OtelExportError, render_otlp_json, render_otlp_json_line};
