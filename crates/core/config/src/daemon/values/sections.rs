//! Section parsing for the flat operator config parser.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FlatParserSection {
    Root,
    Export,
    ExportRoute,
    ExportOtelJsonlRoute,
    SemanticRetention,
    SemanticRetentionL0LlmCall,
    SemanticRetentionL1Sse,
    SemanticRetentionL2Http,
    SemanticRetentionL3Http2Frame,
    SemanticRetentionL4Payload,
    FileObservation,
    FileObservationTty,
    FileObservationBulkRead,
    FileObservationEnumerate,
}

pub(super) fn parse_section_header(
    line: &str,
    line_number: usize,
) -> Result<Option<FlatParserSection>, String> {
    if line.starts_with("[[") {
        if !line.ends_with("]]") {
            return Err(format!("invalid config section line {line_number}"));
        }
        if line == "[[export.routes]]" {
            return Ok(Some(FlatParserSection::ExportRoute));
        }
        return Err(format!("unsupported config section line {line_number}"));
    }
    if line.ends_with("]]") {
        return Err(format!("invalid config section line {line_number}"));
    }
    if !(line.starts_with('[') || line.ends_with(']')) {
        return Ok(None);
    }
    if !(line.starts_with('[') && line.ends_with(']')) {
        return Err(format!("invalid config section line {line_number}"));
    }
    match &line[1..line.len() - 1] {
        "export" => Ok(Some(FlatParserSection::Export)),
        section if section.starts_with("export.routes.otel-jsonl.") => {
            Ok(Some(FlatParserSection::ExportOtelJsonlRoute))
        }
        "semantic_retention" => Ok(Some(FlatParserSection::SemanticRetention)),
        "semantic_retention.L0_llm_call" => Ok(Some(FlatParserSection::SemanticRetentionL0LlmCall)),
        "semantic_retention.L1_sse" => Ok(Some(FlatParserSection::SemanticRetentionL1Sse)),
        "semantic_retention.L2_http" => Ok(Some(FlatParserSection::SemanticRetentionL2Http)),
        "semantic_retention.L3_http2_frame" => {
            Ok(Some(FlatParserSection::SemanticRetentionL3Http2Frame))
        }
        "semantic_retention.L4_payload" => Ok(Some(FlatParserSection::SemanticRetentionL4Payload)),
        "file_observation" => Ok(Some(FlatParserSection::FileObservation)),
        "file_observation.tty" => Ok(Some(FlatParserSection::FileObservationTty)),
        "file_observation.bulk_read" => Ok(Some(FlatParserSection::FileObservationBulkRead)),
        "file_observation.enumerate" => Ok(Some(FlatParserSection::FileObservationEnumerate)),
        _ => Err(format!("unsupported config section line {line_number}")),
    }
}

pub(super) fn section_storage_key(
    section: &FlatParserSection,
    key: &str,
) -> Result<Option<String>, String> {
    if !section_key_allowed(section, key) {
        return Err(format!(
            "unexpected config key {key} inside section-based config"
        ));
    }
    match section {
        FlatParserSection::Root => Ok(Some(key.to_string())),
        FlatParserSection::Export
        | FlatParserSection::ExportRoute
        | FlatParserSection::ExportOtelJsonlRoute => Ok(None),
        FlatParserSection::SemanticRetention => Ok(Some(format!("semantic_retention_{key}"))),
        FlatParserSection::SemanticRetentionL0LlmCall => {
            Ok(Some(format!("semantic_retention_L0_llm_call_{key}")))
        }
        FlatParserSection::SemanticRetentionL1Sse => {
            Ok(Some(format!("semantic_retention_L1_sse_{key}")))
        }
        FlatParserSection::SemanticRetentionL2Http => {
            Ok(Some(format!("semantic_retention_L2_http_{key}")))
        }
        FlatParserSection::SemanticRetentionL3Http2Frame => {
            Ok(Some(format!("semantic_retention_L3_http2_frame_{key}")))
        }
        FlatParserSection::SemanticRetentionL4Payload => {
            Ok(Some(format!("semantic_retention_L4_payload_{key}")))
        }
        FlatParserSection::FileObservation => Ok(Some(format!("file_observation_{key}"))),
        FlatParserSection::FileObservationTty => Ok(Some(format!("file_observation_tty_{key}"))),
        FlatParserSection::FileObservationBulkRead => {
            Ok(Some(format!("file_observation_bulk_read_{key}")))
        }
        FlatParserSection::FileObservationEnumerate => {
            Ok(Some(format!("file_observation_enumerate_{key}")))
        }
    }
}

fn section_key_allowed(section: &FlatParserSection, key: &str) -> bool {
    match section {
        FlatParserSection::Root => true,
        FlatParserSection::Export => key == "enabled",
        FlatParserSection::ExportRoute => matches!(key, "name" | "kind" | "delivery" | "enabled"),
        FlatParserSection::ExportOtelJsonlRoute => matches!(
            key,
            "path" | "overwrite_enabled" | "queue_capacity" | "flush_every_spans"
        ),
        FlatParserSection::SemanticRetention => key == "content_owner",
        FlatParserSection::SemanticRetentionL0LlmCall => {
            matches!(
                key,
                "enabled" | "request_content" | "response_content" | "tool_calls" | "usage"
            )
        }
        FlatParserSection::SemanticRetentionL1Sse => {
            matches!(key, "enabled" | "stream_summary" | "event_content")
        }
        FlatParserSection::SemanticRetentionL2Http => {
            matches!(
                key,
                "enabled" | "message_summary" | "headers" | "body_content"
            )
        }
        FlatParserSection::SemanticRetentionL3Http2Frame => {
            matches!(key, "enabled" | "frame_summary" | "data_content")
        }
        FlatParserSection::SemanticRetentionL4Payload => {
            matches!(key, "enabled" | "stats" | "body_content")
        }
        FlatParserSection::FileObservation => matches!(key, "enabled" | "metadata_retention"),
        FlatParserSection::FileObservationTty => {
            matches!(
                key,
                "enabled" | "path" | "operation" | "raw_event_retention"
            )
        }
        FlatParserSection::FileObservationBulkRead => matches!(
            key,
            "enabled"
                | "mode"
                | "raw_event_retention"
                | "min_unique_paths"
                | "max_paths_per_set"
                | "path_set_chunk_max_paths"
        ),
        FlatParserSection::FileObservationEnumerate => matches!(
            key,
            "enabled"
                | "raw_event_retention"
                | "min_unique_paths"
                | "max_paths_per_set"
                | "path_set_chunk_max_paths"
        ),
    }
}
