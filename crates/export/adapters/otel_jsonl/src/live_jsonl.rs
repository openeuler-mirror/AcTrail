//! Live JSONL exporter backed by OTLP semantic-action rendering.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};

use export_core::{
    BestEffortSemanticActionRoute, BestEffortSemanticActionRouteConfig, BestEffortSink,
    ExportError, SemanticActionExportAdapter, SemanticActionExportRecord,
};

use crate::OtelJsonlExporterConfig;

const OTEL_JSONL_EXPORTER_NAME: &str = "otel_live_jsonl";
const WRITER_THREAD_NAME: &str = "actrail-live-otel-export";

pub type OtelJsonlSemanticActionRoute =
    BestEffortSemanticActionRoute<OtelJsonlSemanticActionAdapter>;

pub fn build_otel_jsonl_semantic_action_route(
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

pub struct OtelJsonlSemanticActionAdapter;

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
