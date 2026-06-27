//! Live JSONL exporter backed by OTLP semantic-action rendering.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};

use export_core::{
    BestEffortSemanticActionRoute, BestEffortSemanticActionRouteConfig, BestEffortSink,
    ExportError, SemanticActionExportAdapter, SemanticActionExportRecord,
    SemanticActionExportRoute,
};
use plugin_system::{
    DEFAULT_OBSERVATION_EVENT_FAMILIES, ObservationBatch, ObservationConsumeReport,
    ObservationConsumer, ObservationEventFamily, PluginDroppedRecord, PluginRuntimeError,
    PluginRuntimeKind,
};

use crate::OtelJsonlExporterConfig;

const OTEL_JSONL_EXPORTER_NAME: &str = "otel_live_jsonl";
const OTEL_JSONL_PLUGIN_ID: &str = "otel-jsonl";
pub const OTEL_JSONL_BUILTIN_PLUGIN_INSTANCE_ID: &str = "builtin.otel-jsonl";
const WRITER_THREAD_NAME: &str = "actrail-live-otel-export";

type OtelJsonlSemanticActionRoute = BestEffortSemanticActionRoute<OtelJsonlSemanticActionAdapter>;

pub fn build_otel_jsonl_observation_consumer(
    config: OtelJsonlExporterConfig,
) -> Result<OtelJsonlObservationConsumer, ExportError> {
    build_otel_jsonl_observation_consumer_instance(OTEL_JSONL_BUILTIN_PLUGIN_INSTANCE_ID, config)
}

pub fn build_otel_jsonl_observation_consumer_instance(
    instance_id: impl Into<String>,
    config: OtelJsonlExporterConfig,
) -> Result<OtelJsonlObservationConsumer, ExportError> {
    build_otel_jsonl_observation_consumer_instance_with_subscriptions(
        instance_id,
        config,
        DEFAULT_OBSERVATION_EVENT_FAMILIES.to_vec(),
    )
}

pub fn build_otel_jsonl_observation_consumer_instance_with_subscriptions(
    instance_id: impl Into<String>,
    config: OtelJsonlExporterConfig,
    event_families: Vec<ObservationEventFamily>,
) -> Result<OtelJsonlObservationConsumer, ExportError> {
    Ok(OtelJsonlObservationConsumer::new(
        instance_id,
        build_otel_jsonl_semantic_action_route(config)?,
        event_families,
    ))
}

pub fn parse_otel_jsonl_plugin_config(raw: &str) -> Result<OtelJsonlExporterConfig, String> {
    let value = raw
        .parse::<toml::Value>()
        .map_err(|error| format!("parse otel-jsonl plugin config: {error}"))?;
    let table = value
        .as_table()
        .ok_or_else(|| "otel-jsonl plugin config must be a TOML table".to_string())?;
    let mut entries = Vec::new();
    for (key, value) in table {
        let parsed = match key.as_str() {
            "path" => value
                .as_str()
                .ok_or_else(|| "otel-jsonl config path must be a string".to_string())?
                .to_string(),
            "overwrite_enabled" => value
                .as_bool()
                .ok_or_else(|| "otel-jsonl config overwrite_enabled must be a bool".to_string())?
                .to_string(),
            "queue_capacity" | "flush_every_spans" => value
                .as_integer()
                .ok_or_else(|| format!("otel-jsonl config {key} must be an integer"))?
                .to_string(),
            _ => {
                return Err(format!("unknown config key plugin.otel-jsonl.{key}"));
            }
        };
        entries.push((key.clone(), parsed));
    }
    let config = OtelJsonlExporterConfig::parse_section("plugin.otel-jsonl", entries)?;
    config.validate_enabled_route()?;
    Ok(config)
}

fn build_otel_jsonl_semantic_action_route(
    config: OtelJsonlExporterConfig,
) -> Result<OtelJsonlSemanticActionRoute, ExportError> {
    let file = open_output_file(&config)?;
    if config.flush_every_spans == u32::default() {
        return Err(ExportError::new(
            OTEL_JSONL_EXPORTER_NAME,
            "flush span count must be positive",
        ));
    }
    let flush_every_spans = usize::try_from(config.flush_every_spans).map_err(|error| {
        ExportError::new(
            OTEL_JSONL_EXPORTER_NAME,
            format!("flush span count overflow: {error}"),
        )
    })?;
    let sink = JsonlLineSink::new(file, flush_every_spans);

    BestEffortSemanticActionRoute::spawn(
        OtelJsonlSemanticActionAdapter,
        BestEffortSemanticActionRouteConfig {
            worker_thread_name: WRITER_THREAD_NAME,
            queue_capacity: config.queue_capacity,
        },
        sink,
    )
}

pub struct OtelJsonlObservationConsumer {
    instance_id: String,
    event_families: Vec<ObservationEventFamily>,
    route: OtelJsonlSemanticActionRoute,
}

