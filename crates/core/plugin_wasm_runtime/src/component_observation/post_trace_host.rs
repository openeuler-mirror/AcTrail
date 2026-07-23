use plugin_system::{PluginRuntimeError, TraceFileStateStatus};
use wasmtime::component::{Linker as ComponentLinker, Val};

use crate::component_observation::wire::{system_time_millis, trace_analysis_action_val};
use crate::engine::WasmStoreState;

const TRACE_ANALYSIS_IMPORT: &str = "actrail:plugin/trace-analysis-read@0.2.0";
const TRACE_FILE_STATE_IMPORT: &str = "actrail:plugin/trace-file-state-read@0.2.0";

pub(super) fn register_post_trace_interfaces(
    linker: &mut ComponentLinker<WasmStoreState>,
) -> Result<(), PluginRuntimeError> {
    register_trace_analysis(linker)?;
    register_file_state(linker)
}

fn register_trace_analysis(
    linker: &mut ComponentLinker<WasmStoreState>,
) -> Result<(), PluginRuntimeError> {
    let mut interface = linker.instance(TRACE_ANALYSIS_IMPORT).map_err(link_error)?;
    interface
        .func_new("context-get", |store, _ty, _params, results| {
            set_result(results, analysis_context(store.data()));
            Ok(())
        })
        .map_err(link_error)?;
    interface
        .func_new(
            "semantic-actions-list",
            |mut store, _ty, params, results| {
                let result = page_request(store.data(), params).and_then(
                    |(host, trace_id, offset, limit)| {
                        let page = host.semantic_actions_page(trace_id, offset, limit)?;
                        let row_count = page.actions.len();
                        let task = store.data_mut().post_trace_task_mut().ok_or_else(|| {
                            PluginRuntimeError::new(
                                "post_trace_host",
                                "post-trace task context is unavailable",
                            )
                        })?;
                        task.action_rows_read = task
                            .action_rows_read
                            .checked_add(row_count)
                            .ok_or_else(|| {
                                PluginRuntimeError::new(
                                    "post_trace_host",
                                    "action read counter overflow",
                                )
                            })?;
                        if task.action_rows_read > task.limits.action_total_max_count {
                            return Err(PluginRuntimeError::new(
                                "post_trace_host",
                                "trace action total read limit exceeded",
                            ));
                        }
                        Ok(Val::Record(vec![
                            (
                                "actions".to_string(),
                                Val::List(
                                    page.actions.iter().map(trace_analysis_action_val).collect(),
                                ),
                            ),
                            (
                                "next-offset".to_string(),
                                option_u64(
                                    page.next_offset.and_then(|value| value.try_into().ok()),
                                ),
                            ),
                        ]))
                    },
                );
                set_result(results, result);
                Ok(())
            },
        )
        .map_err(link_error)?;
    Ok(())
}

fn register_file_state(
    linker: &mut ComponentLinker<WasmStoreState>,
) -> Result<(), PluginRuntimeError> {
    let mut interface = linker
        .instance(TRACE_FILE_STATE_IMPORT)
        .map_err(link_error)?;
    interface
        .func_new("get", |mut store, _ty, params, results| {
            let result =
                file_state_request(store.data(), params).and_then(|(host, trace_id, action_id)| {
                    let state = host.file_state(trace_id, &action_id)?;
                    let task = store.data_mut().post_trace_task_mut().ok_or_else(|| {
                        PluginRuntimeError::new(
                            "post_trace_host",
                            "post-trace task context is unavailable",
                        )
                    })?;
                    task.file_state_queries =
                        task.file_state_queries.checked_add(1).ok_or_else(|| {
                            PluginRuntimeError::new(
                                "post_trace_host",
                                "file-state query counter overflow",
                            )
                        })?;
                    if task.file_state_queries > task.limits.file_state_query_max_count {
                        return Err(PluginRuntimeError::new(
                            "post_trace_host",
                            "file-state query limit exceeded",
                        ));
                    }
                    Ok(Val::Record(vec![
                        (
                            "status".to_string(),
                            Val::Enum(file_state_status(state.status).to_string()),
                        ),
                        (
                            "checked-at".to_string(),
                            Val::U64(system_time_millis(state.checked_at)?),
                        ),
                        (
                            "file-kind".to_string(),
                            Val::Option(state.file_kind.map(Val::String).map(Box::new)),
                        ),
                    ]))
                });
            set_result(results, result);
            Ok(())
        })
        .map_err(link_error)?;
    Ok(())
}

fn analysis_context(state: &WasmStoreState) -> Result<Val, PluginRuntimeError> {
    require_grant(
        state.host_grants().can_read_trace_analysis(),
        "trace-analysis-read",
    )?;
    let (host, trace_id) = current_task(state)?;
    let context = host.analysis_context(trace_id)?;
    Ok(Val::Record(vec![
        (
            "terminal-at".to_string(),
            Val::U64(system_time_millis(context.terminal_at)?),
        ),
        (
            "working-directory".to_string(),
            Val::Option(context.working_directory.map(Val::String).map(Box::new)),
        ),
    ]))
}

