use storage_core::SemanticActionChildPageQuery;

use crate::view;

pub(super) fn parse_action_tree_page(query: &str) -> Result<SemanticActionChildPageQuery, String> {
    let offset = required_query_usize(query, "offset")?;
    let limit = required_query_usize(query, "limit")?;
    if limit == usize::default() {
        return Err("invalid query parameter limit: value must be positive".to_string());
    }
    Ok(SemanticActionChildPageQuery { offset, limit })
}

pub(super) fn parse_llm_request_content_query(query: &str) -> Result<usize, String> {
    let max_bytes = required_query_usize(query, "max_bytes")?;
    if max_bytes == usize::default() {
        return Err("invalid query parameter max_bytes: value must be positive".to_string());
    }
    Ok(max_bytes)
}

pub(super) fn parse_token_usage_stats_query(
    query: &str,
) -> Result<view::TokenUsageStatsQuery, String> {
    let from_ms = required_query_u64(query, "from_ms")?;
    let to_ms = required_query_u64(query, "to_ms")?;
    if from_ms >= to_ms {
        return Err("invalid token stats range: from_ms must be less than to_ms".to_string());
    }
    Ok(view::TokenUsageStatsQuery { from_ms, to_ms })
}

pub(super) fn parse_llm_activity_query(query: &str) -> Result<view::LlmActivityQuery, String> {
    let from_ms = required_query_u64(query, "from_ms")?;
    let to_ms = required_query_u64(query, "to_ms")?;
    if from_ms >= to_ms {
        return Err("invalid llm activity range: from_ms must be less than to_ms".to_string());
    }
    let rollup = optional_query_param(query, "rollup")?
        .map(|raw| view::Rollup::parse(&raw))
        .transpose()?;
    Ok(view::LlmActivityQuery {
        from_ms,
        to_ms,
        rollup,
    })
}

pub(super) fn parse_llm_rows_query(query: &str) -> Result<view::LlmRowsQuery, String> {
    let from_ms = required_query_u64(query, "from_ms")?;
    let to_ms = required_query_u64(query, "to_ms")?;
    if from_ms >= to_ms {
        return Err("invalid llm rows range: from_ms must be less than to_ms".to_string());
    }
    let offset = required_query_usize(query, "offset")?;
    let limit = required_query_usize(query, "limit")?;
    if limit == usize::default() {
        return Err("invalid query parameter limit: value must be positive".to_string());
    }
    Ok(view::LlmRowsQuery {
        from_ms,
        to_ms,
        offset,
        limit,
    })
}

pub(super) fn parse_llm_export_query(query: &str) -> Result<view::LlmExportQuery, String> {
    let from_ms = required_query_u64(query, "from_ms")?;
    let to_ms = required_query_u64(query, "to_ms")?;
    if from_ms >= to_ms {
        return Err("invalid llm export range: from_ms must be less than to_ms".to_string());
    }
    let view = required_query_param(query, "view")?;
    Ok(view::LlmExportQuery {
        from_ms,
        to_ms,
        view: view::ExportView::parse(&view)?,
    })
}

pub(super) fn required_query_param(query: &str, key: &'static str) -> Result<String, String> {
    for part in query.split('&').filter(|part| !part.is_empty()) {
        let Some((candidate, value)) = part.split_once('=') else {
            continue;
        };
        if candidate == key {
            return percent_decode(value)
                .map_err(|error| format!("invalid query parameter {key}: {error}"));
        }
    }
    Err(format!("missing query parameter {key}"))
}

pub(super) fn optional_query_param(
    query: &str,
    key: &'static str,
) -> Result<Option<String>, String> {
    for part in query.split('&').filter(|part| !part.is_empty()) {
        let Some((candidate, value)) = part.split_once('=') else {
            continue;
        };
        if candidate == key {
            return percent_decode(value)
                .map(Some)
                .map_err(|error| format!("invalid query parameter {key}: {error}"));
        }
    }
    Ok(None)
}

pub(super) fn parse_u64(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|error| format!("invalid numeric path segment {value}: {error}"))
}

pub(super) fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            output.push(bytes[index]);
            index += 1;
            continue;
        }
        let Some(hex) = bytes.get(index + 1..index + 3) else {
            return Err(format!("invalid percent escape in {value}"));
        };
        let text = std::str::from_utf8(hex)
            .map_err(|error| format!("invalid percent escape in {value}: {error}"))?;
        let decoded = u8::from_str_radix(text, 16)
            .map_err(|error| format!("invalid percent escape %{text}: {error}"))?;
        output.push(decoded);
        index += 3;
    }
    String::from_utf8(output).map_err(|error| format!("invalid path utf-8: {error}"))
}

fn required_query_usize(query: &str, key: &'static str) -> Result<usize, String> {
    let raw = required_query_param(query, key)?;
    raw.parse::<usize>()
        .map_err(|error| format!("invalid query parameter {key}: {error}"))
}

fn required_query_u64(query: &str, key: &'static str) -> Result<u64, String> {
    let raw = required_query_param(query, key)?;
    raw.parse::<u64>()
        .map_err(|error| format!("invalid query parameter {key}: {error}"))
}
