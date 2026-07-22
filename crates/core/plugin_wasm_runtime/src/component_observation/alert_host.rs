use alert_contract::AlertDraft;
use model_core::ids::TraceId;
use model_core::trace::TraceAlertToken;
use plugin_system::PluginRuntimeError;
use wasmtime::component::{Linker as ComponentLinker, Val};

use crate::engine::WasmStoreState;

const ALERT_WRITE_IMPORT: &str = "actrail:plugin/alert-write@0.2.0";

pub(super) fn register_alert_interface(
    linker: &mut ComponentLinker<WasmStoreState>,
) -> Result<(), PluginRuntimeError> {
    let mut interface = linker.instance(ALERT_WRITE_IMPORT).map_err(link_error)?;
    interface
        .func_new("submit", |store, _ty, params, results| {
            let result = AlertCall::decode(store.data(), params).and_then(AlertCall::submit);
            set_result(results, result);
            Ok(())
        })
        .map_err(link_error)?;
    Ok(())
}

struct AlertCall {
    host: std::sync::Arc<dyn plugin_system::AlertHost>,
    trace_id: TraceId,
    alert_token: TraceAlertToken,
    draft: AlertDraft,
}

impl AlertCall {
    fn decode(state: &WasmStoreState, params: &[Val]) -> Result<Self, PluginRuntimeError> {
        if !state.host_grants().can_write_alerts() {
            return Err(PluginRuntimeError::new(
                "alert_host",
                "alert-write capability is not granted",
            ));
        }
        let [Val::Record(fields)] = params else {
            return Err(invalid_params());
        };
        let trace_id = record_trace_id(fields)?;
        let alert_token = record_alert_token(fields)?;
        let draft_fields = match record_field(fields, "draft") {
            Some(Val::Record(fields)) => fields,
            _ => return Err(invalid_field("draft")),
        };
        let definition_key = record_string(draft_fields, "definition-key")?;
        let payload_json = record_string(draft_fields, "payload-json")?;
        if payload_json.len() > state.host_limits().alert_payload_max_bytes {
            return Err(PluginRuntimeError::new(
                "alert_host",
                "alert payload exceeds the configured byte limit",
            ));
        }
        let host = state
            .alert_host()
            .cloned()
            .ok_or_else(|| PluginRuntimeError::new("alert_host", "alert ingress is unavailable"))?;
        Ok(Self {
            host,
            trace_id,
            alert_token,
            draft: AlertDraft {
                definition_key,
                payload_json,
            },
        })
    }

    fn submit(self) -> Result<(), PluginRuntimeError> {
        self.host
            .submit_alert(self.trace_id, self.alert_token, self.draft)
    }
}

fn record_trace_id(fields: &[(String, Val)]) -> Result<TraceId, PluginRuntimeError> {
    let raw = record_string(fields, "trace-id")?;
    let value = raw
        .strip_prefix("trace-")
        .ok_or_else(|| invalid_field("trace-id"))?
        .parse::<u64>()
        .map_err(|_| invalid_field("trace-id"))?;
    Ok(TraceId::new(value))
}

fn record_alert_token(fields: &[(String, Val)]) -> Result<TraceAlertToken, PluginRuntimeError> {
    let Some(Val::List(values)) = record_field(fields, "alert-token") else {
        return Err(invalid_field("alert-token"));
    };
    let bytes = values
        .iter()
        .map(|value| match value {
            Val::U8(byte) => Ok(*byte),
            _ => Err(invalid_field("alert-token")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    TraceAlertToken::from_slice(&bytes).ok_or_else(|| invalid_field("alert-token"))
}

fn record_field<'a>(fields: &'a [(String, Val)], name: &str) -> Option<&'a Val> {
    fields
        .iter()
        .find_map(|(field, value)| (field == name).then_some(value))
}

fn record_string(fields: &[(String, Val)], name: &str) -> Result<String, PluginRuntimeError> {
    match record_field(fields, name) {
        Some(Val::String(value)) if !value.is_empty() => Ok(value.clone()),
        _ => Err(invalid_field(name)),
    }
}

fn invalid_field(name: &str) -> PluginRuntimeError {
    PluginRuntimeError::new("alert_host", format!("missing or invalid {name}"))
}

fn set_result(results: &mut [Val], result: Result<(), PluginRuntimeError>) {
    let Some(slot) = results.first_mut() else {
        return;
    };
    *slot = match result {
        Ok(()) => Val::Result(Ok(None)),
        Err(error) => Val::Result(Err(Some(Box::new(Val::String(format!(
            "{}: {}",
            error.code, error.message
        )))))),
    };
}

fn invalid_params() -> PluginRuntimeError {
    PluginRuntimeError::new(
        "alert_host",
        "alert-write.submit received invalid parameters",
    )
}

fn link_error(error: impl std::fmt::Display) -> PluginRuntimeError {
    PluginRuntimeError::new(
        "wasm_runtime",
        format!("define alert-write component import failed: {error}"),
    )
}
