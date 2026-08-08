//! Legacy core-module dynamic file-policy hostcalls and binary codec.

use plugin_system::{
    FilePolicyApplyMode, FilePolicyApplyPrecondition, FilePolicyApplyRequest, FilePolicyDecision,
    FilePolicyListFilter, FilePolicyMatchDryRunRequest, FilePolicyOperation, FilePolicyPatchItem,
    FilePolicyPatchOp, FilePolicyRuleDraft,
};
use wasmtime::{Caller, Memory};

use crate::engine::WasmStoreState;

use super::{exported_memory, guest_range, read_guest_bytes};

const FILE_POLICY_RULES_DENIED: i64 = -1;
const FILE_POLICY_RULES_NOT_FOUND: i64 = -2;
const FILE_POLICY_RULES_INVALID: i64 = -3;
const FILE_POLICY_RULES_TOO_LARGE: i64 = -4;
const FILE_POLICY_RULES_REJECTED: i64 = -5;
const FILE_POLICY_RULES_BINARY_VERSION: u8 = 1;

pub(super) fn file_policy_rules_version_get(caller: Caller<'_, WasmStoreState>) -> i64 {
    if !can_access_file_policy_rules(caller.data()) {
        return FILE_POLICY_RULES_DENIED;
    }
    let Some(host) = caller.data().file_policy_host().cloned() else {
        return FILE_POLICY_RULES_NOT_FOUND;
    };
    match host.rules_version_get() {
        Ok(revision) => i64::try_from(revision).unwrap_or(FILE_POLICY_RULES_TOO_LARGE),
        Err(_) => FILE_POLICY_RULES_INVALID,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn file_policy_rules_list(
    caller: &mut Caller<'_, WasmStoreState>,
    filter_ptr: i32,
    filter_len: i32,
    cursor_ptr: i32,
    cursor_len: i32,
    limit: i32,
    out_ptr: i32,
    max_len: i32,
) -> i64 {
    if !caller.data().host_grants().can_read_file_policy_rules() {
        return FILE_POLICY_RULES_DENIED;
    }
    let Some(host) = caller.data().file_policy_host().cloned() else {
        return FILE_POLICY_RULES_NOT_FOUND;
    };
    let Ok(memory) = exported_memory(caller) else {
        return FILE_POLICY_RULES_INVALID;
    };
    let Ok(filter) = read_guest_bytes(caller, &memory, filter_ptr, filter_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if filter.len() > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    let Ok(cursor_bytes) = read_guest_bytes(caller, &memory, cursor_ptr, cursor_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if cursor_bytes.len() > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    let filter = match parse_file_policy_list_filter(&filter) {
        Ok(filter) => filter,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    let cursor = if cursor_bytes.is_empty() {
        None
    } else {
        match String::from_utf8(cursor_bytes) {
            Ok(cursor) => Some(cursor),
            Err(_) => return FILE_POLICY_RULES_INVALID,
        }
    };
    let limit = match u32::try_from(limit) {
        Ok(limit) => limit,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    let result = match host.rules_list(filter, cursor, limit) {
        Ok(result) => result,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    write_guest_response(
        caller,
        &memory,
        out_ptr,
        max_len,
        &encode_file_policy_list_result(&result),
    )
}

pub(super) fn file_policy_rules_match_dry_run(
    caller: &mut Caller<'_, WasmStoreState>,
    request_ptr: i32,
    request_len: i32,
    out_ptr: i32,
    max_len: i32,
) -> i64 {
    if !caller
        .data()
        .host_grants()
        .can_match_dry_run_file_policy_rules()
    {
        return FILE_POLICY_RULES_DENIED;
    }
    let Some(host) = caller.data().file_policy_host().cloned() else {
        return FILE_POLICY_RULES_NOT_FOUND;
    };
    let Ok(memory) = exported_memory(caller) else {
        return FILE_POLICY_RULES_INVALID;
    };
    let Ok(request) = read_guest_bytes(caller, &memory, request_ptr, request_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if request.len() > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    let request = match parse_file_policy_match_dry_run_request(&request) {
        Ok(request) => request,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    let result = match host.rules_match_dry_run(request) {
        Ok(result) => result,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    write_guest_response(
        caller,
        &memory,
        out_ptr,
        max_len,
        &encode_file_policy_match_dry_run_result(&result),
    )
}

pub(super) fn file_policy_rules_apply_or_validate(
    caller: &mut Caller<'_, WasmStoreState>,
    patch_ptr: i32,
    patch_len: i32,
    out_ptr: i32,
    max_len: i32,
    apply: bool,
) -> i64 {
    if apply && !caller.data().host_grants().can_apply_file_policy_rules() {
        return FILE_POLICY_RULES_DENIED;
    }
    if !apply && !caller.data().host_grants().can_validate_file_policy_rules() {
        return FILE_POLICY_RULES_DENIED;
    }
    let Some(host) = caller.data().file_policy_host().cloned() else {
        return FILE_POLICY_RULES_NOT_FOUND;
    };
    let Some(owner) = caller
        .data()
        .file_policy_owner_instance_id()
        .map(str::to_string)
    else {
        return FILE_POLICY_RULES_NOT_FOUND;
    };
    let grants = caller
        .data()
        .host_grants()
        .file_policy_rules_apply_grants()
        .to_vec();
    let Ok(memory) = exported_memory(caller) else {
        return FILE_POLICY_RULES_INVALID;
    };
    let Ok((patch_offset, patch_len)) = guest_range(patch_ptr, patch_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if patch_len > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    let Ok((out_offset, max_len)) = guest_range(out_ptr, max_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if max_len > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    let mut patch = vec![0_u8; patch_len];
    if memory.read(&mut *caller, patch_offset, &mut patch).is_err() {
        return FILE_POLICY_RULES_INVALID;
    }
    let request = match parse_file_policy_apply_request(&patch) {
        Ok(request) => request,
        Err(_) => return FILE_POLICY_RULES_INVALID,
    };
    let result = if apply {
        host.rules_apply(&owner, &grants, request)
    } else {
        host.rules_validate(&owner, &grants, &request)
    };
    let result = match result {
        Ok(result) => result,
        Err(_) => return FILE_POLICY_RULES_REJECTED,
    };
    let response = encode_file_policy_apply_result(&result);
    if response.len() > max_len {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    if memory.write(&mut *caller, out_offset, &response).is_err() {
        return FILE_POLICY_RULES_INVALID;
    }
    i64::try_from(response.len()).unwrap_or(FILE_POLICY_RULES_TOO_LARGE)
}

fn can_access_file_policy_rules(state: &WasmStoreState) -> bool {
    state.host_grants().can_read_file_policy_rules()
        || state.host_grants().can_match_dry_run_file_policy_rules()
        || state.host_grants().can_validate_file_policy_rules()
        || state.host_grants().can_apply_file_policy_rules()
}

fn parse_file_policy_list_filter(bytes: &[u8]) -> Result<FilePolicyListFilter, String> {
    let mut cursor = BinaryCursor::new(bytes);
    let version = cursor.read_u8()?;
    if version != FILE_POLICY_RULES_BINARY_VERSION {
        return Err(format!("unsupported file policy binary version {version}"));
    }
    let decision = match cursor.read_u8()? {
        0 => None,
        _ => Some(FilePolicyDecision::from_code(cursor.read_u8()?)?),
    };
    let operation = match cursor.read_u8()? {
        0 => None,
        _ => Some(FilePolicyOperation::from_code(cursor.read_u8()?)?),
    };
    let path_prefix = cursor.read_string_u16()?;
    if !cursor.is_empty() {
        return Err("file policy list filter has trailing bytes".to_string());
    }
    Ok(FilePolicyListFilter {
        decision,
        path_prefix,
        operation,
    })
}

fn parse_file_policy_match_dry_run_request(
    bytes: &[u8],
) -> Result<FilePolicyMatchDryRunRequest, String> {
    let mut cursor = BinaryCursor::new(bytes);
    let version = cursor.read_u8()?;
    if version != FILE_POLICY_RULES_BINARY_VERSION {
        return Err(format!("unsupported file policy binary version {version}"));
    }
    let operation = FilePolicyOperation::from_code(cursor.read_u8()?)?;
    let path = cursor
        .read_string_u16()?
        .ok_or_else(|| "file policy dry-run path is required".to_string())?;
    if !cursor.is_empty() {
        return Err("file policy dry-run request has trailing bytes".to_string());
    }
    Ok(FilePolicyMatchDryRunRequest { path, operation })
}

fn parse_file_policy_apply_request(bytes: &[u8]) -> Result<FilePolicyApplyRequest, String> {
    let mut cursor = BinaryCursor::new(bytes);
    let version = cursor.read_u8()?;
    if version != FILE_POLICY_RULES_BINARY_VERSION {
        return Err(format!("unsupported file policy binary version {version}"));
    }
    let apply_mode = FilePolicyApplyMode::from_code(cursor.read_u8()?)?;
    let base_revision = cursor.read_u64()?;
    let item_count = cursor.read_u32()?;
    let item_count = usize::try_from(item_count)
        .map_err(|error| format!("file policy item count overflow: {error}"))?;
    let mut items = Vec::with_capacity(item_count);
    for _ in 0..item_count {
        items.push(parse_file_policy_patch_item(&mut cursor)?);
    }
    if !cursor.is_empty() {
        return Err("file policy patch has trailing bytes".to_string());
    }
    Ok(FilePolicyApplyRequest {
        items,
        precondition: FilePolicyApplyPrecondition {
            base_revision,
            mutation_id: String::new(),
            reason: None,
            correlation_id: None,
            apply_mode,
        },
    })
}

fn parse_file_policy_patch_item(
    cursor: &mut BinaryCursor<'_>,
) -> Result<FilePolicyPatchItem, String> {
    let op = FilePolicyPatchOp::from_code(cursor.read_u8()?)?;
    let decision = FilePolicyDecision::from_code(cursor.read_u8()?)?;
    let operation = FilePolicyOperation::from_code(cursor.read_u8()?)?;
    let priority = cursor.read_i32()?;
    let gray_target = match cursor.read_u64()? {
        0 => None,
        value => Some(value),
    };
    let rule_id = cursor.read_string_u16()?;
    let path = cursor.read_string_u16()?;
    let rule = matches!(op, FilePolicyPatchOp::Upsert).then(|| FilePolicyRuleDraft {
        rule_id: rule_id.clone(),
        decision,
        operation,
        path: path.unwrap_or_default(),
        gray_target,
        priority,
    });
    Ok(FilePolicyPatchItem { op, rule_id, rule })
}

fn encode_file_policy_apply_result(result: &plugin_system::FilePolicyApplyResult) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(result.status.code());
    bytes.extend_from_slice(&result.new_revision.to_le_bytes());
    bytes.extend_from_slice(&result.applied_count.to_le_bytes());
    bytes.extend_from_slice(&result.rejected_count.to_le_bytes());
    bytes.extend_from_slice(&(result.errors.len() as u32).to_le_bytes());
    for error in &result.errors {
        bytes.extend_from_slice(&error.item_index.to_le_bytes());
        push_u16_bytes(&mut bytes, error.code.as_bytes());
        push_u16_bytes(&mut bytes, error.message.as_bytes());
    }
    bytes
}

fn encode_file_policy_list_result(result: &plugin_system::FilePolicyListResult) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&result.source_revision.to_le_bytes());
    push_u16_bytes(
        &mut bytes,
        result.next_cursor.as_deref().unwrap_or_default().as_bytes(),
    );
    bytes.extend_from_slice(&(result.rules.len() as u32).to_le_bytes());
    for rule in &result.rules {
        bytes.push(rule.decision.code());
        bytes.push(rule.operation.code());
        bytes.extend_from_slice(&rule.gray_target.unwrap_or_default().to_le_bytes());
        bytes.extend_from_slice(&rule.priority.to_le_bytes());
        bytes.push(u8::from(rule.enabled));
        bytes.extend_from_slice(&rule.updated_sequence.to_le_bytes());
        push_u16_bytes(&mut bytes, rule.rule_id.as_bytes());
        push_u16_bytes(&mut bytes, rule.owner_instance_id.as_bytes());
        push_u16_bytes(&mut bytes, rule.path.as_bytes());
    }
    bytes
}

fn encode_file_policy_match_dry_run_result(
    result: &plugin_system::FilePolicyMatchDryRunResult,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(u8::from(result.matched));
    bytes.push(result.decision.code());
    bytes.push(result.operation.code());
    bytes.extend_from_slice(&result.source_revision.to_le_bytes());
    push_u16_bytes(
        &mut bytes,
        result.rule_id.as_deref().unwrap_or_default().as_bytes(),
    );
    push_u16_bytes(&mut bytes, result.canonical_path.as_bytes());
    bytes
}

fn push_u16_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&bytes[..usize::from(len)]);
}

struct BinaryCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> BinaryCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn read_u8(&mut self) -> Result<u8, String> {
        let bytes = self.read_exact(1)?;
        Ok(bytes[0])
    }

    fn read_u16(&mut self) -> Result<u16, String> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        let bytes = self.read_exact(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u64(&mut self) -> Result<u64, String> {
        let bytes = self.read_exact(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_i32(&mut self) -> Result<i32, String> {
        let bytes = self.read_exact(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_string_u16(&mut self) -> Result<Option<String>, String> {
        let len = usize::from(self.read_u16()?);
        if len == 0 {
            return Ok(None);
        }
        let bytes = self.read_exact(len)?;
        let value = std::str::from_utf8(bytes)
            .map_err(|error| format!("file policy string is not utf-8: {error}"))?;
        Ok(Some(value.to_string()))
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| "file policy binary offset overflow".to_string())?;
        if end > self.bytes.len() {
            return Err("file policy binary payload is truncated".to_string());
        }
        let bytes = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
}

fn write_guest_response(
    caller: &mut Caller<'_, WasmStoreState>,
    memory: &Memory,
    out_ptr: i32,
    max_len: i32,
    response: &[u8],
) -> i64 {
    let Ok((out_offset, max_len)) = guest_range(out_ptr, max_len) else {
        return FILE_POLICY_RULES_INVALID;
    };
    if max_len > caller.data().host_limits().file_policy_io_max_bytes {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    if response.len() > max_len {
        return FILE_POLICY_RULES_TOO_LARGE;
    }
    if memory.write(caller, out_offset, response).is_err() {
        return FILE_POLICY_RULES_INVALID;
    }
    i64::try_from(response.len()).unwrap_or(FILE_POLICY_RULES_TOO_LARGE)
}
