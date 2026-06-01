//! Policy-record encoding used by the SQLite storage adapter.

use model_core::policy::{PolicyRecord, RedactionRecord, TruncationRecord};
use rusqlite::Error as SqlError;

use crate::records::enums::{
    decode_policy_verdict, decode_truncation_reason, encode_truncation_reason,
};
use crate::records::helpers::{escape, unescape};

pub fn encode_policy_record(policy: &PolicyRecord) -> (String, String) {
    let redactions = policy
        .redactions
        .iter()
        .map(|record| format!("{}={}", escape(&record.field), escape(&record.reason)))
        .collect::<Vec<_>>()
        .join("\n");
    let truncations = policy
        .truncations
        .iter()
        .map(|record| {
            format!(
                "{}={}|{}|{}",
                escape(&record.field),
                record.original_size,
                record.retained_size,
                encode_truncation_reason(record.reason),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    (redactions, truncations)
}

pub fn decode_policy_record(
    verdict: &str,
    note: Option<String>,
    redactions: &str,
    truncations: &str,
) -> Result<PolicyRecord, SqlError> {
    let redactions = redactions
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(field, reason)| RedactionRecord::new(unescape(field), unescape(reason)))
        .collect();
    let truncations = truncations
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter_map(|(field, rest)| {
            let mut parts = rest.split('|');
            let original_size = parts.next()?.parse::<usize>().ok()?;
            let retained_size = parts.next()?.parse::<usize>().ok()?;
            let reason = decode_truncation_reason(parts.next()?).ok()?;
            Some(TruncationRecord::new(
                unescape(field),
                original_size,
                retained_size,
                reason,
            ))
        })
        .collect();
    Ok(PolicyRecord {
        verdict: decode_policy_verdict(verdict)?,
        redactions,
        truncations,
        note,
    })
}
