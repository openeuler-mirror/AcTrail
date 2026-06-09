//! Plan lookup request/response codec for the preloaded runtime.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use crate::plan::{
    RuntimePlanDescriptor, decode_hex_string, decode_runtime_plan, encode_hex, encode_runtime_plan,
};
use crate::{SyncError, SyncResult};

const LOOKUP_VERSION: &str = "v1";
const FIELD_SEPARATOR: char = '\t';
const REQUEST_OPCODE: &str = "plan.lookup";
const FOUND_OPCODE: &str = "plan.found";
const UNSUPPORTED_OPCODE: &str = "plan.unsupported";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanLookupRequest {
    pub binary: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanLookupResponse {
    Found(RuntimePlanDescriptor),
    Unsupported { reason: String },
}

pub fn lookup_runtime_plan(socket_path: &Path, binary: &Path) -> SyncResult<PlanLookupResponse> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.write_all(&encode_plan_lookup_request(&PlanLookupRequest {
        binary: binary.to_path_buf(),
    }))?;
    let mut line = Vec::new();
    let mut reader = BufReader::new(stream);
    reader.read_until(b'\n', &mut line)?;
    if line.is_empty() {
        return Err(SyncError::new("empty plan lookup response"));
    }
    decode_plan_lookup_response(&line)
}

pub fn encode_plan_lookup_request(request: &PlanLookupRequest) -> Vec<u8> {
    let mut line = [
        LOOKUP_VERSION.to_string(),
        REQUEST_OPCODE.to_string(),
        encode_hex(request.binary.display().to_string().as_bytes()),
    ]
    .join(&FIELD_SEPARATOR.to_string())
    .into_bytes();
    line.push(b'\n');
    line
}

pub fn decode_plan_lookup_request(line: &[u8]) -> SyncResult<PlanLookupRequest> {
    let fields = line_fields(line)?;
    if fields.first().copied() != Some(LOOKUP_VERSION)
        || fields.get(1).copied() != Some(REQUEST_OPCODE)
    {
        return Err(SyncError::new("not a plan lookup request"));
    }
    require_len(&fields, 3, REQUEST_OPCODE)?;
    Ok(PlanLookupRequest {
        binary: PathBuf::from(decode_hex_string(fields[2])?),
    })
}

pub fn encode_plan_lookup_response(response: &PlanLookupResponse) -> Vec<u8> {
    let fields = match response {
        PlanLookupResponse::Found(plan) => vec![
            LOOKUP_VERSION.to_string(),
            FOUND_OPCODE.to_string(),
            encode_runtime_plan(plan),
        ],
        PlanLookupResponse::Unsupported { reason } => vec![
            LOOKUP_VERSION.to_string(),
            UNSUPPORTED_OPCODE.to_string(),
            encode_hex(reason.as_bytes()),
        ],
    };
    let mut line = fields.join(&FIELD_SEPARATOR.to_string()).into_bytes();
    line.push(b'\n');
    line
}

pub fn decode_plan_lookup_response(line: &[u8]) -> SyncResult<PlanLookupResponse> {
    let fields = line_fields(line)?;
    if fields.first().copied() != Some(LOOKUP_VERSION) {
        return Err(SyncError::new("unsupported plan lookup response version"));
    }
    match fields.get(1).copied() {
        Some(FOUND_OPCODE) => {
            require_len(&fields, 3, FOUND_OPCODE)?;
            Ok(PlanLookupResponse::Found(decode_runtime_plan(fields[2])?))
        }
        Some(UNSUPPORTED_OPCODE) => {
            require_len(&fields, 3, UNSUPPORTED_OPCODE)?;
            Ok(PlanLookupResponse::Unsupported {
                reason: decode_hex_string(fields[2])?,
            })
        }
        _ => Err(SyncError::new("unknown plan lookup response opcode")),
    }
}

fn line_fields(line: &[u8]) -> SyncResult<Vec<&str>> {
    let line = std::str::from_utf8(line)
        .map_err(|error| SyncError::new(format!("plan lookup utf8: {error}")))?
        .trim_end_matches('\n');
    Ok(line.split(FIELD_SEPARATOR).collect())
}

fn require_len(fields: &[&str], expected: usize, opcode: &str) -> SyncResult<()> {
    if fields.len() == expected {
        Ok(())
    } else {
        Err(SyncError::new(format!(
            "invalid {opcode} field count {}",
            fields.len()
        )))
    }
}