type PageRequest = (
    std::sync::Arc<dyn plugin_system::PostTraceHost>,
    model_core::ids::TraceId,
    usize,
    usize,
);

fn page_request(state: &WasmStoreState, params: &[Val]) -> Result<PageRequest, PluginRuntimeError> {
    require_grant(
        state.host_grants().can_read_trace_analysis(),
        "trace-analysis-read",
    )?;
    let [offset, Val::U32(limit)] = params else {
        return Err(invalid_params("semantic-actions-list"));
    };
    let offset = option_u64_value(offset)?
        .map(|value| usize::try_from(value).map_err(limit_overflow))
        .transpose()?
        .unwrap_or_default();
    let limit = usize::try_from(*limit).map_err(limit_overflow)?;
    let task = state
        .post_trace_task()
        .ok_or_else(|| missing_task("semantic-actions-list"))?;
    if limit == 0 || limit > task.limits.action_page_max_count {
        return Err(PluginRuntimeError::new(
            "post_trace_host",
            "semantic-actions-list page limit is zero or exceeds the grant",
        ));
    }
    let (host, trace_id) = current_task(state)?;
    Ok((host, trace_id, offset, limit))
}

type ActionRequest = (
    std::sync::Arc<dyn plugin_system::PostTraceHost>,
    model_core::ids::TraceId,
    String,
);

fn file_state_request(
    state: &WasmStoreState,
    params: &[Val],
) -> Result<ActionRequest, PluginRuntimeError> {
    require_grant(
        state.host_grants().can_read_trace_file_state(),
        "trace-file-state-read",
    )?;
    let [Val::String(action_id)] = params else {
        return Err(invalid_params("trace-file-state-read.get"));
    };
    if action_id.is_empty() {
        return Err(PluginRuntimeError::new(
            "post_trace_host",
            "action id must not be empty",
        ));
    }
    let task = state
        .post_trace_task()
        .ok_or_else(|| missing_task("trace-file-state-read.get"))?;
    if task.file_state_queries >= task.limits.file_state_query_max_count {
        return Err(PluginRuntimeError::new(
            "post_trace_host",
            "file-state query limit exceeded",
        ));
    }
    let (host, trace_id) = current_task(state)?;
    Ok((host, trace_id, action_id.clone()))
}

fn current_task(
    state: &WasmStoreState,
) -> Result<
    (
        std::sync::Arc<dyn plugin_system::PostTraceHost>,
        model_core::ids::TraceId,
    ),
    PluginRuntimeError,
> {
    let trace_id = state
        .post_trace_task()
        .map(|task| task.trace_id)
        .ok_or_else(|| missing_task("hostcall"))?;
    let host = state
        .post_trace_host()
        .cloned()
        .ok_or_else(|| PluginRuntimeError::new("post_trace_host", "host broker is unavailable"))?;
    Ok((host, trace_id))
}

fn option_u64_value(value: &Val) -> Result<Option<u64>, PluginRuntimeError> {
    match value {
        Val::Option(None) => Ok(None),
        Val::Option(Some(value)) => match value.as_ref() {
            Val::U64(value) => Ok(Some(*value)),
            _ => Err(invalid_params("option<u64>")),
        },
        _ => Err(invalid_params("option<u64>")),
    }
}

fn option_u64(value: Option<u64>) -> Val {
    Val::Option(value.map(Val::U64).map(Box::new))
}

fn set_result(results: &mut [Val], result: Result<Val, PluginRuntimeError>) {
    let Some(slot) = results.first_mut() else {
        return;
    };
    *slot = match result {
        Ok(value) => Val::Result(Ok(Some(Box::new(value)))),
        Err(error) => Val::Result(Err(Some(Box::new(Val::String(format!(
            "{}: {}",
            error.code, error.message
        )))))),
    };
}

fn file_state_status(status: TraceFileStateStatus) -> &'static str {
    match status {
        TraceFileStateStatus::Exists => "exists",
        TraceFileStateStatus::NotFound => "not-found",
        TraceFileStateStatus::Inaccessible => "inaccessible",
        TraceFileStateStatus::Unavailable => "unavailable",
    }
}

fn require_grant(granted: bool, name: &str) -> Result<(), PluginRuntimeError> {
    if granted {
        Ok(())
    } else {
        Err(PluginRuntimeError::new(
            "post_trace_host",
            format!("{name} capability is not granted"),
        ))
    }
}

fn link_error(error: impl std::fmt::Display) -> PluginRuntimeError {
    PluginRuntimeError::new(
        "wasm_runtime",
        format!("define post-trace component import failed: {error}"),
    )
}

fn invalid_params(operation: &str) -> PluginRuntimeError {
    PluginRuntimeError::new(
        "post_trace_host",
        format!("{operation} received invalid parameters"),
    )
}

fn missing_task(operation: &str) -> PluginRuntimeError {
    PluginRuntimeError::new(
        "post_trace_host",
        format!("{operation} requires an active post-trace task"),
    )
}

fn limit_overflow(error: std::num::TryFromIntError) -> PluginRuntimeError {
    PluginRuntimeError::new(
        "post_trace_host",
        format!("hostcall integer conversion overflow: {error}"),
    )
}
