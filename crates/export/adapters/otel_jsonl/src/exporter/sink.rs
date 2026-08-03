use export_core::{BestEffortSink, ExportError};

use super::file::FileExporterSink;
use super::json_rpc_http::JsonRpcHttpExporterSink;
use crate::config::ExporterConfig;

pub(crate) struct ExporterSink {
    selected: SelectedExporterSink,
}

enum SelectedExporterSink {
    File(FileExporterSink),
    JsonRpcHttp(JsonRpcHttpExporterSink),
}

impl ExporterSink {
    pub(crate) fn open(config: ExporterConfig) -> Result<Self, ExportError> {
        let selected = match config {
            ExporterConfig::File(config) => {
                FileExporterSink::open(config).map(SelectedExporterSink::File)
            }
            ExporterConfig::JsonRpcHttp(config) => {
                JsonRpcHttpExporterSink::open(config).map(SelectedExporterSink::JsonRpcHttp)
            }
        }?;
        Ok(Self { selected })
    }
}

impl BestEffortSink<String> for ExporterSink {
    fn deliver(&mut self, message: String) -> Result<u64, String> {
        match &mut self.selected {
            SelectedExporterSink::File(sink) => sink.deliver(message),
            SelectedExporterSink::JsonRpcHttp(sink) => sink.deliver(message),
        }
    }

    fn finish(&mut self) -> Result<u64, String> {
        match &mut self.selected {
            SelectedExporterSink::File(sink) => sink.finish(),
            SelectedExporterSink::JsonRpcHttp(sink) => sink.finish(),
        }
    }
}
