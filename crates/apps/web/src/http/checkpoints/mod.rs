//! Checkpoint history and the checkpoint commands Web hands to the daemon.

mod actions;
mod listing;
mod navigation;

use std::path::Path;
use std::time::Duration;

use checkpoint_core::model::{OperationId, OperationRecord};
use checkpoint_sqlite::CheckpointSqliteReader;
use config_core::daemon::WebCheckpointsConfig;
use model_core::ids::TraceId;

use super::query::{parse_u64, required_query_param};
use super::{
    Response, STATUS_BAD_REQUEST, STATUS_METHOD_NOT_ALLOWED, STATUS_NOT_FOUND, WebContext,
};

const COMMANDS: [&str; 6] = ["save", "restore", "resume", "delete", "disconnect", "force"];

/// Routes under `/api/traces/{trace}/checkpoints`.
pub(super) struct CheckpointHttp<'a> {
    context: &'a WebContext,
}

impl<'a> CheckpointHttp<'a> {
    pub(super) fn new(context: &'a WebContext) -> Self {
        Self { context }
    }

    pub(super) fn route(
        &self,
        method: &str,
        path: &str,
        query: &str,
    ) -> Option<Result<Response, String>> {
        let (trace, rest) = path.strip_prefix("/api/traces/")?.split_once('/')?;
        let rest = match rest.strip_prefix("checkpoints")? {
            "" => "",
            rest => rest.strip_prefix('/')?,
        };
        if self.context.cluster_root.is_some() {
            return Some(Ok(Response::text(
                STATUS_NOT_FOUND,
                "checkpoints are queried from the originating host",
            )));
        }
        let trace_id = match parse_u64(trace) {
            Ok(id) => TraceId::new(id),
            Err(error) => return Some(Ok(Response::text(STATUS_BAD_REQUEST, error))),
        };
        let expected = if COMMANDS.contains(&rest) {
            "POST"
        } else {
            "GET"
        };
        if method != expected {
            return Some(Ok(Response::text(
                STATUS_METHOD_NOT_ALLOWED,
                format!("{expected} required for {path}"),
            )));
        }
        let config = self.context.operator_config.as_ref();
        let Some((config, catalog_path)) =
            config.and_then(|config| Some((config, config.checkpoint_catalog_path.as_deref()?)))
        else {
            return Some(Ok(match rest {
                "" => Response::json(r#"{"enabled":false}"#.to_string()),
                _ => Response::text(STATUS_NOT_FOUND, "checkpoints are not enabled"),
            }));
        };
        let request = Request {
            context: self.context,
            catalog_path,
            socket_path: &config.socket_path,
            busy_timeout: Duration::from_millis(config.checkpoint_runtime.busy_timeout_ms),
            config: config.web.checkpoints,
            trace_id,
            query,
        };
        let result = match rest {
            "" => listing::render(&request),
            "navigation" => navigation::render(&request),
            command if COMMANDS.contains(&command) => actions::send(&request, command),
            _ => return Some(Ok(Response::text(STATUS_NOT_FOUND, "not found"))),
        };
        Some(match result {
            Ok(response) => Ok(response),
            Err(Failure::BadRequest(message)) => Ok(Response::text(STATUS_BAD_REQUEST, message)),
            Err(Failure::NotFound(message)) => Ok(Response::text(STATUS_NOT_FOUND, message)),
            Err(Failure::Internal(message)) => Err(message),
        })
    }
}

/// One checkpoint request about a Trace.
struct Request<'a> {
    context: &'a WebContext,
    catalog_path: &'a Path,
    /// The daemon control socket the commands go to.
    socket_path: &'a Path,
    busy_timeout: Duration,
    config: WebCheckpointsConfig,
    trace_id: TraceId,
    query: &'a str,
}

/// Why a checkpoint request has no successful answer.
enum Failure {
    BadRequest(String),
    NotFound(String),
    Internal(String),
}

impl Request<'_> {
    fn reader(&self) -> Result<CheckpointSqliteReader, Failure> {
        CheckpointSqliteReader::open(self.catalog_path, self.busy_timeout).map_err(|error| {
            Failure::Internal(format!(
                "open checkpoint catalog {}: {}",
                self.catalog_path.display(),
                error.message
            ))
        })
    }

    /// A numeric id parameter; the `checkpoint-7` form shown in replies is accepted too.
    fn id(&self, key: &'static str) -> Result<u64, Failure> {
        let value = required_query_param(self.query, key).map_err(Failure::BadRequest)?;
        let digits = value
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('-'))
            .unwrap_or(&value);
        parse_u64(digits).map_err(Failure::BadRequest)
    }

    /// An operation of this Trace; those of other Traces are not found.
    fn operation(&self, reader: &CheckpointSqliteReader) -> Result<OperationRecord, Failure> {
        let id = self.id("operation")?;
        reader
            .operation(OperationId::new(id))
            .map_err(catalog_error)?
            .filter(|operation| operation.trace_id == self.trace_id)
            .ok_or_else(|| Failure::NotFound(format!("operation-{id} of {}", self.trace_id)))
    }
}

fn catalog_error(error: checkpoint_core::ports::CatalogError) -> Failure {
    Failure::Internal(format!("read checkpoint catalog: {}", error.message))
}
