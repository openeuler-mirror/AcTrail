//! Links operations at LLM request boundaries to the LLM calls of the ActionTree.
//!
//! An operation records the `llm.request` action whose response waited at its boundary. The LLM
//! call that the ActionTree shows that request under is the match, and both navigation directions
//! use it.

use std::collections::BTreeMap;

use checkpoint_core::model::{OperationId, OperationRecord};
use model_core::ids::TraceId;
use storage_core::{StorageBackend, StorageOpenMode};
use storage_factory::open_storage_backend;

use super::{Failure, Request};
use crate::http::Response;
use crate::view;

/// The ActionTree path to the LLM call of an operation's boundary, shaped like the LLM
/// navigation.
pub(super) fn render(request: &Request<'_>) -> Result<Response, Failure> {
    let reader = request.reader()?;
    let operation = request.operation(&reader)?;
    let Some(boundary) = operation.boundary.as_ref() else {
        return Ok(not_found());
    };
    let storage = storage(request).map_err(Failure::Internal)?;
    match view::llm_call_of_request(storage.as_ref(), request.trace_id, &boundary.request_action_id)
        .map_err(Failure::Internal)?
    {
        Some(call) => view::action_tree_path_json(storage.as_ref(), request.trace_id, &call)
            .map(Response::json)
            .map_err(Failure::Internal),
        None => Ok(not_found()),
    }
}

/// The LLM call of each operation's boundary. The observation data may already be gone while the
/// checkpoint history stays, which leaves nothing to link.
pub(super) fn llm_calls(
    request: &Request<'_>,
    operations: &[OperationRecord],
) -> BTreeMap<OperationId, String> {
    if operations
        .iter()
        .all(|operation| operation.boundary.is_none())
    {
        return BTreeMap::new();
    }
    storage(request)
        .and_then(|storage| calls(storage.as_ref(), request.trace_id, operations))
        .unwrap_or_default()
}

fn calls(
    storage: &dyn StorageBackend,
    trace_id: TraceId,
    operations: &[OperationRecord],
) -> Result<BTreeMap<OperationId, String>, String> {
    let mut calls = BTreeMap::new();
    for operation in operations {
        let Some(boundary) = operation.boundary.as_ref() else {
            continue;
        };
        if let Some(call) = view::llm_call_of_request(storage, trace_id, &boundary.request_action_id)?
        {
            calls.insert(operation.operation_id, call);
        }
    }
    Ok(calls)
}

fn storage(request: &Request<'_>) -> Result<Box<dyn StorageBackend>, String> {
    open_storage_backend(&request.context.storage, StorageOpenMode::ReadOnly).map_err(|error| {
        format!(
            "open storage read-only failed: {}: {}",
            error.stage, error.message
        )
    })
}

fn not_found() -> Response {
    Response::json(r#"{"found":false,"path":[]}"#.to_string())
}