impl OtelJsonlObservationConsumer {
    fn new(
        instance_id: impl Into<String>,
        route: OtelJsonlSemanticActionRoute,
        event_families: Vec<ObservationEventFamily>,
    ) -> Self {
        Self {
            instance_id: instance_id.into(),
            event_families,
            route,
        }
    }
}

impl ObservationConsumer for OtelJsonlObservationConsumer {
    fn instance_id(&self) -> &str {
        &self.instance_id
    }

    fn plugin_id(&self) -> &str {
        OTEL_JSONL_PLUGIN_ID
    }

    fn runtime_kind(&self) -> PluginRuntimeKind {
        PluginRuntimeKind::Builtin
    }

    fn subscribed_event_families(&self) -> Vec<ObservationEventFamily> {
        self.event_families.clone()
    }

    fn consume(
        &self,
        batch: ObservationBatch<'_>,
    ) -> Result<ObservationConsumeReport, PluginRuntimeError> {
        validate_observation_batch(&batch)?;
        let mut dropped_records = Vec::new();
        for action in batch.semantic_actions {
            let record = SemanticActionExportRecord {
                trace: batch.trace,
                action,
                links: batch.semantic_links,
            };
            match self.route.publish(record) {
                Ok(result) => {
                    let Some(drop) = result.dropped_outcome() else {
                        continue;
                    };
                    if drop.dropped_records() == u64::default() {
                        continue;
                    }
                    dropped_records.push(PluginDroppedRecord {
                        trace_id: action.trace_id,
                        plugin_instance: self.instance_id.clone(),
                        reason: drop.reason().code().to_string(),
                        queue_capacity: drop.queue_capacity(),
                        dropped_records: drop.dropped_records(),
                    });
                }
                Err(error) => {
                    dropped_records.push(PluginDroppedRecord {
                        trace_id: action.trace_id,
                        plugin_instance: self.instance_id.clone(),
                        reason: format!("{}: {}", error.code, error.message),
                        queue_capacity: error.queue_capacity(),
                        dropped_records: 1,
                    });
                }
            }
        }
        Ok(ObservationConsumeReport { dropped_records })
    }
}

struct OtelJsonlSemanticActionAdapter;

impl SemanticActionExportAdapter for OtelJsonlSemanticActionAdapter {
    type Message = String;

    fn name(&self) -> &'static str {
        OTEL_JSONL_EXPORTER_NAME
    }

    fn encode(&self, record: SemanticActionExportRecord<'_>) -> Result<Self::Message, ExportError> {
        Ok(export_otel_codec::render_otlp_json_line(
            record.trace,
            record.action,
            record.links,
        ))
    }
}

fn validate_observation_batch(batch: &ObservationBatch<'_>) -> Result<(), PluginRuntimeError> {
    for action in batch.semantic_actions {
        if action.trace_id != batch.trace.trace_id {
            return Err(PluginRuntimeError::new(
                OTEL_JSONL_EXPORTER_NAME,
                "semantic action trace_id does not match observation trace",
            ));
        }
    }
    for link in batch.semantic_links {
        if link.trace_id != batch.trace.trace_id {
            return Err(PluginRuntimeError::new(
                OTEL_JSONL_EXPORTER_NAME,
                "semantic action link trace_id does not match observation trace",
            ));
        }
    }
    Ok(())
}

struct JsonlLineSink {
    writer: BufWriter<File>,
    flush_every_lines: usize,
    pending_flush: usize,
}

impl JsonlLineSink {
    fn new(file: File, flush_every_lines: usize) -> Self {
        Self {
            writer: BufWriter::new(file),
            flush_every_lines,
            pending_flush: usize::default(),
        }
    }
}

impl BestEffortSink<String> for JsonlLineSink {
    fn deliver(&mut self, line: String) -> Result<(), String> {
        writeln!(self.writer, "{line}").map_err(|error| error.to_string())?;
        self.pending_flush = self.pending_flush.saturating_add(1);
        if self.pending_flush >= self.flush_every_lines {
            self.writer.flush().map_err(|error| error.to_string())?;
            self.pending_flush = usize::default();
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        self.writer.flush().map_err(|error| error.to_string())
    }
}

fn open_output_file(config: &OtelJsonlExporterConfig) -> Result<File, ExportError> {
    create_parent_directory(config)?;
    let mut options = OpenOptions::new();
    options.write(true);
    if config.overwrite_enabled {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    options.open(&config.path).map_err(|error| {
        ExportError::new(
            OTEL_JSONL_EXPORTER_NAME,
            format!("open {} failed: {error}", config.path.display()),
        )
    })
}

fn create_parent_directory(config: &OtelJsonlExporterConfig) -> Result<(), ExportError> {
    let Some(parent) = config
        .path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|error| {
        ExportError::new(
            OTEL_JSONL_EXPORTER_NAME,
            format!(
                "create live OTEL output directory {} failed: {error}",
                parent.display()
            ),
        )
    })
}
