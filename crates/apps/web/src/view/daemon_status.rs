//! Operator-socket probe backing the console's daemon liveness indicator.

use config_core::daemon::OperatorConfig;
use control_contract::command::{ControlCommand, DoctorCommand, ListTracesCommand};
use control_contract::reply::ControlReply;
use uds_control_client::{UdsControlClient, UdsSocketTransport};

use super::web_request_id;
use crate::json;

pub(crate) fn daemon_status_json(
    operator_config: Option<&OperatorConfig>,
) -> Result<String, String> {
    let Some(config) = operator_config else {
        return Ok(status_json(
            false,
            None,
            Some("operator config was not loaded; actrailweb is running in storage-only mode"),
            None,
        ));
    };
    let socket_path = config.socket_path.display().to_string();
    let mut client = UdsControlClient::new(UdsSocketTransport::new(config.socket_path.clone()));
    let reply = client.send(ControlCommand::Doctor(DoctorCommand {
        request_id: web_request_id()?,
    }));
    match reply {
        Ok(ControlReply::Doctor(_)) => {
            // Traces the daemon is tracking right now; the stored lifecycle state
            // is only a snapshot from the last write and goes stale once the
            // daemon stops, so it cannot answer "is an agent running" on its own.
            let active_traces = match client.send(ControlCommand::ListTraces(ListTracesCommand {
                request_id: web_request_id()?,
                selector: None,
            })) {
                Ok(ControlReply::TraceList(items)) => Some(items.len()),
                Ok(_) | Err(_) => None,
            };
            Ok(status_json(true, Some(&socket_path), None, active_traces))
        }
        Ok(_) => Err("daemon returned an unexpected reply to the readiness probe".to_string()),
        Err(error) => Ok(status_json(
            false,
            Some(&socket_path),
            Some(&format!(
                "daemon did not answer: {}: {}",
                error.code, error.message
            )),
            None,
        )),
    }
}

fn status_json(
    available: bool,
    socket_path: Option<&str>,
    reason: Option<&str>,
    active_traces: Option<usize>,
) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "available", &json::boolean(available));
    output.push(',');
    json::field(&mut output, "reason", &json::optional_string(reason));
    output.push(',');
    json::field(
        &mut output,
        "socket_path",
        &json::optional_string(socket_path),
    );
    output.push(',');
    json::field(
        &mut output,
        "active_traces",
        &json::optional_number(active_traces),
    );
    output.push('}');
    output
}
