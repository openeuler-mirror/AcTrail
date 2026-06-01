//! Non-blocking live OTEL JSONL sink for committed semantic actions.

use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use config_core::daemon::LiveOtelExportConfig;
use control_contract::reply::ControlError;
use model_core::trace::TraceRecord;
use semantic_action::SemanticAction;

const WRITER_THREAD_NAME: &str = "actrail-live-otel-export";

pub(crate) struct LiveOtelExporter {
    sender: Option<SyncSender<WriterMessage>>,
    writer: Option<JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
    queue_capacity: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LiveOtelPublishResult {
    dropped_spans: u64,
}

enum WriterMessage {
    Line(String),
}

impl LiveOtelExporter {
    pub(crate) fn new(config: LiveOtelExportConfig) -> Result<Self, ControlError> {
        let error = Arc::new(Mutex::new(None));
        if !config.enabled {
            return Ok(Self {
                sender: None,
                writer: None,
                error,
                queue_capacity: config.queue_capacity,
            });
        }

        let file = open_output_file(&config)?;
        let queue_capacity = usize::try_from(config.queue_capacity).map_err(|error| {
            ControlError::new(
                "otel_live_export",
                format!("queue capacity overflow: {error}"),
            )
        })?;
        let flush_every_spans = usize::try_from(config.flush_every_spans).map_err(|error| {
            ControlError::new(
                "otel_live_export",
                format!("flush span count overflow: {error}"),
            )
        })?;
        let (sender, receiver) = sync_channel(queue_capacity);
        let thread_error = Arc::clone(&error);
        let writer = thread::Builder::new()
            .name(WRITER_THREAD_NAME.to_string())
            .spawn(move || {
                let mut writer = BufWriter::new(file);
                let mut pending_flush = usize::default();
                while let Ok(message) = receiver.recv() {
                    match message {
                        WriterMessage::Line(line) => {
                            if let Err(error) = writeln!(writer, "{line}") {
                                store_writer_error(&thread_error, error.to_string());
                                return;
                            }
                            pending_flush = pending_flush.saturating_add(1);
                            if pending_flush >= flush_every_spans {
                                if let Err(error) = writer.flush() {
                                    store_writer_error(&thread_error, error.to_string());
                                    return;
                                }
                                pending_flush = usize::default();
                            }
                        }
                    }
                }
                if let Err(error) = writer.flush() {
                    store_writer_error(&thread_error, error.to_string());
                }
            })
            .map_err(|error| {
                ControlError::new(
                    "otel_live_export",
                    format!("spawn writer thread failed: {error}"),
                )
            })?;

        Ok(Self {
            sender: Some(sender),
            writer: Some(writer),
            error,
            queue_capacity: config.queue_capacity,
        })
    }

    pub(crate) fn enabled(&self) -> bool {
        self.sender.is_some()
    }

    pub(crate) fn queue_capacity(&self) -> u32 {
        self.queue_capacity
    }

    pub(crate) fn check_health(&self) -> Result<(), ControlError> {
        let error = self.error.lock().map_err(|error| {
            ControlError::new(
                "otel_live_export",
                format!("writer error lock poisoned: {error}"),
            )
        })?;
        match error.as_ref() {
            Some(message) => Err(ControlError::new("otel_live_export", message.clone())),
            None => Ok(()),
        }
    }

    pub(crate) fn publish(
        &self,
        trace: &TraceRecord,
        action: &SemanticAction,
    ) -> Result<LiveOtelPublishResult, ControlError> {
        self.check_health()?;
        let Some(sender) = &self.sender else {
            return Ok(LiveOtelPublishResult {
                dropped_spans: u64::default(),
            });
        };
        let line = otel_export::render_otlp_json_line(trace, action);
        match sender.try_send(WriterMessage::Line(line)) {
            Ok(()) => Ok(LiveOtelPublishResult {
                dropped_spans: u64::default(),
            }),
            Err(TrySendError::Full(_)) => Ok(LiveOtelPublishResult { dropped_spans: 1 }),
            Err(TrySendError::Disconnected(_)) => {
                self.check_health()?;
                Err(ControlError::new(
                    "otel_live_export",
                    "writer channel disconnected",
                ))
            }
        }
    }
}

impl LiveOtelPublishResult {
    pub(crate) const fn dropped_spans(self) -> u64 {
        self.dropped_spans
    }
}

impl Drop for LiveOtelExporter {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

fn open_output_file(config: &LiveOtelExportConfig) -> Result<std::fs::File, ControlError> {
    let mut options = OpenOptions::new();
    options.write(true);
    if config.overwrite_enabled {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    options.open(&config.path).map_err(|error| {
        ControlError::new(
            "otel_live_export",
            format!("open {} failed: {error}", config.path.display()),
        )
    })
}

fn store_writer_error(error: &Arc<Mutex<Option<String>>>, message: String) {
    if let Ok(mut slot) = error.lock() {
        *slot = Some(message);
    }
}
